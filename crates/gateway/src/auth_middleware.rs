use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{ConnectInfo, FromRef, FromRequestParts, State},
    http::{HeaderMap, StatusCode, request::Parts},
    middleware::Next,
    response::{IntoResponse, Json},
};

#[cfg(feature = "web-ui")]
use axum::response::Redirect;

use crate::{
    auth::{AuthIdentity, AuthMethod, CredentialStore},
    server::is_local_connection,
    state::GatewayState,
};

/// Session cookie name.
pub const SESSION_COOKIE: &str = "moltis_session";

// ── AuthResult — single source of truth for auth decisions ──────────────────

/// Outcome of an auth check against a credential store.
#[derive(Debug, Clone)]
pub enum AuthResult {
    /// Request is authorized.
    Allowed(AuthIdentity),
    /// No credentials configured yet; only local connections may pass.
    SetupRequired,
    /// Credentials exist but request is not authenticated.
    Unauthorized,
}

/// Single source of truth for auth decisions.
///
/// Every code path that needs to decide "is this request authenticated?" must
/// call this function instead of reimplementing the logic. This prevents the
/// split-brain bugs that arise when `is_setup_complete()` and `has_password()`
/// diverge (e.g. passkey-only setups).
pub async fn check_auth(
    store: &CredentialStore,
    headers: &HeaderMap,
    is_local: bool,
) -> AuthResult {
    if store.is_auth_disabled() {
        return AuthResult::Allowed(AuthIdentity {
            method: AuthMethod::Loopback,
        });
    }

    if !store.is_setup_complete() {
        return if is_local {
            AuthResult::Allowed(AuthIdentity {
                method: AuthMethod::Loopback,
            })
        } else {
            AuthResult::SetupRequired
        };
    }

    // Check session cookie.
    if let Some(token) = cookie_header(headers).and_then(|h| parse_cookie(h, SESSION_COOKIE))
        && store.validate_session(token).await.unwrap_or(false)
    {
        return AuthResult::Allowed(AuthIdentity {
            method: AuthMethod::Password,
        });
    }

    // Check Bearer API key.
    if let Some(key) = bearer_token(headers)
        && store.verify_api_key(key).await.ok().flatten().is_some()
    {
        return AuthResult::Allowed(AuthIdentity {
            method: AuthMethod::ApiKey,
        });
    }

    AuthResult::Unauthorized
}

// ── auth_gate — covers the entire router ────────────────────────────────────

/// Middleware that applies auth to **all** routes.
///
/// Public paths (assets, auth endpoints, health, etc.) are allowed through
/// without authentication. Everything else goes through [`check_auth()`].
#[cfg(feature = "web-ui")]
pub async fn auth_gate(
    State(state): State<super::server::AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    mut request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> axum::response::Response {
    let path = request.uri().path();

    // Public paths — no auth needed.
    if is_public_path(path) {
        return next.run(request).await;
    }

    let Some(ref store) = state.gateway.credential_store else {
        // No credential store configured — pass through.
        return next.run(request).await;
    };

    let is_local = is_local_connection(request.headers(), addr, state.gateway.behind_proxy);

    match check_auth(store, request.headers(), is_local).await {
        AuthResult::Allowed(identity) => {
            request.extensions_mut().insert(identity);
            next.run(request).await
        },
        AuthResult::SetupRequired => {
            if path == "/onboarding" {
                // Allow the onboarding page through during setup — it is only
                // public while setup is incomplete. Once credentials are
                // configured, normal auth applies and prevents access.
                request.extensions_mut().insert(AuthIdentity {
                    method: AuthMethod::Loopback,
                });
                next.run(request).await
            } else if path.starts_with("/api/") || path == "/ws" {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({"error": "setup required"})),
                )
                    .into_response()
            } else {
                Redirect::to("/onboarding").into_response()
            }
        },
        AuthResult::Unauthorized => {
            if path.starts_with("/api/") || path == "/ws" {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({"error": "not authenticated"})),
                )
                    .into_response()
            } else {
                Redirect::to("/login").into_response()
            }
        },
    }
}

/// Paths that never require authentication.
#[cfg(feature = "web-ui")]
fn is_public_path(path: &str) -> bool {
    matches!(
        path,
        "/health" | "/auth/callback" | "/manifest.json" | "/sw.js" | "/login"
    ) || path.starts_with("/api/auth/")
        || path.starts_with("/assets/")
        || path.starts_with("/share/")
}

// ── AuthSession extractor ───────────────────────────────────────────────────

/// Axum extractor that validates the session cookie and produces an
/// `AuthIdentity`. Returns 401 if the session is missing or invalid.
///
/// When `auth_gate` has already run, it reads the [`AuthIdentity`] the
/// middleware inserted into extensions. For auth routes (on the public
/// allowlist, where `auth_gate` skips auth), it falls back to validating
/// the session cookie directly.
pub struct AuthSession(pub AuthIdentity);

impl<S> FromRequestParts<S> for AuthSession
where
    S: Send + Sync,
    Arc<CredentialStore>: FromRef<S>,
    Arc<GatewayState>: FromRef<S>,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        // If auth_gate already ran and set identity, use it.
        if let Some(id) = parts.extensions.get::<AuthIdentity>() {
            return Ok(AuthSession(id.clone()));
        }

        // Fallback for auth routes (allowlisted, middleware skipped):
        // validate session cookie directly, or check the local-bypass logic.
        let store = Arc::<CredentialStore>::from_ref(state);
        let gw = Arc::<GatewayState>::from_ref(state);

        let is_local = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .is_some_and(|ci| is_local_connection(&parts.headers, ci.0, gw.behind_proxy));

        match check_auth(&store, &parts.headers, is_local).await {
            AuthResult::Allowed(identity) => Ok(AuthSession(identity)),
            _ => Err((StatusCode::UNAUTHORIZED, "not authenticated")),
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Extract the Cookie header value.
fn cookie_header(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
}

/// Extract a Bearer token from the Authorization header.
fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
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
