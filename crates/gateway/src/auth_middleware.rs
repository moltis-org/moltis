use std::sync::Arc;

use axum::{
    extract::{FromRef, FromRequestParts, State},
    http::{StatusCode, request::Parts},
    middleware::Next,
    response::{IntoResponse, Json},
};

use crate::{
    auth::{AuthIdentity, AuthMethod, CredentialStore},
    state::GatewayState,
};

/// Session cookie name.
pub const SESSION_COOKIE: &str = "moltis_session";

/// Axum extractor that validates the session cookie and produces an
/// `AuthIdentity`. Returns 401 if the session is missing or invalid.
pub struct AuthSession(pub AuthIdentity);

impl<S> FromRequestParts<S> for AuthSession
where
    S: Send + Sync,
    Arc<CredentialStore>: FromRef<S>,
    Arc<GatewayState>: FromRef<S>,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let store = Arc::<CredentialStore>::from_ref(state);
        let gw = Arc::<GatewayState>::from_ref(state);

        // On localhost with no password, grant access without a session.
        if gw.localhost_only && !store.has_password().await.unwrap_or(true) {
            return Ok(AuthSession(AuthIdentity {
                method: AuthMethod::Loopback,
            }));
        }

        let cookie_header = parts
            .headers
            .get(axum::http::header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        let token = parse_cookie(cookie_header, SESSION_COOKIE);

        if let Some(token) = token
            && store.validate_session(token).await.unwrap_or(false)
        {
            return Ok(AuthSession(AuthIdentity {
                method: AuthMethod::Password,
            }));
        }

        Err((StatusCode::UNAUTHORIZED, "not authenticated"))
    }
}

/// Middleware that protects routes behind authentication.
///
/// When no credential store is configured or setup isn't complete, all requests
/// pass through (backward compat). Otherwise, validates either a session cookie
/// or an `Authorization: Bearer <api_key>` header.
pub async fn require_auth(
    State(state): axum::extract::State<super::server::AppState>,
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> axum::response::Response {
    let Some(ref cred_store) = state.gateway.credential_store else {
        // No credential store configured — pass through.
        return next.run(request).await;
    };

    if cred_store.is_auth_disabled() {
        // Auth explicitly disabled via "remove all auth" — pass through.
        return next.run(request).await;
    }

    if !cred_store.is_setup_complete() {
        // Setup not yet done — pass through.
        return next.run(request).await;
    }

    // Check session cookie.
    let cookie_header = request
        .headers()
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if let Some(token) = parse_cookie(cookie_header, SESSION_COOKIE)
        && cred_store.validate_session(token).await.unwrap_or(false)
    {
        return next.run(request).await;
    }

    // Check Authorization: Bearer <api_key>.
    if let Some(auth_header) = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        && let Some(key) = auth_header.strip_prefix("Bearer ")
        && cred_store
            .verify_api_key(key)
            .await
            .ok()
            .flatten()
            .is_some()
    {
        return next.run(request).await;
    }

    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({"error": "not authenticated"})),
    )
        .into_response()
}

/// Parse a specific cookie value from a Cookie header string.
pub fn parse_cookie<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    for part in header.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix(name)
            && let Some(value) = value.strip_prefix('=')
        {
            return Some(value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cookie() {
        assert_eq!(
            parse_cookie("moltis_session=abc123; other=def", "moltis_session"),
            Some("abc123")
        );
        assert_eq!(
            parse_cookie("other=def; moltis_session=xyz", "moltis_session"),
            Some("xyz")
        );
        assert_eq!(parse_cookie("other=def", "moltis_session"), None);
        assert_eq!(parse_cookie("", "moltis_session"), None);
    }
}
