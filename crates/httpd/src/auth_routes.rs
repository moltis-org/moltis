use std::{net::SocketAddr, sync::Arc};

use secrecy::ExposeSecret;

use {
    axum::{
        Json,
        extract::{ConnectInfo, State},
        http::StatusCode,
        response::IntoResponse,
        routing::{delete, get, post},
    },
    axum_extra::extract::Host,
};

use {
    moltis_auth::locality::is_local_connection,
    moltis_gateway::{
        auth::CredentialStore, auth_webauthn::SharedWebAuthnRegistry, state::GatewayState,
    },
};

use crate::{
    auth_middleware::{AuthResult, AuthSession, SESSION_COOKIE, check_auth},
    login_guard::LoginGuard,
};

#[cfg(feature = "vault")]
use crate::auth_routes::vault::{run_vault_env_migration, start_stored_channels_on_vault_unseal};

/// Auth-related application state.
#[derive(Clone)]
pub struct AuthState {
    pub credential_store: Arc<CredentialStore>,
    pub webauthn_registry: Option<SharedWebAuthnRegistry>,
    pub gateway_state: Arc<GatewayState>,
    pub login_guard: LoginGuard,
}

impl axum::extract::FromRef<AuthState> for Arc<CredentialStore> {
    fn from_ref(state: &AuthState) -> Self {
        Arc::clone(&state.credential_store)
    }
}

impl axum::extract::FromRef<AuthState> for Arc<GatewayState> {
    fn from_ref(state: &AuthState) -> Self {
        Arc::clone(&state.gateway_state)
    }
}

/// Build the auth router with all `/api/auth/*` routes.
pub fn auth_router() -> axum::Router<AuthState> {
    axum::Router::new()
        .route("/status", get(status_handler))
        .route("/setup", post(setup_handler))
        .route("/login", post(login_handler))
        .route("/logout", post(logout_handler))
        .route("/password/change", post(change_password_handler))
        .route(
            "/api-keys",
            get(list_api_keys_handler).post(create_api_key_handler),
        )
        .route("/api-keys/{id}", delete(revoke_api_key_handler))
        .route("/passkeys", get(list_passkeys_handler))
        .route(
            "/passkeys/{id}",
            delete(remove_passkey_handler).patch(rename_passkey_handler),
        )
        .route(
            "/passkey/register/begin",
            post(passkey_register_begin_handler),
        )
        .route(
            "/passkey/register/finish",
            post(passkey_register_finish_handler),
        )
        .route("/passkey/auth/begin", post(passkey_auth_begin_handler))
        .route("/passkey/auth/finish", post(passkey_auth_finish_handler))
        .route(
            "/setup/passkey/register/begin",
            post(setup_passkey_register_begin_handler),
        )
        .route(
            "/setup/passkey/register/finish",
            post(setup_passkey_register_finish_handler),
        )
        .route("/reset", post(reset_auth_handler))
        // Vault endpoints (encryption-at-rest).
        .merge(vault_routes())
}

/// Build vault-specific routes (no-op when vault feature is disabled).
#[cfg(feature = "vault")]
fn vault_routes() -> axum::Router<AuthState> {
    axum::Router::new()
        .route("/vault/status", get(vault::vault_status_handler))
        .route("/vault/unlock", post(vault::vault_unlock_handler))
        .route("/vault/recovery", post(vault::vault_recovery_handler))
}

#[cfg(not(feature = "vault"))]
fn vault_routes() -> axum::Router<AuthState> {
    axum::Router::new()
}

// ── Brute-force block response ────────────────────────────────────────────────

fn blocked_response(reason: crate::login_guard::BlockReason) -> axum::response::Response {
    use crate::login_guard::BlockReason;
    let (message, retry_after) = match reason {
        BlockReason::IpBanned { retry_after } => (
            "too many failed attempts from this address — try again later",
            retry_after,
        ),
        BlockReason::AccountLocked { retry_after } => (
            "account temporarily locked due to suspicious activity — try again later",
            retry_after,
        ),
    };
    let retry_secs = retry_after.as_secs().max(1);
    let mut resp = (
        StatusCode::TOO_MANY_REQUESTS,
        Json(serde_json::json!({
            "code": "LOGIN_BLOCKED",
            "error": message,
            "retry_after_seconds": retry_secs,
        })),
    )
        .into_response();
    if let Ok(val) = retry_secs.to_string().parse() {
        resp.headers_mut()
            .insert(axum::http::header::RETRY_AFTER, val);
    }
    resp
}

// ── Status ───────────────────────────────────────────────────────────────────

async fn status_handler(
    State(state): State<AuthState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let auth_disabled = state.credential_store.is_auth_disabled();
    let localhost_only = state.gateway_state.localhost_only;
    let has_password = state.credential_store.has_password().await.unwrap_or(false);
    let has_passkeys = state.credential_store.has_passkeys().await.unwrap_or(false);

    let is_local = is_local_connection(&headers, addr, state.gateway_state.behind_proxy);
    let auth_result = check_auth(&state.credential_store, &headers, is_local).await;
    let authenticated = matches!(auth_result, AuthResult::Allowed(_));
    let setup_required = matches!(auth_result, AuthResult::SetupRequired);

    let setup_code_required = state.gateway_state.inner.read().await.setup_code.is_some();

    let webauthn_available = state.webauthn_registry.is_some();

    let passkey_origins: Vec<String> = if let Some(registry) = state.webauthn_registry.as_ref() {
        registry.read().await.get_all_origins()
    } else {
        Vec::new()
    };

    if !has_passkeys {
        state
            .gateway_state
            .clear_all_passkey_host_update_pending()
            .await;
    }
    let passkey_host_update_hosts = if has_passkeys {
        state.gateway_state.passkey_host_update_pending().await
    } else {
        Vec::new()
    };
    let passkey_host_update_required = !passkey_host_update_hosts.is_empty();

    let setup_complete = state.credential_store.is_setup_complete();

    Json(serde_json::json!({
        "setup_required": setup_required,
        "setup_complete": setup_complete,
        "has_passkeys": has_passkeys,
        "authenticated": authenticated,
        "auth_disabled": auth_disabled,
        "setup_code_required": setup_code_required,
        "has_password": has_password,
        "localhost_only": localhost_only,
        "webauthn_available": webauthn_available,
        "passkey_origins": passkey_origins,
        "passkey_host_update_required": passkey_host_update_required,
        "passkey_host_update_hosts": passkey_host_update_hosts,
    }))
}

// ── Setup (first run) ────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct SetupRequest {
    password: Option<String>,
    setup_code: Option<String>,
}

async fn setup_handler(
    State(state): State<AuthState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
    Json(body): Json<SetupRequest>,
) -> impl IntoResponse {
    if state.credential_store.is_setup_complete() {
        return (StatusCode::FORBIDDEN, "setup already completed").into_response();
    }

    let client_ip = crate::request_throttle::resolve_client_ip(
        &headers,
        addr,
        state.gateway_state.behind_proxy,
    );
    const SETUP_ACCOUNT: &str = "__setup__";

    if let Some(block) = state.login_guard.check(client_ip, SETUP_ACCOUNT) {
        return blocked_response(block);
    }

    // Validate setup code if one was generated at startup.
    {
        let inner = state.gateway_state.inner.read().await;
        if let Some(ref expected) = inner.setup_code {
            // Expire setup code after 30 minutes.
            if let Some(created_at) = inner.setup_code_created_at
                && created_at.elapsed() > std::time::Duration::from_secs(30 * 60)
            {
                return (
                    StatusCode::GONE,
                    "setup code has expired — restart the server to generate a new one",
                )
                    .into_response();
            }
            if body.setup_code.as_deref() != Some(expected.expose_secret().as_str()) {
                state.login_guard.record_failure(client_ip, SETUP_ACCOUNT);
                return (StatusCode::FORBIDDEN, "invalid or missing setup code").into_response();
            }
        }
    }

    let password = body.password.unwrap_or_default();

    let is_local = is_local_connection(&headers, addr, state.gateway_state.behind_proxy);
    if password.is_empty() && is_local {
        // Local connection with no password: skip setup without setting one.
        if let Err(e) = state.credential_store.clear_auth_disabled().await {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to clear auth-disabled state: {e}"),
            )
                .into_response();
        }
    } else {
        if password.len() < 12 {
            return (
                StatusCode::BAD_REQUEST,
                "password must be at least 12 characters",
            )
                .into_response();
        }
        if let Err(e) = state.credential_store.set_initial_password(&password).await {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to set password: {e}"),
            )
                .into_response();
        }
    }

    // Initialize the vault when a password was set.
    #[cfg(feature = "vault")]
    let vault_recovery_key = if !password.is_empty() {
        if let Some(ref vault) = state.gateway_state.vault {
            match vault.initialize(&password).await {
                Ok(rk) => {
                    tracing::info!("vault initialized");
                    run_vault_env_migration(&state).await;
                    start_stored_channels_on_vault_unseal(&state).await;
                    Some(rk.phrase().to_owned())
                },
                Err(moltis_vault::VaultError::AlreadyInitialized) => {
                    tracing::debug!("vault already initialized, skipping");
                    None
                },
                Err(e) => {
                    tracing::warn!(error = %e, "vault initialization failed");
                    None
                },
            }
        } else {
            None
        }
    } else {
        None
    };

    // Disconnect pre-setup WebSocket clients and clear setup code.
    state
        .gateway_state
        .disconnect_all_clients("setup_complete")
        .await;
    state.gateway_state.inner.write().await.setup_code = None;
    match state.credential_store.create_session().await {
        Ok(token) => {
            let bp = state.gateway_state.behind_proxy;
            let secure = state.gateway_state.is_secure();
            #[cfg(feature = "vault")]
            if let Some(rk) = vault_recovery_key {
                let domain_attr = localhost_cookie_domain(&headers, bp);
                let secure_attr = if secure {
                    "; Secure"
                } else {
                    ""
                };
                let cookie = format!(
                    "{SESSION_COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age=2592000{domain_attr}{secure_attr}"
                );
                return (
                    StatusCode::OK,
                    [(axum::http::header::SET_COOKIE, cookie)],
                    Json(serde_json::json!({ "ok": true, "recovery_key": rk })),
                )
                    .into_response();
            }
            session_response(token, &headers, bp, secure)
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to create session: {e}"),
        )
            .into_response(),
    }
}

// ── Login ────────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct LoginRequest {
    password: String,
}

async fn login_handler(
    State(state): State<AuthState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
    Json(body): Json<LoginRequest>,
) -> impl IntoResponse {
    let client_ip = crate::request_throttle::resolve_client_ip(
        &headers,
        addr,
        state.gateway_state.behind_proxy,
    );
    const PASSWORD_ACCOUNT: &str = "__password__";

    if let Some(block) = state.login_guard.check(client_ip, PASSWORD_ACCOUNT) {
        return blocked_response(block);
    }

    let ip_str = client_ip.to_string();
    match state.credential_store.verify_password(&body.password).await {
        Ok(true) => {
            state.login_guard.record_success(client_ip);
            state
                .credential_store
                .audit_log("login_success", Some(&ip_str), None)
                .await;
            // Best-effort vault unseal on successful login.
            #[cfg(feature = "vault")]
            if let Some(ref vault) = state.gateway_state.vault {
                match vault.unseal(&body.password).await {
                    Ok(()) => {
                        tracing::info!("vault unsealed on login");
                        run_vault_env_migration(&state).await;
                        start_stored_channels_on_vault_unseal(&state).await;
                    },
                    Err(e) => {
                        tracing::debug!(error = %e, "vault unseal on login skipped");
                    },
                }
            }
            match state.credential_store.create_session().await {
                Ok(token) => {
                    let bp = state.gateway_state.behind_proxy;
                    session_response(token, &headers, bp, state.gateway_state.is_secure())
                },
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("session error: {e}"),
                )
                    .into_response(),
            }
        },
        Ok(false) => {
            state
                .login_guard
                .record_failure(client_ip, PASSWORD_ACCOUNT);
            state
                .credential_store
                .audit_log("login_failure", Some(&ip_str), None)
                .await;
            (StatusCode::UNAUTHORIZED, "invalid password").into_response()
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("auth error: {e}"),
        )
            .into_response(),
    }
}

// ── Logout ───────────────────────────────────────────────────────────────────

async fn logout_handler(
    State(state): State<AuthState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    if let Some(token) = extract_session_token(&headers) {
        let _ = state.credential_store.delete_session(token).await;
    }
    let bp = state.gateway_state.behind_proxy;
    clear_session_response(&headers, bp, state.gateway_state.is_secure())
}

// ── Reset all auth (requires session) ─────────────────────────────────────────

async fn reset_auth_handler(
    _session: AuthSession,
    State(state): State<AuthState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    match state.credential_store.reset_all().await {
        Ok(()) => {
            // Disconnect all clients before generating new setup code.
            state
                .gateway_state
                .disconnect_all_clients("auth_reset")
                .await;
            let code = moltis_gateway::auth::generate_setup_code();
            tracing::info!("setup code: {code} (enter this in the browser to set your password)");
            {
                let mut inner = state.gateway_state.inner.write().await;
                inner.setup_code = Some(secrecy::Secret::new(code));
                inner.setup_code_created_at = Some(std::time::Instant::now());
            }
            let bp = state.gateway_state.behind_proxy;
            clear_session_response(&headers, bp, state.gateway_state.tls_active || bp)
        },
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ── Password change (requires session) ───────────────────────────────────────

#[derive(serde::Deserialize)]
struct ChangePasswordRequest {
    current_password: Option<String>,
    new_password: String,
}

async fn change_password_handler(
    _session: AuthSession,
    State(state): State<AuthState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
    Json(body): Json<ChangePasswordRequest>,
) -> impl IntoResponse {
    if body.new_password.len() < 12 {
        return (
            StatusCode::BAD_REQUEST,
            "new password must be at least 12 characters",
        )
            .into_response();
    }

    let has_password = state.credential_store.has_password().await.unwrap_or(false);

    if !has_password {
        // No password set yet — add one (works even after passkey-only setup).
        return match state
            .credential_store
            .add_password(&body.new_password)
            .await
        {
            Ok(()) => {
                // Initialize the vault now that we have a password.
                #[cfg(feature = "vault")]
                let vault_recovery_key = if let Some(ref vault) = state.gateway_state.vault {
                    match vault.initialize(&body.new_password).await {
                        Ok(rk) => {
                            tracing::info!("vault initialized on first password set");
                            run_vault_env_migration(&state).await;
                            start_stored_channels_on_vault_unseal(&state).await;
                            Some(rk.phrase().to_owned())
                        },
                        Err(moltis_vault::VaultError::AlreadyInitialized) => {
                            tracing::debug!("vault already initialized, unsealing");
                            let _ = vault.unseal(&body.new_password).await;
                            None
                        },
                        Err(e) => {
                            tracing::warn!(error = %e, "vault initialization failed");
                            None
                        },
                    }
                } else {
                    None
                };
                state
                    .gateway_state
                    .disconnect_all_clients("password_changed")
                    .await;
                #[cfg(feature = "vault")]
                if let Some(rk) = vault_recovery_key {
                    return Json(serde_json::json!({ "ok": true, "recovery_key": rk }))
                        .into_response();
                }
                Json(serde_json::json!({ "ok": true })).into_response()
            },
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        };
    }

    let client_ip = crate::request_throttle::resolve_client_ip(
        &headers,
        addr,
        state.gateway_state.behind_proxy,
    );
    const PASSWORD_CHANGE_ACCOUNT: &str = "__password_change__";

    if let Some(block) = state.login_guard.check(client_ip, PASSWORD_CHANGE_ACCOUNT) {
        return blocked_response(block);
    }

    let current_password = body.current_password.unwrap_or_default();
    match state
        .credential_store
        .change_password(&current_password, &body.new_password)
        .await
    {
        Ok(()) => {
            state.login_guard.record_success(client_ip);
            // Best-effort vault password rotation.
            #[cfg(feature = "vault")]
            if let Some(ref vault) = state.gateway_state.vault {
                match vault
                    .change_password(&current_password, &body.new_password)
                    .await
                {
                    Ok(()) => tracing::info!("vault password rotated"),
                    Err(e) => tracing::warn!(error = %e, "vault password rotation failed"),
                }
            }
            state
                .gateway_state
                .disconnect_all_clients("password_changed")
                .await;
            Json(serde_json::json!({ "ok": true })).into_response()
        },
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("incorrect") {
                state
                    .login_guard
                    .record_failure(client_ip, PASSWORD_CHANGE_ACCOUNT);
                (StatusCode::FORBIDDEN, msg).into_response()
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        },
    }
}

// ── API Keys (require session) ───────────────────────────────────────────────

async fn list_api_keys_handler(
    _session: AuthSession,
    State(state): State<AuthState>,
) -> impl IntoResponse {
    match state.credential_store.list_api_keys().await {
        Ok(keys) => Json(serde_json::json!({ "api_keys": keys })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(serde::Deserialize)]
struct CreateApiKeyRequest {
    label: String,
    /// Optional scopes. If omitted or empty, the key has full access.
    scopes: Option<Vec<String>>,
}

async fn create_api_key_handler(
    _session: AuthSession,
    State(state): State<AuthState>,
    Json(body): Json<CreateApiKeyRequest>,
) -> impl IntoResponse {
    if body.label.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "label is required").into_response();
    }

    // Validate scopes if provided
    if let Some(ref scopes) = body.scopes {
        for scope in scopes {
            if !moltis_gateway::auth::VALID_SCOPES.contains(&scope.as_str()) {
                return (StatusCode::BAD_REQUEST, format!("invalid scope: {scope}"))
                    .into_response();
            }
        }
    }

    match state
        .credential_store
        .create_api_key(body.label.trim(), body.scopes.as_deref())
        .await
    {
        Ok((id, key)) => {
            state
                .credential_store
                .audit_log(
                    "key_created",
                    None,
                    Some(&format!("id={id}, label={}", body.label.trim())),
                )
                .await;
            Json(serde_json::json!({ "id": id, "key": key })).into_response()
        },
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn revoke_api_key_handler(
    _session: AuthSession,
    State(state): State<AuthState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> impl IntoResponse {
    match state.credential_store.revoke_api_key(id).await {
        Ok(()) => {
            state
                .credential_store
                .audit_log("key_revoked", None, Some(&format!("id={id}")))
                .await;
            Json(serde_json::json!({ "ok": true })).into_response()
        },
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ── Passkeys (require session) ───────────────────────────────────────────────

async fn list_passkeys_handler(
    _session: AuthSession,
    State(state): State<AuthState>,
) -> impl IntoResponse {
    match state.credential_store.list_passkeys().await {
        Ok(passkeys) => Json(serde_json::json!({ "passkeys": passkeys })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn remove_passkey_handler(
    _session: AuthSession,
    State(state): State<AuthState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> impl IntoResponse {
    let was_setup_complete = state.credential_store.is_setup_complete();
    match state.credential_store.remove_passkey(id).await {
        Ok(()) => {
            // If removing the last credential flipped setup_complete from true→false,
            // disconnect all clients so they are forced through re-setup.
            if was_setup_complete && !state.credential_store.is_setup_complete() {
                state
                    .gateway_state
                    .disconnect_all_clients("last_credential_removed")
                    .await;
            }
            Json(serde_json::json!({ "ok": true })).into_response()
        },
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(serde::Deserialize)]
struct RenamePasskeyRequest {
    name: String,
}

async fn rename_passkey_handler(
    _session: AuthSession,
    State(state): State<AuthState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
    Json(body): Json<RenamePasskeyRequest>,
) -> impl IntoResponse {
    let name = body.name.trim();
    if name.is_empty() {
        return (StatusCode::BAD_REQUEST, "name cannot be empty").into_response();
    }
    match state.credential_store.rename_passkey(id, name).await {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Build a session cookie string, adding `Domain=localhost` when the request
/// arrived on a `.localhost` subdomain (e.g. `moltis.localhost`) so the cookie
/// is shared across all loopback names per RFC 6761.
fn session_response(
    token: String,
    headers: &axum::http::HeaderMap,
    behind_proxy: bool,
    secure: bool,
) -> axum::response::Response {
    let domain_attr = localhost_cookie_domain(headers, behind_proxy);
    let secure_attr = if secure {
        "; Secure"
    } else {
        ""
    };
    let cookie = format!(
        "{SESSION_COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age=2592000{domain_attr}{secure_attr}"
    );
    (
        StatusCode::OK,
        [(axum::http::header::SET_COOKIE, cookie)],
        Json(serde_json::json!({ "ok": true })),
    )
        .into_response()
}

fn clear_session_response(
    headers: &axum::http::HeaderMap,
    behind_proxy: bool,
    secure: bool,
) -> axum::response::Response {
    let domain_attr = localhost_cookie_domain(headers, behind_proxy);
    let secure_attr = if secure {
        "; Secure"
    } else {
        ""
    };
    let cookie = format!(
        "{SESSION_COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0{domain_attr}{secure_attr}"
    );
    (
        StatusCode::OK,
        [(axum::http::header::SET_COOKIE, cookie)],
        Json(serde_json::json!({ "ok": true })),
    )
        .into_response()
}

/// Return `; Domain=localhost` when the request's `Host` header is a
/// `.localhost` subdomain (e.g. `moltis.localhost:8080`), otherwise `""`.
///
/// Without this, a session cookie set on `localhost` isn't sent by the browser
/// to `moltis.localhost` and vice versa because `Set-Cookie` without a `Domain`
/// attribute is a host-only cookie.  Adding `Domain=localhost` makes the
/// cookie available to `localhost` **and** all its subdomains (RFC 6265 §5.2.3).
fn localhost_cookie_domain(headers: &axum::http::HeaderMap, behind_proxy: bool) -> &'static str {
    let Some(host) = cookie_host(headers, behind_proxy) else {
        return "";
    };

    // Strip port.
    let name = host.rsplit_once(':').map_or(host, |(h, _)| h);

    // Behind a proxy we only add Domain=localhost for explicit .localhost
    // forwarded hosts. This avoids setting an invalid Domain when proxies
    // rewrite Host to the upstream loopback address.
    if name.ends_with(".localhost") || (!behind_proxy && name == "localhost") {
        "; Domain=localhost"
    } else {
        ""
    }
}

fn cookie_host(headers: &axum::http::HeaderMap, behind_proxy: bool) -> Option<&str> {
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty());

    if !behind_proxy {
        return host;
    }

    headers
        .get("x-forwarded-host")
        .and_then(|v| v.to_str().ok())
        .and_then(|raw| raw.split(',').next())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .or(host)
}
// ── Passkey registration (requires session) ──────────────────────────────────

async fn passkey_register_begin_handler(
    _session: AuthSession,
    State(state): State<AuthState>,
    Host(host): Host,
) -> impl IntoResponse {
    let Some(ref registry) = state.webauthn_registry else {
        return (StatusCode::NOT_IMPLEMENTED, "passkeys not configured").into_response();
    };
    let Some(wa) = host_to_webauthn(&host, registry).await else {
        return (
            StatusCode::BAD_REQUEST,
            "no passkey config for this hostname",
        )
            .into_response();
    };

    let existing = moltis_gateway::auth_webauthn::load_passkeys(&state.credential_store)
        .await
        .unwrap_or_default();

    match wa.start_registration(&existing) {
        Ok((challenge_id, ccr)) => Json(serde_json::json!({
            "challenge_id": challenge_id,
            "options": ccr,
        }))
        .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(serde::Deserialize)]
struct PasskeyRegisterFinishRequest {
    challenge_id: String,
    name: String,
    credential: webauthn_rs::prelude::RegisterPublicKeyCredential,
}

async fn passkey_register_finish_handler(
    _session: AuthSession,
    State(state): State<AuthState>,
    Host(host): Host,
    Json(body): Json<PasskeyRegisterFinishRequest>,
) -> impl IntoResponse {
    let Some(ref registry) = state.webauthn_registry else {
        return (StatusCode::NOT_IMPLEMENTED, "passkeys not configured").into_response();
    };
    let Some(wa) = host_to_webauthn(&host, registry).await else {
        return (
            StatusCode::BAD_REQUEST,
            "no passkey config for this hostname",
        )
            .into_response();
    };

    let passkey = match wa.finish_registration(&body.challenge_id, &body.credential) {
        Ok(pk) => pk,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };

    let cred_id = passkey.cred_id().as_ref();
    let data = match serde_json::to_vec(&passkey) {
        Ok(d) => d,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    let name = if body.name.trim().is_empty() {
        "Passkey"
    } else {
        body.name.trim()
    };

    match state
        .credential_store
        .store_passkey(cred_id, name, &data)
        .await
    {
        Ok(id) => {
            state
                .gateway_state
                .clear_passkey_host_update_pending(&host)
                .await;
            Json(serde_json::json!({ "id": id })).into_response()
        },
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ── Passkey authentication (no session required) ─────────────────────────────

async fn passkey_auth_begin_handler(
    State(state): State<AuthState>,
    Host(host): Host,
) -> impl IntoResponse {
    let Some(ref registry) = state.webauthn_registry else {
        return (StatusCode::NOT_IMPLEMENTED, "passkeys not configured").into_response();
    };
    let Some(wa) = host_to_webauthn(&host, registry).await else {
        return (
            StatusCode::BAD_REQUEST,
            "no passkey config for this hostname",
        )
            .into_response();
    };

    let passkeys = match moltis_gateway::auth_webauthn::load_passkeys(&state.credential_store).await
    {
        Ok(pks) => pks,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    match wa.start_authentication(&passkeys) {
        Ok((challenge_id, rcr)) => Json(serde_json::json!({
            "challenge_id": challenge_id,
            "options": rcr,
        }))
        .into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

#[derive(serde::Deserialize)]
struct PasskeyAuthFinishRequest {
    challenge_id: String,
    credential: webauthn_rs::prelude::PublicKeyCredential,
}

async fn passkey_auth_finish_handler(
    State(state): State<AuthState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Host(host): Host,
    headers: axum::http::HeaderMap,
    Json(body): Json<PasskeyAuthFinishRequest>,
) -> impl IntoResponse {
    let client_ip = crate::request_throttle::resolve_client_ip(
        &headers,
        addr,
        state.gateway_state.behind_proxy,
    );
    const PASSKEY_ACCOUNT: &str = "__passkey__";

    if let Some(block) = state.login_guard.check(client_ip, PASSKEY_ACCOUNT) {
        return blocked_response(block);
    }

    let Some(ref registry) = state.webauthn_registry else {
        return (StatusCode::NOT_IMPLEMENTED, "passkeys not configured").into_response();
    };
    let Some(wa) = host_to_webauthn(&host, registry).await else {
        return (
            StatusCode::BAD_REQUEST,
            "no passkey config for this hostname",
        )
            .into_response();
    };

    match wa.finish_authentication(&body.challenge_id, &body.credential) {
        Ok(_result) => {
            state.login_guard.record_success(client_ip);
            match state.credential_store.create_session().await {
                Ok(token) => {
                    let bp = state.gateway_state.behind_proxy;
                    session_response(token, &headers, bp, state.gateway_state.tls_active || bp)
                },
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
            }
        },
        Err(e) => {
            state.login_guard.record_failure(client_ip, PASSKEY_ACCOUNT);
            (StatusCode::UNAUTHORIZED, e.to_string()).into_response()
        },
    }
}

// ── Setup-time passkey registration (setup code instead of session) ───────────

#[derive(serde::Deserialize)]
struct SetupPasskeyBeginRequest {
    setup_code: Option<String>,
}

async fn setup_passkey_register_begin_handler(
    State(state): State<AuthState>,
    Host(host): Host,
    Json(body): Json<SetupPasskeyBeginRequest>,
) -> impl IntoResponse {
    if state.credential_store.is_setup_complete() {
        return (StatusCode::FORBIDDEN, "setup already completed").into_response();
    }

    // Validate setup code if one was generated at startup.
    {
        let inner = state.gateway_state.inner.read().await;
        if let Some(ref expected) = inner.setup_code
            && body.setup_code.as_deref() != Some(expected.expose_secret().as_str())
        {
            return (StatusCode::FORBIDDEN, "invalid or missing setup code").into_response();
        }
    }

    let Some(ref registry) = state.webauthn_registry else {
        return (StatusCode::NOT_IMPLEMENTED, "passkeys not configured").into_response();
    };
    let Some(wa) = host_to_webauthn(&host, registry).await else {
        return (
            StatusCode::BAD_REQUEST,
            "no passkey config for this hostname",
        )
            .into_response();
    };

    let existing = moltis_gateway::auth_webauthn::load_passkeys(&state.credential_store)
        .await
        .unwrap_or_default();

    match wa.start_registration(&existing) {
        Ok((challenge_id, ccr)) => Json(serde_json::json!({
            "challenge_id": challenge_id,
            "options": ccr,
        }))
        .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(serde::Deserialize)]
struct SetupPasskeyFinishRequest {
    challenge_id: String,
    name: String,
    setup_code: Option<String>,
    credential: webauthn_rs::prelude::RegisterPublicKeyCredential,
}

async fn setup_passkey_register_finish_handler(
    State(state): State<AuthState>,
    Host(host): Host,
    headers: axum::http::HeaderMap,
    Json(body): Json<SetupPasskeyFinishRequest>,
) -> impl IntoResponse {
    if state.credential_store.is_setup_complete() {
        return (StatusCode::FORBIDDEN, "setup already completed").into_response();
    }

    // Validate setup code if one was generated at startup.
    {
        let inner = state.gateway_state.inner.read().await;
        if let Some(ref expected) = inner.setup_code
            && body.setup_code.as_deref() != Some(expected.expose_secret().as_str())
        {
            return (StatusCode::FORBIDDEN, "invalid or missing setup code").into_response();
        }
    }

    let Some(ref registry) = state.webauthn_registry else {
        return (StatusCode::NOT_IMPLEMENTED, "passkeys not configured").into_response();
    };
    let Some(wa) = host_to_webauthn(&host, registry).await else {
        return (
            StatusCode::BAD_REQUEST,
            "no passkey config for this hostname",
        )
            .into_response();
    };

    let passkey = match wa.finish_registration(&body.challenge_id, &body.credential) {
        Ok(pk) => pk,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };

    let cred_id = passkey.cred_id().as_ref();
    let data = match serde_json::to_vec(&passkey) {
        Ok(d) => d,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    let name = if body.name.trim().is_empty() {
        "Passkey"
    } else {
        body.name.trim()
    };

    if let Err(e) = state
        .credential_store
        .store_passkey(cred_id, name, &data)
        .await
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }

    state
        .gateway_state
        .clear_passkey_host_update_pending(&host)
        .await;

    if let Err(e) = state.credential_store.mark_setup_complete().await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to mark setup complete: {e}"),
        )
            .into_response();
    }

    // Disconnect pre-setup WebSocket clients and clear setup code.
    state
        .gateway_state
        .disconnect_all_clients("setup_complete")
        .await;
    state.gateway_state.inner.write().await.setup_code = None;
    match state.credential_store.create_session().await {
        Ok(token) => {
            let bp = state.gateway_state.behind_proxy;
            session_response(token, &headers, bp, state.gateway_state.tls_active || bp)
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to create session: {e}"),
        )
            .into_response(),
    }
}

/// Look up the `WebAuthnState` matching the request hostname (`Host` or
/// `:authority`). Matching includes RP IDs and explicitly allowed origin hosts.
async fn host_to_webauthn(
    host: &str,
    registry: &SharedWebAuthnRegistry,
) -> Option<Arc<moltis_gateway::auth_webauthn::WebAuthnState>> {
    registry.read().await.get_for_host(host)
}

fn extract_session_token(headers: &axum::http::HeaderMap) -> Option<&str> {
    let cookie_header = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())?;
    crate::auth_middleware::parse_cookie(cookie_header, SESSION_COOKIE)
}

mod vault;
