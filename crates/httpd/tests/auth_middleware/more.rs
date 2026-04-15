use super::*;

// ── Onboarding auth protection tests ─────────────────────────────────────────

/// During setup (no password), a local connection to /onboarding passes
/// through without redirect — the SPA handles onboarding routing itself.
#[cfg(feature = "web-ui")]
#[tokio::test]
pub(super) async fn onboarding_passes_through_for_local_during_setup() {
    let (addr, _store, _state) = start_localhost_server().await;

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let resp = client
        .get(format!("http://{addr}/onboarding"))
        .send()
        .await
        .unwrap();

    // Local connections must NOT be redirected to /setup-required.
    let location = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_ne!(
        location, "/setup-required",
        "local /onboarding during setup must not redirect to /setup-required"
    );
}

/// During setup (no password), a remote connection to /onboarding also
/// passes through — the onboarding page handles its own auth via setup
/// codes (step 0).
#[cfg(feature = "web-ui")]
#[tokio::test]
pub(super) async fn onboarding_passes_through_for_remote_during_setup() {
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

    // Remote /onboarding must NOT redirect to /setup-required; it has its
    // own setup-code auth flow.
    let location = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_ne!(
        location, "/setup-required",
        "remote /onboarding during setup must not redirect to /setup-required"
    );
}

/// During setup (no password), a remote connection to / is redirected to
/// /onboarding so the user can enter the setup code and complete first-
/// time setup via the wizard's AuthStep (#646).
#[cfg(feature = "web-ui")]
#[tokio::test]
pub(super) async fn root_redirects_to_onboarding_for_remote() {
    let (addr, _store, _state) = start_proxied_server().await;

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let resp = client.get(format!("http://{addr}/")).send().await.unwrap();

    assert!(
        resp.status().is_redirection(),
        "remote / during setup should redirect"
    );
    let location = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        location, "/onboarding",
        "remote / during setup must redirect to /onboarding"
    );
}

/// /setup-required is still served as a public stale-bookmark fallback
/// even for remote connections during setup. It is no longer the default
/// redirect target, but direct navigation must still work and must not
/// redirect-loop.
#[cfg(feature = "web-ui")]
#[tokio::test]
pub(super) async fn setup_required_page_accessible_for_remote() {
    let (addr, _store, _state) = start_proxied_server().await;

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let resp = client
        .get(format!("http://{addr}/setup-required"))
        .send()
        .await
        .unwrap();

    // /setup-required is a public path — must not redirect.
    assert!(
        resp.status().is_success(),
        "/setup-required should serve content, got {}",
        resp.status()
    );
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("First-time setup"),
        "/setup-required should contain the new setup heading"
    );
    assert!(
        body.contains("href=\"/onboarding\""),
        "/setup-required should link to /onboarding"
    );
}

/// After setup is complete, /setup-required redirects to /login so stale
/// bookmarks don't show a misleading "Authentication Not Configured" page.
#[cfg(feature = "web-ui")]
#[tokio::test]
pub(super) async fn setup_required_redirects_to_login_after_setup() {
    let (addr, store, _state) = start_proxied_server().await;
    store.set_initial_password("testpass12345").await.unwrap();

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let resp = client
        .get(format!("http://{addr}/setup-required"))
        .send()
        .await
        .unwrap();

    assert!(
        resp.status().is_redirection(),
        "/setup-required should redirect after setup, got {}",
        resp.status()
    );
    let location = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        location, "/login",
        "/setup-required should redirect to /login after setup"
    );
}

/// After setup is complete, /onboarding requires authentication — an
/// unauthenticated remote request must be redirected to /login.
#[cfg(feature = "web-ui")]
#[tokio::test]
pub(super) async fn onboarding_requires_auth_after_setup() {
    let (addr, store, _state) = start_proxied_server().await;
    store.set_initial_password("testpass12345").await.unwrap();

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
pub(super) async fn onboarding_accessible_with_session_after_setup() {
    let (addr, store, _state) = start_proxied_server().await;
    store.set_initial_password("testpass12345").await.unwrap();
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

/// After auth is reset, `/onboarding` must stay reachable even if the
/// onboarding service still reports the instance as previously onboarded.
#[cfg(feature = "web-ui")]
#[tokio::test]
pub(super) async fn onboarding_remains_accessible_after_auth_reset_when_onboarded() {
    let (addr, store, _state) = start_server_with_onboarding(true, true).await;
    store.set_initial_password("testpass12345").await.unwrap();
    store.reset_all().await.unwrap();

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let resp = client
        .get(format!("http://{addr}/onboarding"))
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        200,
        "auth-reset instances must render /onboarding instead of redirecting away"
    );

    let body = resp.text().await.unwrap();
    assert!(
        body.contains("id=\"onboardingRoot\""),
        "/onboarding should render the onboarding shell after auth reset"
    );
}

/// POST /api/auth/setup is rejected with 403 after setup is already complete.
/// This prevents an attacker from resetting the password via the setup endpoint.
#[cfg(feature = "web-ui")]
#[tokio::test]
pub(super) async fn setup_endpoint_rejected_after_setup_complete() {
    let (addr, store, _state) = start_proxied_server().await;
    store.set_initial_password("testpass12345").await.unwrap();
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
pub(super) async fn authenticated_api_endpoint_not_rate_limited() {
    let (addr, store) = start_auth_server().await;
    store.set_initial_password("testpass12345").await.unwrap();
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

/// Setting a password via /api/auth/password/change on a localhost server with a
/// vault should initialize the vault and return a recovery key.
#[cfg(all(feature = "web-ui", feature = "vault"))]
#[tokio::test]
pub(super) async fn password_change_initializes_vault() {
    let (addr, store, _state, vault) = start_localhost_server_with_vault().await;

    // Vault starts uninitialized.
    assert_eq!(
        vault.status().await.unwrap(),
        moltis_vault::VaultStatus::Uninitialized
    );

    // Set password via the change endpoint (no current password — first time).
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/api/auth/password/change"))
        .header("Content-Type", "application/json")
        .body(r#"{"new_password":"newpass12345678"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], true);

    // Should have received a recovery key.
    assert!(
        body["recovery_key"].is_string(),
        "response should include a recovery_key after vault initialization"
    );
    let rk = body["recovery_key"].as_str().unwrap();
    assert!(!rk.is_empty());

    // Vault should now be unsealed.
    assert_eq!(
        vault.status().await.unwrap(),
        moltis_vault::VaultStatus::Unsealed
    );

    // Password should be set.
    assert!(store.has_password().await.unwrap());
    assert!(store.verify_password("newpass12345678").await.unwrap());
}

/// Setting a password via /api/auth/password/change when the vault is already
/// initialized should not return a recovery key (no double-init).
#[cfg(all(feature = "web-ui", feature = "vault"))]
#[tokio::test]
pub(super) async fn password_change_on_initialized_vault_no_recovery_key() {
    let (addr, store, _state, vault) = start_localhost_server_with_vault().await;

    // Pre-initialize the vault to simulate a previous setup.
    let _rk = vault.initialize("oldpass123").await.unwrap();
    assert_eq!(
        vault.status().await.unwrap(),
        moltis_vault::VaultStatus::Unsealed
    );

    // Set a password (first credential store password, but vault already initialized).
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/api/auth/password/change"))
        .header("Content-Type", "application/json")
        .body(r#"{"new_password":"newpass12345678"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], true);

    // No recovery key should be returned since vault was already initialized.
    assert!(
        body.get("recovery_key").is_none() || body["recovery_key"].is_null(),
        "should not return recovery_key for an already-initialized vault"
    );

    assert!(store.has_password().await.unwrap());
}

/// Bootstrap remains available when the vault is sealed because it does not
/// serve vault-encrypted session history.
#[cfg(all(feature = "web-ui", feature = "vault"))]
#[tokio::test]
pub(super) async fn sealed_vault_allows_bootstrap() {
    let (addr, _store, _state, vault) = start_localhost_server_with_vault().await;
    let _rk = vault.initialize("testpass12345").await.unwrap();
    vault.seal().await;

    let blocked_resp = reqwest::get(format!("http://{addr}/api/skills"))
        .await
        .unwrap();
    assert_eq!(blocked_resp.status(), 423);

    let resp = reqwest::get(format!(
        "http://{addr}/api/bootstrap?include_sessions=false"
    ))
    .await
    .unwrap();
    assert_eq!(resp.status(), 200);
}

/// Session history remains available when the vault is sealed because session
/// JSONL files are not yet encrypted by the vault.
#[cfg(all(feature = "web-ui", feature = "vault"))]
#[tokio::test]
pub(super) async fn sealed_vault_allows_session_history() {
    let (addr, _store, _state, vault, session_store) =
        start_localhost_server_with_vault_and_session_store().await;
    session_store
        .append(
            "main",
            &serde_json::json!({"role": "user", "content": "hello"}),
        )
        .await
        .unwrap();
    let _rk = vault.initialize("testpass12345").await.unwrap();
    vault.seal().await;

    let blocked_resp = reqwest::get(format!("http://{addr}/api/skills"))
        .await
        .unwrap();
    assert_eq!(blocked_resp.status(), 423);

    let resp = reqwest::get(format!("http://{addr}/api/sessions/main/history"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["history"][0]["content"], "hello");
}

// ── Onboarding auth bypass tests ────────────────────────────────────────────

/// Mock onboarding service with controllable `onboarded` flag.
pub(super) struct MockOnboardingService {
    onboarded: AtomicBool,
}

#[async_trait]
impl OnboardingService for MockOnboardingService {
    async fn wizard_start(&self, _p: serde_json::Value) -> ServiceResult {
        Ok(serde_json::json!({ "step": 0 }))
    }

    async fn wizard_next(&self, _p: serde_json::Value) -> ServiceResult {
        Ok(serde_json::json!({ "step": 0, "done": true }))
    }

    async fn wizard_cancel(&self) -> ServiceResult {
        Ok(serde_json::json!({}))
    }

    async fn wizard_status(&self) -> ServiceResult {
        let onboarded = self.onboarded.load(Ordering::Relaxed);
        Ok(serde_json::json!({ "active": !onboarded, "onboarded": onboarded }))
    }

    async fn identity_get(&self) -> ServiceResult {
        Ok(serde_json::json!({ "name": "moltis", "avatar": null }))
    }

    async fn identity_update(&self, _params: serde_json::Value) -> ServiceResult {
        Err("not configured".into())
    }

    async fn identity_update_soul(&self, _soul: Option<String>) -> ServiceResult {
        Err("not configured".into())
    }

    async fn openclaw_detect(&self) -> ServiceResult {
        Ok(serde_json::json!({ "found": false }))
    }

    async fn openclaw_scan(&self) -> ServiceResult {
        Ok(serde_json::json!({ "conversations": [] }))
    }

    async fn openclaw_import(&self, _params: serde_json::Value) -> ServiceResult {
        Err("not configured".into())
    }
}

/// Start a test server with a mock onboarding service.
///
/// When `behind_proxy` is true, connections are treated as remote.
#[cfg(feature = "web-ui")]
pub(super) async fn start_server_with_onboarding(
    onboarded: bool,
    behind_proxy: bool,
) -> (SocketAddr, Arc<CredentialStore>, Arc<GatewayState>) {
    let tmp = tempfile::tempdir().unwrap();
    moltis_config::set_config_dir(tmp.path().to_path_buf());
    moltis_config::set_data_dir(tmp.path().to_path_buf());
    std::mem::forget(tmp);

    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    let auth_config = moltis_config::AuthConfig::default();
    let cred_store = Arc::new(
        CredentialStore::with_config(pool, &auth_config)
            .await
            .unwrap(),
    );

    let mock_onboarding: Arc<dyn OnboardingService> = Arc::new(MockOnboardingService {
        onboarded: AtomicBool::new(onboarded),
    });

    let resolved_auth = auth::resolve_auth(None, None);
    let services = GatewayServices::noop().with_onboarding(mock_onboarding);
    let state = GatewayState::with_options(
        resolved_auth,
        services,
        moltis_config::MoltisConfig::default(),
        None,
        Some(Arc::clone(&cred_store)),
        None, // pairing_store
        false,
        behind_proxy,
        false,
        None,
        None,
        18789,
        false,
        None,
        None, // session_event_bus
        #[cfg(feature = "metrics")]
        None,
        #[cfg(feature = "metrics")]
        None,
        #[cfg(feature = "vault")]
        None,
    );
    let state_clone = Arc::clone(&state);
    let methods = Arc::new(MethodRegistry::new());
    #[cfg(feature = "push-notifications")]
    let (router, app_state) = build_gateway_base(state, methods, None, None);
    #[cfg(not(feature = "push-notifications"))]
    let (router, app_state) = build_gateway_base(state, methods, None);

    let router = router.merge(moltis_web::web_routes());
    let app = finalize_gateway_app(router, app_state, false);

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

/// During onboarding (password set but onboarded=false), a local API request
/// bypasses auth and succeeds. This is the STT test button scenario.
#[cfg(feature = "web-ui")]
#[tokio::test]
pub(super) async fn local_api_during_onboarding_bypasses_auth() {
    let (addr, store, _state) = start_server_with_onboarding(false, false).await;
    store.set_initial_password("testpass12345").await.unwrap();

    let resp = reqwest::get(format!("http://{addr}/api/bootstrap"))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "local API request during onboarding should bypass auth"
    );
}

/// After onboarding completes (onboarded=true), a local API request without
/// credentials must return 401 — the bypass is no longer active.
#[cfg(feature = "web-ui")]
#[tokio::test]
pub(super) async fn local_api_after_onboarding_requires_auth() {
    let (addr, store, _state) = start_server_with_onboarding(true, false).await;
    store.set_initial_password("testpass12345").await.unwrap();

    let resp = reqwest::get(format!("http://{addr}/api/bootstrap"))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        401,
        "local API request after onboarding must require auth"
    );
}

/// Remote API requests during onboarding must still require auth — the
/// bypass only applies to local connections.
#[cfg(feature = "web-ui")]
#[tokio::test]
pub(super) async fn remote_api_during_onboarding_requires_auth() {
    let (addr, store, _state) = start_server_with_onboarding(false, true).await;
    store.set_initial_password("testpass12345").await.unwrap();

    let resp = reqwest::get(format!("http://{addr}/api/bootstrap"))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        401,
        "remote API request during onboarding must still require auth"
    );
}

/// Privileged endpoints are NOT covered by the onboarding bypass, even for
/// local connections during onboarding. Only the narrow set of paths needed
/// by the wizard is allowed through.
#[cfg(feature = "web-ui")]
#[tokio::test]
pub(super) async fn local_privileged_api_during_onboarding_requires_auth() {
    let (addr, store, _state) = start_server_with_onboarding(false, false).await;
    store.set_initial_password("testpass12345").await.unwrap();

    // /api/config is not in the onboarding bypass allowlist.
    let resp = reqwest::get(format!("http://{addr}/api/config"))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        401,
        "privileged API must require auth even during onboarding"
    );
}
