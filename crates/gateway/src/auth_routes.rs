use std::{net::SocketAddr, sync::Arc};

use secrecy::ExposeSecret;

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
};

use crate::{
    auth::CredentialStore,
    auth_middleware::{AuthResult, AuthSession, SESSION_COOKIE, check_auth},
    auth_webauthn::WebAuthnState,
    server::is_local_connection,
    state::GatewayState,
};

/// Auth-related application state.
#[derive(Clone)]
pub struct AuthState {
    pub credential_store: Arc<CredentialStore>,
    pub webauthn_state: Option<Arc<WebAuthnState>>,
    pub gateway_state: Arc<GatewayState>,
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

    let webauthn_available = state.webauthn_state.is_some();

    let passkey_origins: Vec<String> = state
        .webauthn_state
        .as_ref()
        .map(|wa| wa.get_allowed_origins())
        .unwrap_or_default();

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

    // Validate setup code if one was generated at startup.
    {
        let inner = state.gateway_state.inner.read().await;
        if let Some(ref expected) = inner.setup_code
            && body.setup_code.as_deref() != Some(expected.expose_secret().as_str())
        {
            return (StatusCode::FORBIDDEN, "invalid or missing setup code").into_response();
        }
    }

    let password = body.password.unwrap_or_default();

    let is_local = is_local_connection(&headers, addr, state.gateway_state.behind_proxy);
    if password.is_empty() && is_local {
        // Local connection with no password: skip setup without setting one.
        state.credential_store.clear_auth_disabled();
    } else {
        if password.len() < 8 {
            return (
                StatusCode::BAD_REQUEST,
                "password must be at least 8 characters",
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

    // Clear setup code and create session.
    state.gateway_state.inner.write().await.setup_code = None;
    match state.credential_store.create_session().await {
        Ok(token) => session_response(token, &headers),
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
    headers: axum::http::HeaderMap,
    Json(body): Json<LoginRequest>,
) -> impl IntoResponse {
    match state.credential_store.verify_password(&body.password).await {
        Ok(true) => match state.credential_store.create_session().await {
            Ok(token) => session_response(token, &headers),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("session error: {e}"),
            )
                .into_response(),
        },
        Ok(false) => (StatusCode::UNAUTHORIZED, "invalid password").into_response(),
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
    clear_session_response(&headers)
}

// ── Reset all auth (requires session) ─────────────────────────────────────────

async fn reset_auth_handler(
    _session: AuthSession,
    State(state): State<AuthState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    match state.credential_store.reset_all().await {
        Ok(()) => {
            // Generate a new setup code so the re-setup flow is protected.
            let code = generate_setup_code();
            tracing::info!("setup code: {code} (enter this in the browser to set your password)");
            state.gateway_state.inner.write().await.setup_code = Some(secrecy::Secret::new(code));
            clear_session_response(&headers)
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
    Json(body): Json<ChangePasswordRequest>,
) -> impl IntoResponse {
    if body.new_password.len() < 8 {
        return (
            StatusCode::BAD_REQUEST,
            "new password must be at least 8 characters",
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
            Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        };
    }

    let current_password = body.current_password.unwrap_or_default();
    match state
        .credential_store
        .change_password(&current_password, &body.new_password)
        .await
    {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("incorrect") {
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
            if !crate::auth::VALID_SCOPES.contains(&scope.as_str()) {
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
        Ok((id, key)) => Json(serde_json::json!({ "id": id, "key": key })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn revoke_api_key_handler(
    _session: AuthSession,
    State(state): State<AuthState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> impl IntoResponse {
    match state.credential_store.revoke_api_key(id).await {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
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
    match state.credential_store.remove_passkey(id).await {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
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

/// Generate a random 6-digit numeric setup code.
pub fn generate_setup_code() -> String {
    use rand::Rng;
    rand::rng().random_range(100_000..1_000_000).to_string()
}

/// Build a session cookie string, adding `Domain=localhost` when the request
/// arrived on a `.localhost` subdomain (e.g. `moltis.localhost`) so the cookie
/// is shared across all loopback names per RFC 6761.
fn session_response(token: String, headers: &axum::http::HeaderMap) -> axum::response::Response {
    let domain_attr = localhost_cookie_domain(headers);
    let cookie = format!(
        "{SESSION_COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age=2592000{domain_attr}"
    );
    (
        StatusCode::OK,
        [(axum::http::header::SET_COOKIE, cookie)],
        Json(serde_json::json!({ "ok": true })),
    )
        .into_response()
}

fn clear_session_response(headers: &axum::http::HeaderMap) -> axum::response::Response {
    let domain_attr = localhost_cookie_domain(headers);
    let cookie =
        format!("{SESSION_COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0{domain_attr}");
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
fn localhost_cookie_domain(headers: &axum::http::HeaderMap) -> &'static str {
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // Strip port.
    let name = host.rsplit_once(':').map_or(host, |(h, _)| h);

    if name == "localhost" || name.ends_with(".localhost") {
        "; Domain=localhost"
    } else {
        ""
    }
}

// ── Passkey registration (requires session) ──────────────────────────────────

async fn passkey_register_begin_handler(
    _session: AuthSession,
    State(state): State<AuthState>,
) -> impl IntoResponse {
    let Some(ref wa) = state.webauthn_state else {
        return (StatusCode::NOT_IMPLEMENTED, "passkeys not configured").into_response();
    };

    let existing = crate::auth_webauthn::load_passkeys(&state.credential_store)
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
    Json(body): Json<PasskeyRegisterFinishRequest>,
) -> impl IntoResponse {
    let Some(ref wa) = state.webauthn_state else {
        return (StatusCode::NOT_IMPLEMENTED, "passkeys not configured").into_response();
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
        Ok(id) => Json(serde_json::json!({ "id": id })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ── Passkey authentication (no session required) ─────────────────────────────

async fn passkey_auth_begin_handler(State(state): State<AuthState>) -> impl IntoResponse {
    let Some(ref wa) = state.webauthn_state else {
        return (StatusCode::NOT_IMPLEMENTED, "passkeys not configured").into_response();
    };

    let passkeys = match crate::auth_webauthn::load_passkeys(&state.credential_store).await {
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
    headers: axum::http::HeaderMap,
    Json(body): Json<PasskeyAuthFinishRequest>,
) -> impl IntoResponse {
    let Some(ref wa) = state.webauthn_state else {
        return (StatusCode::NOT_IMPLEMENTED, "passkeys not configured").into_response();
    };

    match wa.finish_authentication(&body.challenge_id, &body.credential) {
        Ok(_result) => match state.credential_store.create_session().await {
            Ok(token) => session_response(token, &headers),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        },
        Err(e) => (StatusCode::UNAUTHORIZED, e.to_string()).into_response(),
    }
}

// ── Setup-time passkey registration (setup code instead of session) ───────────

#[derive(serde::Deserialize)]
struct SetupPasskeyBeginRequest {
    setup_code: Option<String>,
}

async fn setup_passkey_register_begin_handler(
    State(state): State<AuthState>,
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

    let Some(ref wa) = state.webauthn_state else {
        return (StatusCode::NOT_IMPLEMENTED, "passkeys not configured").into_response();
    };

    let existing = crate::auth_webauthn::load_passkeys(&state.credential_store)
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

    let Some(ref wa) = state.webauthn_state else {
        return (StatusCode::NOT_IMPLEMENTED, "passkeys not configured").into_response();
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

    if let Err(e) = state.credential_store.mark_setup_complete().await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to mark setup complete: {e}"),
        )
            .into_response();
    }

    // Clear setup code and create session.
    state.gateway_state.inner.write().await.setup_code = None;
    match state.credential_store.create_session().await {
        Ok(token) => session_response(token, &headers),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to create session: {e}"),
        )
            .into_response(),
    }
}

fn extract_session_token(headers: &axum::http::HeaderMap) -> Option<&str> {
    let cookie_header = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())?;
    crate::auth_middleware::parse_cookie(cookie_header, SESSION_COOKIE)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn headers_with_host(host: &str) -> axum::http::HeaderMap {
        let mut h = axum::http::HeaderMap::new();
        h.insert(
            axum::http::header::HOST,
            host.parse().expect("valid host header"),
        );
        h
    }

    #[test]
    fn localhost_cookie_domain_plain_localhost() {
        let h = headers_with_host("localhost:8080");
        assert_eq!(localhost_cookie_domain(&h), "; Domain=localhost");
    }

    #[test]
    fn localhost_cookie_domain_moltis_subdomain() {
        let h = headers_with_host("moltis.localhost:59263");
        assert_eq!(localhost_cookie_domain(&h), "; Domain=localhost");
    }

    #[test]
    fn localhost_cookie_domain_bare_localhost_no_port() {
        let h = headers_with_host("localhost");
        assert_eq!(localhost_cookie_domain(&h), "; Domain=localhost");
    }

    #[test]
    fn localhost_cookie_domain_external_host_omits_domain() {
        let h = headers_with_host("example.com:443");
        assert_eq!(localhost_cookie_domain(&h), "");
    }

    #[test]
    fn localhost_cookie_domain_tailscale_host_omits_domain() {
        let h = headers_with_host("mybox.tail12345.ts.net:8080");
        assert_eq!(localhost_cookie_domain(&h), "");
    }

    #[test]
    fn localhost_cookie_domain_ip_address_omits_domain() {
        let h = headers_with_host("192.168.1.100:8080");
        assert_eq!(localhost_cookie_domain(&h), "");
    }

    #[test]
    fn localhost_cookie_domain_no_host_header_omits_domain() {
        let h = axum::http::HeaderMap::new();
        assert_eq!(localhost_cookie_domain(&h), "");
    }

    #[test]
    fn session_response_includes_domain_for_localhost() {
        let h = headers_with_host("moltis.localhost:8080");
        let resp = session_response("test-token".into(), &h);
        let cookie = resp
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .expect("login response must set a session cookie")
            .to_str()
            .expect("cookie header must be valid UTF-8");
        assert!(
            cookie.contains("; Domain=localhost"),
            "cookie should include Domain=localhost for .localhost host, got: {cookie}"
        );
        assert!(cookie.contains("moltis_session=test-token"));
    }

    #[test]
    fn session_response_omits_domain_for_external_host() {
        let h = headers_with_host("example.com:443");
        let resp = session_response("test-token".into(), &h);
        let cookie = resp
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .expect("login response must set a session cookie")
            .to_str()
            .expect("cookie header must be valid UTF-8");
        assert!(
            !cookie.contains("Domain="),
            "cookie should NOT include Domain for external host, got: {cookie}"
        );
    }

    #[test]
    fn clear_session_response_includes_domain_for_localhost() {
        let h = headers_with_host("localhost:18080");
        let resp = clear_session_response(&h);
        let cookie = resp
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .expect("clear response must set a session cookie")
            .to_str()
            .expect("cookie header must be valid UTF-8");
        assert!(
            cookie.contains("; Domain=localhost"),
            "clear cookie should include Domain=localhost, got: {cookie}"
        );
        assert!(cookie.contains("Max-Age=0"));
    }
}
