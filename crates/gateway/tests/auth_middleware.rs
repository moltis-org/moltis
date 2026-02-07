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
    start_auth_server_impl(false).await
}

/// Start a localhost-only test server.
async fn start_localhost_server() -> (SocketAddr, Arc<CredentialStore>, Arc<GatewayState>) {
    start_auth_server_impl(true).await
}

async fn start_auth_server_impl(
    localhost_only: bool,
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
        Arc::new(moltis_tools::approval::ApprovalManager::default()),
        None,
        Some(Arc::clone(&cred_store)),
        None,
        localhost_only,
        false,
        None,
        None,
        18789,
        None,
        #[cfg(feature = "metrics")]
        None,
        #[cfg(feature = "metrics")]
        None,
    );
    let state_clone = Arc::clone(&state);
    let methods = Arc::new(MethodRegistry::new());
    #[cfg(feature = "push-notifications")]
    let app = build_gateway_app(state, methods, None);
    #[cfg(not(feature = "push-notifications"))]
    let app = build_gateway_app(state, methods);

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
    let state = GatewayState::new(
        resolved_auth,
        services,
        Arc::new(moltis_tools::approval::ApprovalManager::default()),
    );
    let methods = Arc::new(MethodRegistry::new());
    #[cfg(feature = "push-notifications")]
    let app = build_gateway_app(state, methods, None);
    #[cfg(not(feature = "push-notifications"))]
    let app = build_gateway_app(state, methods);

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
        .setup_code
        .read()
        .await
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
    *state.setup_code.write().await = Some(secrecy::Secret::new("123456".to_string()));

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
    *state.setup_code.write().await = Some(secrecy::Secret::new("123456".to_string()));

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
    *state.setup_code.write().await = Some(secrecy::Secret::new("123456".to_string()));

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
    assert!(state.setup_code.read().await.is_none());
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
#[cfg(feature = "web-ui")]
#[tokio::test]
async fn status_reports_setup_code_required() {
    let (addr, _store, state) = start_auth_server_with_state().await;
    *state.setup_code.write().await = Some(secrecy::Secret::new("654321".to_string()));

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
