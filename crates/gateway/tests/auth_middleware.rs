#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Integration tests for the auth middleware protecting API endpoints.

use std::{net::SocketAddr, sync::Arc};

use secrecy::ExposeSecret;

use tokio::net::TcpListener;

use moltis_gateway::{
    auth::{self, CredentialStore},
    methods::MethodRegistry,
    server::build_gateway_app,
    services::GatewayServices,
    state::GatewayState,
};

/// Start a test server with a credential store (auth enabled).
async fn start_auth_server() -> (SocketAddr, Arc<CredentialStore>) {
    let (addr, store, _state) = start_auth_server_with_state().await;
    (addr, store)
}

/// Start a test server and also return the GatewayState for setup code tests.
async fn start_auth_server_with_state() -> (SocketAddr, Arc<CredentialStore>, Arc<GatewayState>) {
    start_auth_server_impl(false, false).await
}

/// Start a localhost-only test server.
async fn start_localhost_server() -> (SocketAddr, Arc<CredentialStore>, Arc<GatewayState>) {
    start_auth_server_impl(true, false).await
}

/// Start a test server that simulates being behind a proxy (all connections
/// treated as remote even though they originate from loopback).
async fn start_proxied_server() -> (SocketAddr, Arc<CredentialStore>, Arc<GatewayState>) {
    start_auth_server_impl(false, true).await
}

async fn start_auth_server_impl(
    localhost_only: bool,
    behind_proxy: bool,
) -> (SocketAddr, Arc<CredentialStore>, Arc<GatewayState>) {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    let auth_config = moltis_config::AuthConfig::default();
    let cred_store = Arc::new(
        CredentialStore::with_config(pool, &auth_config)
            .await
            .unwrap(),
    );

    let resolved_auth = auth::resolve_auth(None, None);
    let services = GatewayServices::noop();
    let state = GatewayState::with_options(
        resolved_auth,
        services,
        None,
        Some(Arc::clone(&cred_store)),
        localhost_only,
        behind_proxy,
        false,
        None,
        None,
        18789,
        false,
        None,
        #[cfg(feature = "metrics")]
        None,
        #[cfg(feature = "metrics")]
        None,
    );
    let state_clone = Arc::clone(&state);
    let methods = Arc::new(MethodRegistry::new());
    #[cfg(feature = "push-notifications")]
    let app = build_gateway_app(state, methods, None, false, None);
    #[cfg(not(feature = "push-notifications"))]
    let app = build_gateway_app(state, methods, false, None);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    (addr, cred_store, state_clone)
}

/// Start a test server without a credential store (no auth).
async fn start_noauth_server() -> SocketAddr {
    let resolved_auth = auth::resolve_auth(None, None);
    let services = GatewayServices::noop();
    let state = GatewayState::new(resolved_auth, services);
    let methods = Arc::new(MethodRegistry::new());
    #[cfg(feature = "push-notifications")]
    let app = build_gateway_app(state, methods, None, false, None);
    #[cfg(not(feature = "push-notifications"))]
    let app = build_gateway_app(state, methods, false, None);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    addr
}

/// When no credential store is configured, all API routes pass through.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn no_auth_configured_passes_through() {
    let addr = start_noauth_server().await;
    let resp = reqwest::get(format!("http://{addr}/api/bootstrap"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

/// When auth is configured but setup is not complete (no password set),
/// all API routes pass through.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn setup_not_complete_passes_through() {
    let (addr, _store) = start_auth_server().await;
    // No password set yet, so setup is not complete.
    let resp = reqwest::get(format!("http://{addr}/api/bootstrap"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

/// When auth is configured and setup is complete, unauthenticated requests
/// to protected endpoints return 401.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn unauthenticated_returns_401() {
    let (addr, store) = start_auth_server().await;
    store.set_initial_password("testpass123").await.unwrap();

    let resp = reqwest::get(format!("http://{addr}/api/bootstrap"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "not authenticated");
}

/// Authenticated request with a valid session cookie succeeds.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn session_cookie_auth_succeeds() {
    let (addr, store) = start_auth_server().await;
    store.set_initial_password("testpass123").await.unwrap();
    let token = store.create_session().await.unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{addr}/api/bootstrap"))
        .header("Cookie", format!("moltis_session={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

/// Authenticated request with a valid API key in Bearer header succeeds.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn api_key_auth_succeeds() {
    let (addr, store) = start_auth_server().await;
    store.set_initial_password("testpass123").await.unwrap();
    let (_id, raw_key) = store.create_api_key("test", None).await.unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{addr}/api/bootstrap"))
        .header("Authorization", format!("Bearer {raw_key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

/// Unauthenticated request to /api/images/cached returns 401 when auth is set up.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn images_endpoint_returns_401() {
    let (addr, store) = start_auth_server().await;
    store.set_initial_password("testpass123").await.unwrap();

    let resp = reqwest::get(format!("http://{addr}/api/images/cached"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

/// Public routes remain accessible without auth even when auth is configured.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn public_routes_accessible_without_auth() {
    let (addr, store) = start_auth_server().await;
    store.set_initial_password("testpass123").await.unwrap();

    // /health is always public.
    let resp = reqwest::get(format!("http://{addr}/health")).await.unwrap();
    assert_eq!(resp.status(), 200);

    // /api/auth/status is public.
    let resp = reqwest::get(format!("http://{addr}/api/auth/status"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // SPA fallback (root page) is public.
    let resp = reqwest::get(format!("http://{addr}/")).await.unwrap();
    assert_eq!(resp.status(), 200);
}

/// Invalid session cookie returns 401.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn invalid_session_cookie_returns_401() {
    let (addr, store) = start_auth_server().await;
    store.set_initial_password("testpass123").await.unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{addr}/api/bootstrap"))
        .header("Cookie", "moltis_session=invalid_token")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

/// POST /api/auth/reset removes all auth and subsequent requests pass through.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn reset_auth_removes_all_authentication() {
    let (addr, store) = start_auth_server().await;
    store.set_initial_password("testpass123").await.unwrap();
    let token = store.create_session().await.unwrap();

    // Protected endpoint requires auth.
    let resp = reqwest::get(format!("http://{addr}/api/bootstrap"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // Reset auth (requires session).
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/api/auth/reset"))
        .header("Cookie", format!("moltis_session={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Now auth is disabled, so middleware passes through.
    let resp = reqwest::get(format!("http://{addr}/api/bootstrap"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Auth-disabled mode should also bypass endpoint throttling.
    for _ in 0..220 {
        let resp = reqwest::get(format!("http://{addr}/api/bootstrap"))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            200,
            "requests should not be rate-limited when auth is disabled"
        );
    }

    // /api/auth/status should report authenticated: true, auth_disabled: true.
    let resp = reqwest::get(format!("http://{addr}/api/auth/status"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["authenticated"], true);
    assert_eq!(body["setup_required"], false);
    assert_eq!(body["auth_disabled"], true);
}

/// After resetting auth then re-setting up, auth_disabled is cleared.
/// Reset generates a new setup code that must be provided.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn reenable_auth_after_reset() {
    let (addr, store, state) = start_auth_server_with_state().await;
    store.set_initial_password("testpass123").await.unwrap();
    let token = store.create_session().await.unwrap();

    // Reset auth.
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/api/auth/reset"))
        .header("Cookie", format!("moltis_session={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Reset should have generated a new setup code.
    let code = state
        .inner
        .read()
        .await
        .setup_code
        .as_ref()
        .unwrap()
        .expose_secret()
        .clone();

    // Setup without code should fail.
    let resp = client
        .post(format!("http://{addr}/api/auth/setup"))
        .header("Content-Type", "application/json")
        .body(r#"{"password":"newpass123"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);

    // Re-enable: set up a new password with the correct setup code.
    let resp = client
        .post(format!("http://{addr}/api/auth/setup"))
        .header("Content-Type", "application/json")
        .body(format!(
            r#"{{"password":"newpass123","setup_code":"{code}"}}"#
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Status should show auth_disabled: false, authenticated depends on cookie.
    let resp = reqwest::get(format!("http://{addr}/api/auth/status"))
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["auth_disabled"], false);
    assert_eq!(body["setup_required"], false);

    // Protected endpoints require auth again.
    let resp = reqwest::get(format!("http://{addr}/api/bootstrap"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

/// Reset without session returns 401.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn reset_auth_requires_session() {
    let (addr, store) = start_auth_server().await;
    store.set_initial_password("testpass123").await.unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/api/auth/reset"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

/// Revoked API key returns 401.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn revoked_api_key_returns_401() {
    let (addr, store) = start_auth_server().await;
    store.set_initial_password("testpass123").await.unwrap();
    let (id, raw_key) = store.create_api_key("test", None).await.unwrap();
    store.revoke_api_key(id).await.unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{addr}/api/bootstrap"))
        .header("Authorization", format!("Bearer {raw_key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

// ── Setup code tests ─────────────────────────────────────────────────────────

/// Setup without code when code is required returns 403.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn setup_without_code_when_required_returns_403() {
    let (addr, _store, state) = start_auth_server_with_state().await;
    state.inner.write().await.setup_code = Some(secrecy::Secret::new("123456".to_string()));

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/api/auth/setup"))
        .header("Content-Type", "application/json")
        .body(r#"{"password":"testpass123"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
}

/// Setup with wrong code returns 403.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn setup_with_wrong_code_returns_403() {
    let (addr, _store, state) = start_auth_server_with_state().await;
    state.inner.write().await.setup_code = Some(secrecy::Secret::new("123456".to_string()));

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/api/auth/setup"))
        .header("Content-Type", "application/json")
        .body(r#"{"password":"testpass123","setup_code":"999999"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
}

/// Setup with correct code succeeds.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn setup_with_correct_code_succeeds() {
    let (addr, _store, state) = start_auth_server_with_state().await;
    state.inner.write().await.setup_code = Some(secrecy::Secret::new("123456".to_string()));

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/api/auth/setup"))
        .header("Content-Type", "application/json")
        .body(r#"{"password":"testpass123","setup_code":"123456"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Code should be cleared after successful setup.
    assert!(state.inner.read().await.setup_code.is_none());
}

/// Setup code not required when already set up.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn setup_code_not_required_when_already_setup() {
    let (addr, store, _state) = start_auth_server_with_state().await;
    store.set_initial_password("testpass123").await.unwrap();

    let resp = reqwest::get(format!("http://{addr}/api/auth/status"))
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["setup_code_required"], false);
}

/// Status reports setup_code_required when code is set.
/// Uses a "proxied" server so the local connection is treated as remote
/// (otherwise the three-tier model auto-bypasses auth for local connections
/// without a password, making setup_required = false).
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn status_reports_setup_code_required() {
    let (addr, _store, state) = start_proxied_server().await;
    state.inner.write().await.setup_code = Some(secrecy::Secret::new("654321".to_string()));

    let resp = reqwest::get(format!("http://{addr}/api/auth/status"))
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["setup_code_required"], true);
    assert_eq!(body["setup_required"], true);
}

/// Setup code not required when auth is disabled.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn setup_code_not_required_when_auth_disabled() {
    let (addr, store, _state) = start_auth_server_with_state().await;
    store.set_initial_password("testpass123").await.unwrap();
    let token = store.create_session().await.unwrap();

    // Reset auth to disable it.
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/api/auth/reset"))
        .header("Cookie", format!("moltis_session={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let resp = reqwest::get(format!("http://{addr}/api/auth/status"))
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["auth_disabled"], true);
    // After reset, a setup code is generated so setup_code_required is true.
    assert_eq!(body["setup_code_required"], true);
}

// ── Localhost tests ──────────────────────────────────────────────────────────

/// On localhost with no password, status returns authenticated: true.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn localhost_no_password_status_authenticated() {
    let (addr, _store, _state) = start_localhost_server().await;

    let resp = reqwest::get(format!("http://{addr}/api/auth/status"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["authenticated"], true);
    assert_eq!(body["setup_required"], false);
    assert_eq!(body["has_password"], false);
    assert_eq!(body["localhost_only"], true);
}

/// On localhost with no password, session-protected endpoints work (AuthSession bypass).
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn localhost_no_password_session_endpoints_accessible() {
    let (addr, _store, _state) = start_localhost_server().await;

    // /api/auth/api-keys requires AuthSession — should work on localhost without password.
    let resp = reqwest::get(format!("http://{addr}/api/auth/api-keys"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

/// On localhost with no password, can set a password via /api/auth/password/change.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn localhost_set_password_without_current() {
    let (addr, store, _state) = start_localhost_server().await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/api/auth/password/change"))
        .header("Content-Type", "application/json")
        .body(r#"{"new_password":"newpass123"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Password should now be set.
    assert!(store.has_password().await.unwrap());
    assert!(store.verify_password("newpass123").await.unwrap());

    // After adding a password, localhost bypass should stop applying.
    let status = reqwest::get(format!("http://{addr}/api/auth/status"))
        .await
        .unwrap();
    assert_eq!(status.status(), 200);
    let body: serde_json::Value = status.json().await.unwrap();
    assert_eq!(body["has_password"], true);
    assert_eq!(body["authenticated"], false);

    let protected = reqwest::get(format!("http://{addr}/api/bootstrap"))
        .await
        .unwrap();
    assert_eq!(protected.status(), 401);
}

/// Unauthenticated POST to /api/sessions/:key/upload returns 401.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn upload_endpoint_requires_auth() {
    let (addr, store) = start_auth_server().await;
    store.set_initial_password("testpass123").await.unwrap();

    // Unauthenticated POST should get 401.
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/api/sessions/main/upload"))
        .header("Content-Type", "audio/webm")
        .body(vec![0u8; 100])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // Authenticated POST should NOT get 401 (may get 503 since session store
    // is noop, but definitely not 401).
    let token = store.create_session().await.unwrap();
    let resp = client
        .post(format!("http://{addr}/api/sessions/main/upload"))
        .header("Cookie", format!("moltis_session={token}"))
        .header("Content-Type", "audio/webm")
        .body(vec![0u8; 100])
        .send()
        .await
        .unwrap();
    assert_ne!(resp.status(), 401);
}

/// Unauthenticated GET to /api/sessions/:key/media/:file returns 401.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn media_endpoint_requires_auth() {
    let (addr, store) = start_auth_server().await;
    store.set_initial_password("testpass123").await.unwrap();

    // Unauthenticated GET should get 401.
    let resp = reqwest::get(format!("http://{addr}/api/sessions/main/media/test.png"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // Authenticated GET should NOT get 401.
    let token = store.create_session().await.unwrap();
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{addr}/api/sessions/main/media/test.png"))
        .header("Cookie", format!("moltis_session={token}"))
        .send()
        .await
        .unwrap();
    assert_ne!(resp.status(), 401);
}

/// On localhost with password set, status returns has_password: true.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn localhost_with_password_requires_login() {
    let (addr, store, _state) = start_localhost_server().await;
    store.set_initial_password("testpass123").await.unwrap();

    let resp = reqwest::get(format!("http://{addr}/api/auth/status"))
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["has_password"], true);
    assert_eq!(body["setup_required"], false);
    // Not authenticated without a session.
    assert_eq!(body["authenticated"], false);
}

/// On localhost with a passkey registered, unauthenticated requests require login.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn localhost_with_passkey_requires_login() {
    let (addr, store, _state) = start_localhost_server().await;
    store
        .store_passkey(b"cred-1", "MacBook Touch ID", b"serialized-passkey")
        .await
        .unwrap();

    let status = reqwest::get(format!("http://{addr}/api/auth/status"))
        .await
        .unwrap();
    assert_eq!(status.status(), 200);
    let body: serde_json::Value = status.json().await.unwrap();
    assert_eq!(body["has_passkeys"], true);
    assert_eq!(body["setup_required"], false);
    assert_eq!(body["authenticated"], false);

    let protected = reqwest::get(format!("http://{addr}/api/bootstrap"))
        .await
        .unwrap();
    assert_eq!(protected.status(), 401);
}

// ── Three-tier model tests ──────────────────────────────────────────────────

/// Tier 3: proxied server + no password → protected API returns 401.
/// Remote connections without a password can only reach /api/auth/* for setup.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn proxied_no_password_protected_returns_401() {
    let (addr, _store, _state) = start_proxied_server().await;
    let resp = reqwest::get(format!("http://{addr}/api/bootstrap"))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        401,
        "remote connection without password must not access protected API"
    );
}

/// Tier 3: proxied server + no password → auth status is accessible (public route).
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn proxied_no_password_auth_status_accessible() {
    let (addr, _store, _state) = start_proxied_server().await;
    let resp = reqwest::get(format!("http://{addr}/api/auth/status"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    // Remote connection: not auto-authenticated despite no password.
    assert_eq!(body["authenticated"], false);
    assert_eq!(body["setup_required"], true);
}

/// Tier 1: proxied server + password set → always requires auth.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn proxied_with_password_requires_auth() {
    let (addr, store, _state) = start_proxied_server().await;
    store.set_initial_password("testpass123").await.unwrap();

    let resp = reqwest::get(format!("http://{addr}/api/bootstrap"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // With a valid session, it works.
    let token = store.create_session().await.unwrap();
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{addr}/api/bootstrap"))
        .header("Cookie", format!("moltis_session={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

// ── Cookie domain tests ─────────────────────────────────────────────────────

/// Login via /api/auth/login with a Host header containing a .localhost
/// subdomain (e.g. moltis.localhost) should set Domain=localhost on the
/// session cookie so the cookie is shared across all loopback hostnames.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn login_cookie_includes_domain_for_localhost_subdomain() {
    let (addr, store, _state) = start_localhost_server().await;
    store.set_initial_password("testpass123").await.unwrap();

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let resp = client
        .post(format!("http://{addr}/api/auth/login"))
        .header("Host", "moltis.localhost:18080")
        .header("Content-Type", "application/json")
        .body(r#"{"password":"testpass123"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "login should succeed");

    let cookie_header = resp
        .headers()
        .get("set-cookie")
        .expect("login response must set a session cookie")
        .to_str()
        .unwrap();

    assert!(
        cookie_header.contains("Domain=localhost"),
        "session cookie should include Domain=localhost for .localhost host, got: {cookie_header}"
    );
    assert!(cookie_header.contains("moltis_session="));
}

/// Login with a plain localhost Host should also include Domain=localhost
/// so the cookie works for both localhost and moltis.localhost.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn login_cookie_includes_domain_for_plain_localhost() {
    let (addr, store, _state) = start_localhost_server().await;
    store.set_initial_password("testpass123").await.unwrap();

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let resp = client
        .post(format!("http://{addr}/api/auth/login"))
        .header("Host", "localhost:18080")
        .header("Content-Type", "application/json")
        .body(r#"{"password":"testpass123"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let cookie_header = resp
        .headers()
        .get("set-cookie")
        .expect("login response must set a session cookie")
        .to_str()
        .unwrap();

    assert!(
        cookie_header.contains("Domain=localhost"),
        "session cookie should include Domain=localhost for localhost host, got: {cookie_header}"
    );
}

/// Login with an external Host header should NOT add a Domain attribute
/// to the cookie (host-only cookie, no domain sharing).
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn login_cookie_omits_domain_for_external_host() {
    let (addr, store, _state) = start_localhost_server().await;
    store.set_initial_password("testpass123").await.unwrap();

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let resp = client
        .post(format!("http://{addr}/api/auth/login"))
        .header("Host", "mybox.example.com:443")
        .header("Content-Type", "application/json")
        .body(r#"{"password":"testpass123"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let cookie_header = resp
        .headers()
        .get("set-cookie")
        .expect("login response must set a session cookie")
        .to_str()
        .unwrap();

    assert!(
        !cookie_header.contains("Domain="),
        "session cookie should NOT include Domain for external host, got: {cookie_header}"
    );
}

/// Password login attempts are throttled to reduce brute-force attempts.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn login_endpoint_rate_limited_after_repeated_failures() {
    let (addr, store) = start_auth_server().await;
    store.set_initial_password("testpass123").await.unwrap();

    let client = reqwest::Client::new();

    for _ in 0..5 {
        let resp = client
            .post(format!("http://{addr}/api/auth/login"))
            .header("Content-Type", "application/json")
            .body(r#"{"password":"wrong-password"}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            401,
            "login should fail before throttle engages"
        );
    }

    let throttled = client
        .post(format!("http://{addr}/api/auth/login"))
        .header("Content-Type", "application/json")
        .body(r#"{"password":"wrong-password"}"#)
        .send()
        .await
        .unwrap();

    assert_eq!(throttled.status(), 429);

    let retry_after = throttled
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    assert!(
        retry_after >= 1,
        "expected Retry-After header on throttled login response"
    );
}

/// Normal API endpoints are also throttled with a higher ceiling for regular use.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn api_endpoint_rate_limited_after_high_request_volume() {
    let (addr, store) = start_auth_server().await;
    store.set_initial_password("testpass123").await.unwrap();

    let client = reqwest::Client::new();

    for _ in 0..180 {
        let resp = client
            .get(format!("http://{addr}/api/bootstrap"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            401,
            "unauthenticated protected requests should pass through auth middleware before throttle engages"
        );
    }

    let throttled = client
        .get(format!("http://{addr}/api/bootstrap"))
        .send()
        .await
        .unwrap();

    assert_eq!(throttled.status(), 429);
}

// ── Onboarding auth protection tests ─────────────────────────────────────────

/// During setup (no password), a remote connection to /onboarding is allowed
/// through — the auth gate must not redirect back to /onboarding (which would
/// cause an infinite 303 loop).
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn onboarding_accessible_during_setup_for_remote() {
    let (addr, _store, _state) = start_proxied_server().await;

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let resp = client
        .get(format!("http://{addr}/onboarding"))
        .send()
        .await
        .unwrap();

    // Must NOT be a redirect (especially not 303 to /onboarding).
    assert_ne!(
        resp.status(),
        303,
        "/onboarding must not redirect to itself during setup"
    );
    assert!(
        !resp.status().is_redirection(),
        "/onboarding should serve the page during setup, not redirect"
    );
}

/// After setup is complete, /onboarding requires authentication — an
/// unauthenticated remote request must be redirected to /login.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn onboarding_requires_auth_after_setup() {
    let (addr, store, _state) = start_proxied_server().await;
    store.set_initial_password("testpass123").await.unwrap();

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let resp = client
        .get(format!("http://{addr}/onboarding"))
        .send()
        .await
        .unwrap();

    // After setup, unauthenticated request to /onboarding must redirect to /login.
    assert!(
        resp.status().is_redirection(),
        "/onboarding should redirect when setup is complete and request is unauthenticated"
    );
    let location = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        location, "/login",
        "/onboarding should redirect to /login after setup, not {location}"
    );
}

/// After setup, an authenticated request to /onboarding is allowed through
/// (the onboarding handler itself decides whether to show the page or redirect
/// to /).
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn onboarding_accessible_with_session_after_setup() {
    let (addr, store, _state) = start_proxied_server().await;
    store.set_initial_password("testpass123").await.unwrap();
    let token = store.create_session().await.unwrap();

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let resp = client
        .get(format!("http://{addr}/onboarding"))
        .header("Cookie", format!("moltis_session={token}"))
        .send()
        .await
        .unwrap();

    // Authenticated request must not get 401 or redirect to /login.
    assert_ne!(resp.status(), 401);
    let location = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_ne!(
        location, "/login",
        "authenticated request to /onboarding should not redirect to /login"
    );
}

/// POST /api/auth/setup is rejected with 403 after setup is already complete.
/// This prevents an attacker from resetting the password via the setup endpoint.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn setup_endpoint_rejected_after_setup_complete() {
    let (addr, store, _state) = start_proxied_server().await;
    store.set_initial_password("testpass123").await.unwrap();
    let token = store.create_session().await.unwrap();

    let client = reqwest::Client::new();

    // Even with a valid session, /api/auth/setup must reject once setup is done.
    let resp = client
        .post(format!("http://{addr}/api/auth/setup"))
        .header("Cookie", format!("moltis_session={token}"))
        .header("Content-Type", "application/json")
        .body(r#"{"password":"evil-new-password"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        403,
        "/api/auth/setup must return 403 after setup is complete"
    );
}

/// Authenticated requests bypass IP throttling.
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn authenticated_api_endpoint_not_rate_limited() {
    let (addr, store) = start_auth_server().await;
    store.set_initial_password("testpass123").await.unwrap();
    let token = store.create_session().await.unwrap();

    let client = reqwest::Client::new();

    for _ in 0..220 {
        let resp = client
            .get(format!("http://{addr}/api/bootstrap"))
            .header("Cookie", format!("moltis_session={token}"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            200,
            "authenticated requests should bypass throttling"
        );
    }
}
