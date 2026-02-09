use std::sync::Arc;

use secrecy::ExposeSecret;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
};

use crate::{
    auth::CredentialStore,
    auth_middleware::{AuthSession, SESSION_COOKIE},
    auth_webauthn::WebAuthnState,
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
        .route("/reset", post(reset_auth_handler))
}

// ── Status ───────────────────────────────────────────────────────────────────

async fn status_handler(
    State(state): State<AuthState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let auth_disabled = state.credential_store.is_auth_disabled();
    let localhost_only = state.gateway_state.localhost_only;
    let has_password = state.credential_store.has_password().await.unwrap_or(false);
    let has_passkeys = state.credential_store.has_passkeys().await.unwrap_or(false);

    // Localhost with no password is treated as fully open (no auth needed).
    let auth_bypassed = auth_disabled || (localhost_only && !has_password);

    let authenticated = if auth_bypassed {
        true
    } else {
        let token = extract_session_token(&headers);
        match token {
            Some(t) => state
                .credential_store
                .validate_session(t)
                .await
                .unwrap_or(false),
            None => false,
        }
    };

    let setup_required = !auth_bypassed && !state.credential_store.is_setup_complete();

    let setup_code_required = state.gateway_state.inner.read().await.setup_code.is_some();

    Json(serde_json::json!({
        "setup_required": setup_required,
        "has_passkeys": has_passkeys,
        "authenticated": authenticated,
        "auth_disabled": auth_disabled,
        "setup_code_required": setup_code_required,
        "has_password": has_password,
        "localhost_only": localhost_only,
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

    if password.is_empty() && state.gateway_state.localhost_only {
        // Localhost with no password: skip setup without setting one.
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
        Ok(token) => session_response(token),
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
    Json(body): Json<LoginRequest>,
) -> impl IntoResponse {
    match state.credential_store.verify_password(&body.password).await {
        Ok(true) => match state.credential_store.create_session().await {
            Ok(token) => session_response(token),
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
    clear_session_response()
}

// ── Reset all auth (requires session) ─────────────────────────────────────────

async fn reset_auth_handler(
    _session: AuthSession,
    State(state): State<AuthState>,
) -> impl IntoResponse {
    match state.credential_store.reset_all().await {
        Ok(()) => {
            // Generate a new setup code so the re-setup flow is protected.
            let code = generate_setup_code();
            tracing::info!("setup code: {code} (enter this in the browser to set your password)");
            state.gateway_state.inner.write().await.setup_code = Some(secrecy::Secret::new(code));
            clear_session_response()
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
        // No password set yet — use set_initial_password (no current password needed).
        return match state
            .credential_store
            .set_initial_password(&body.new_password)
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

fn session_response(token: String) -> axum::response::Response {
    let cookie =
        format!("{SESSION_COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age=2592000");
    (
        StatusCode::OK,
        [(axum::http::header::SET_COOKIE, cookie)],
        Json(serde_json::json!({ "ok": true })),
    )
        .into_response()
}

fn clear_session_response() -> axum::response::Response {
    let cookie = format!("{SESSION_COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0");
    (
        StatusCode::OK,
        [(axum::http::header::SET_COOKIE, cookie)],
        Json(serde_json::json!({ "ok": true })),
    )
        .into_response()
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
    Json(body): Json<PasskeyAuthFinishRequest>,
) -> impl IntoResponse {
    let Some(ref wa) = state.webauthn_state else {
        return (StatusCode::NOT_IMPLEMENTED, "passkeys not configured").into_response();
    };

    match wa.finish_authentication(&body.challenge_id, &body.credential) {
        Ok(_result) => match state.credential_store.create_session().await {
            Ok(token) => session_response(token),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        },
        Err(e) => (StatusCode::UNAUTHORIZED, e.to_string()).into_response(),
    }
}

fn extract_session_token(headers: &axum::http::HeaderMap) -> Option<&str> {
    let cookie_header = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())?;
    crate::auth_middleware::parse_cookie(cookie_header, SESSION_COOKIE)
}
