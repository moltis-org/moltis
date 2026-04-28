#[cfg(feature = "vault")]
use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};

#[cfg(any(feature = "vault", test))]
use super::AuthState;

#[cfg(test)]
use std::sync::Arc;

#[cfg(test)]
use super::{clear_session_response, localhost_cookie_domain, session_response};

#[cfg(test)]
use crate::login_guard::LoginGuard;

#[cfg(test)]
use moltis_gateway::{auth::CredentialStore, state::GatewayState};

// ── Vault handlers ──────────────────────────────────────────────────────────

#[cfg(feature = "vault")]
pub(super) async fn vault_status_handler(State(state): State<AuthState>) -> impl IntoResponse {
    let status = if let Some(ref vault) = state.gateway_state.vault {
        match vault.status().await {
            Ok(s) => format!("{s:?}").to_lowercase(),
            Err(_) => "error".to_owned(),
        }
    } else {
        "disabled".to_owned()
    };
    Json(serde_json::json!({ "status": status }))
}

#[cfg(feature = "vault")]
#[derive(serde::Deserialize)]
pub(super) struct VaultUnlockRequest {
    password: String,
}

#[cfg(feature = "vault")]
pub(super) async fn vault_unlock_handler(
    State(state): State<AuthState>,
    Json(body): Json<VaultUnlockRequest>,
) -> impl IntoResponse {
    let Some(ref vault) = state.gateway_state.vault else {
        return (StatusCode::NOT_FOUND, "vault not available").into_response();
    };
    match vault.unseal(&body.password).await {
        Ok(()) => {
            run_vault_env_migration(&state).await;
            start_stored_channels_on_vault_unseal(&state).await;
            Json(serde_json::json!({ "ok": true })).into_response()
        },
        Err(moltis_vault::VaultError::BadCredential) => {
            (StatusCode::LOCKED, "invalid password").into_response()
        },
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[cfg(feature = "vault")]
#[derive(serde::Deserialize)]
pub(super) struct VaultRecoveryRequest {
    recovery_key: String,
}

#[cfg(feature = "vault")]
pub(super) async fn vault_recovery_handler(
    State(state): State<AuthState>,
    Json(body): Json<VaultRecoveryRequest>,
) -> impl IntoResponse {
    let Some(ref vault) = state.gateway_state.vault else {
        return (StatusCode::NOT_FOUND, "vault not available").into_response();
    };
    match vault.unseal_with_recovery(&body.recovery_key).await {
        Ok(()) => {
            run_vault_env_migration(&state).await;
            start_stored_channels_on_vault_unseal(&state).await;
            Json(serde_json::json!({ "ok": true })).into_response()
        },
        Err(moltis_vault::VaultError::BadCredential) => {
            (StatusCode::LOCKED, "invalid recovery key").into_response()
        },
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Migrate plaintext secrets to encrypted storage after vault unseal.
#[cfg(feature = "vault")]
pub(super) async fn run_vault_env_migration(state: &AuthState) {
    if let Some(vault) = state.credential_store.vault() {
        let pool = state.credential_store.db_pool();
        match moltis_vault::migration::migrate_env_vars(vault, pool).await {
            Ok(n) if n > 0 => {
                tracing::info!(count = n, "migrated env vars to encrypted");
            },
            Ok(_) => {},
            Err(e) => {
                tracing::warn!(error = %e, "env var migration failed");
            },
        }
        match moltis_vault::migration::migrate_ssh_keys(vault, pool).await {
            Ok(n) if n > 0 => {
                tracing::info!(count = n, "migrated ssh keys to encrypted");
            },
            Ok(_) => {},
            Err(e) => {
                tracing::warn!(error = %e, "ssh key migration failed");
            },
        }

        // Migrate provider_keys.json (LLM + voice API keys) to encrypted storage.
        // This uses `encrypt_json_file` which creates a `.enc` alongside the
        // original instead of renaming it, so sync callers (KeyStore) can still
        // read the plaintext until KeyStore gains a vault-aware read path.
        if let Some(config_dir) = moltis_config::config_dir() {
            let provider_keys_path = config_dir.join("provider_keys.json");
            match moltis_vault::migration::encrypt_json_file(
                vault,
                &provider_keys_path,
                "provider_keys",
            )
            .await
            {
                Ok(true) => {
                    tracing::info!("encrypted provider_keys.json to vault storage");
                },
                Ok(false) => {},
                Err(e) => {
                    tracing::warn!(error = %e, "provider_keys.json encryption failed");
                },
            }
        }
    }
}

/// Start stored channel accounts after vault unseal.
///
/// When the vault is unsealed, previously encrypted channel configs become
/// decryptable. This function reads all stored channels and starts any that
/// aren't already running in the channel registry. This is the counterpart to
/// the startup-time channel loading in `server.rs` — it handles the case where
/// the vault was sealed at startup and channels couldn't be started then.
#[cfg(feature = "vault")]
#[tracing::instrument(skip(state))]
pub(super) async fn start_stored_channels_on_vault_unseal(state: &AuthState) {
    let Some(registry) = state.gateway_state.services.channel_registry.as_ref() else {
        tracing::debug!("no channel registry available, skipping channel startup on vault unseal");
        return;
    };
    let Some(store) = state.gateway_state.services.channel_store.as_ref() else {
        tracing::debug!("no channel store available, skipping channel startup on vault unseal");
        return;
    };

    let stored = match store.list().await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "failed to list stored channels on vault unseal");
            return;
        },
    };

    if stored.is_empty() {
        return;
    }

    for ch in stored {
        // Skip channel types with no registered plugin.
        if registry.get(&ch.channel_type).is_none() {
            tracing::debug!(
                account_id = ch.account_id,
                channel_type = ch.channel_type,
                "unsupported channel type on vault unseal, skipping stored account"
            );
            continue;
        }

        // Skip accounts that are already running.
        if registry.resolve_channel_type(&ch.account_id).is_some() {
            continue;
        }

        tracing::info!(
            account_id = ch.account_id,
            channel_type = ch.channel_type,
            "starting stored channel on vault unseal"
        );

        if let Err(e) = registry
            .start_account(&ch.channel_type, &ch.account_id, ch.config)
            .await
        {
            tracing::warn!(
                account_id = ch.account_id,
                channel_type = ch.channel_type,
                error = %e,
                "failed to start stored channel on vault unseal"
            );
        }
    }
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
        assert_eq!(localhost_cookie_domain(&h, false), "; Domain=localhost");
    }

    #[test]
    fn localhost_cookie_domain_moltis_subdomain() {
        let h = headers_with_host("moltis.localhost:59263");
        assert_eq!(localhost_cookie_domain(&h, false), "; Domain=localhost");
    }

    #[test]
    fn localhost_cookie_domain_bare_localhost_no_port() {
        let h = headers_with_host("localhost");
        assert_eq!(localhost_cookie_domain(&h, false), "; Domain=localhost");
    }

    #[test]
    fn localhost_cookie_domain_external_host_omits_domain() {
        let h = headers_with_host("example.com:443");
        assert_eq!(localhost_cookie_domain(&h, false), "");
    }

    #[test]
    fn localhost_cookie_domain_tailscale_host_omits_domain() {
        let h = headers_with_host("mybox.tail12345.ts.net:8080");
        assert_eq!(localhost_cookie_domain(&h, false), "");
    }

    #[test]
    fn localhost_cookie_domain_ip_address_omits_domain() {
        let h = headers_with_host("192.168.1.100:8080");
        assert_eq!(localhost_cookie_domain(&h, false), "");
    }

    #[test]
    fn localhost_cookie_domain_no_host_header_omits_domain() {
        let h = axum::http::HeaderMap::new();
        assert_eq!(localhost_cookie_domain(&h, false), "");
    }

    #[test]
    fn localhost_cookie_domain_proxy_mode_ignores_upstream_localhost_host() {
        let h = headers_with_host("localhost:13131");
        assert_eq!(localhost_cookie_domain(&h, true), "");
    }

    #[test]
    fn localhost_cookie_domain_proxy_mode_uses_forwarded_host() {
        let mut h = headers_with_host("localhost:13131");
        h.insert("x-forwarded-host", "chat.example.com".parse().unwrap());
        assert_eq!(localhost_cookie_domain(&h, true), "");
    }

    #[test]
    fn localhost_cookie_domain_proxy_mode_supports_forwarded_localhost_subdomain() {
        let mut h = headers_with_host("localhost:13131");
        h.insert("x-forwarded-host", "moltis.localhost:8080".parse().unwrap());
        assert_eq!(localhost_cookie_domain(&h, true), "; Domain=localhost");
    }

    #[test]
    fn session_response_includes_domain_for_localhost() {
        let h = headers_with_host("moltis.localhost:8080");
        let resp = session_response("test-token".into(), &h, false, false);
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
        assert!(
            !cookie.contains("; Secure"),
            "cookie should NOT include Secure when not using TLS, got: {cookie}"
        );
    }

    #[test]
    fn session_response_omits_domain_for_external_host() {
        let h = headers_with_host("example.com:443");
        let resp = session_response("test-token".into(), &h, false, false);
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
    fn session_response_includes_secure_when_tls_active() {
        let h = headers_with_host("localhost:8443");
        let resp = session_response("test-token".into(), &h, false, true);
        let cookie = resp
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .expect("login response must set a session cookie")
            .to_str()
            .expect("cookie header must be valid UTF-8");
        assert!(
            cookie.contains("; Secure"),
            "cookie should include Secure when TLS is active, got: {cookie}"
        );
    }

    #[test]
    fn clear_session_response_includes_secure_when_tls_active() {
        let h = headers_with_host("localhost:8443");
        let resp = clear_session_response(&h, false, true);
        let cookie = resp
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .expect("clear response must set a session cookie")
            .to_str()
            .expect("cookie header must be valid UTF-8");
        assert!(
            cookie.contains("; Secure"),
            "clear cookie should include Secure when TLS is active, got: {cookie}"
        );
        assert!(cookie.contains("Max-Age=0"));
    }

    #[test]
    fn clear_session_response_includes_domain_for_localhost() {
        let h = headers_with_host("localhost:18080");
        let resp = clear_session_response(&h, false, false);
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

    #[test]
    fn session_response_proxy_mode_omits_localhost_domain_without_forwarded_host() {
        let h = headers_with_host("localhost:13131");
        let resp = session_response("test-token".into(), &h, true, true);
        let cookie = resp
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .expect("login response must set a session cookie")
            .to_str()
            .expect("cookie header must be valid UTF-8");
        assert!(
            !cookie.contains("Domain="),
            "cookie should omit Domain in proxy mode when only upstream localhost host is visible, got: {cookie}"
        );
        assert!(
            cookie.contains("; Secure"),
            "cookie should include Secure in proxy mode (proxy implies TLS), got: {cookie}"
        );
    }
}

#[cfg(test)]
#[cfg(feature = "vault")]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod vault_unseal_tests {
    use {
        super::*,
        async_trait::async_trait,
        moltis_channels::{
            ChannelRegistry,
            config_view::ChannelConfigView,
            plugin::{ChannelOutbound, ChannelPlugin, ChannelStreamOutbound, StreamEvent},
            store::{ChannelStore, StoredChannel},
        },
    };

    use {
        moltis_channels::plugin::ChannelStatus, moltis_common::types::ReplyPayload,
        std::sync::Mutex, tokio::sync::RwLock,
    };

    /// Helper to build a minimal AuthState with the given services.
    async fn build_auth_state(services: moltis_gateway::services::GatewayServices) -> AuthState {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let auth_config = moltis_config::AuthConfig::default();
        let cred_store = Arc::new(
            CredentialStore::with_config(pool, &auth_config)
                .await
                .unwrap(),
        );
        let gateway_state =
            GatewayState::new(moltis_gateway::auth::resolve_auth(None, None), services);
        AuthState {
            credential_store: cred_store,
            webauthn_registry: None,
            gateway_state,
            login_guard: LoginGuard::new(),
        }
    }

    // ── Mock implementations ─────────────────────────────────────────────

    struct MockChannelStore {
        channels: Vec<StoredChannel>,
        should_fail: bool,
    }

    impl MockChannelStore {
        fn new(channels: Vec<StoredChannel>) -> Self {
            Self {
                channels,
                should_fail: false,
            }
        }

        fn failing() -> Self {
            Self {
                channels: vec![],
                should_fail: true,
            }
        }
    }

    #[async_trait]
    impl ChannelStore for MockChannelStore {
        async fn list(&self) -> moltis_channels::Result<Vec<StoredChannel>> {
            if self.should_fail {
                return Err(moltis_channels::Error::unavailable("store error"));
            }
            Ok(self.channels.clone())
        }

        async fn get(
            &self,
            _channel_type: &str,
            _account_id: &str,
        ) -> moltis_channels::Result<Option<StoredChannel>> {
            Ok(None)
        }

        async fn upsert(&self, _channel: StoredChannel) -> moltis_channels::Result<()> {
            Ok(())
        }

        async fn delete(
            &self,
            _channel_type: &str,
            _account_id: &str,
        ) -> moltis_channels::Result<()> {
            Ok(())
        }
    }

    /// Null outbound that does nothing.
    struct NullOutbound;

    #[async_trait]
    impl ChannelOutbound for NullOutbound {
        async fn send_text(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: Option<&str>,
        ) -> moltis_channels::Result<()> {
            Ok(())
        }

        async fn send_media(
            &self,
            _: &str,
            _: &str,
            _: &ReplyPayload,
            _: Option<&str>,
        ) -> moltis_channels::Result<()> {
            Ok(())
        }
    }

    /// Null stream outbound.
    struct NullStreamOutbound;

    #[async_trait]
    impl ChannelStreamOutbound for NullStreamOutbound {
        async fn send_stream(
            &self,
            _: &str,
            _: &str,
            _: Option<&str>,
            mut stream: moltis_channels::plugin::StreamReceiver,
        ) -> moltis_channels::Result<()> {
            while let Some(event) = stream.recv().await {
                if matches!(event, StreamEvent::Done | StreamEvent::Error(_)) {
                    break;
                }
            }
            Ok(())
        }
    }

    /// Test plugin that records start_account calls.
    struct RecordingPlugin {
        id: String,
        started_accounts: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
        should_fail_start: bool,
    }

    impl RecordingPlugin {
        fn new(id: &str) -> Self {
            Self {
                id: id.to_string(),
                started_accounts: Arc::new(Mutex::new(Vec::new())),
                should_fail_start: false,
            }
        }

        fn failing(id: &str) -> Self {
            Self {
                id: id.to_string(),
                started_accounts: Arc::new(Mutex::new(Vec::new())),
                should_fail_start: true,
            }
        }
    }

    #[async_trait]
    impl ChannelPlugin for RecordingPlugin {
        fn id(&self) -> &str {
            &self.id
        }

        fn name(&self) -> &str {
            &self.id
        }

        async fn start_account(
            &mut self,
            account_id: &str,
            config: serde_json::Value,
        ) -> moltis_channels::Result<()> {
            if self.should_fail_start {
                return Err(moltis_channels::Error::unavailable("start failed"));
            }
            self.started_accounts
                .lock()
                .unwrap()
                .push((account_id.to_string(), config));
            Ok(())
        }

        async fn stop_account(&mut self, _account_id: &str) -> moltis_channels::Result<()> {
            Ok(())
        }

        fn outbound(&self) -> Option<&dyn ChannelOutbound> {
            None
        }

        fn status(&self) -> Option<&dyn ChannelStatus> {
            None
        }

        fn has_account(&self, _account_id: &str) -> bool {
            false
        }

        fn account_ids(&self) -> Vec<String> {
            Vec::new()
        }

        fn account_config(&self, _account_id: &str) -> Option<Box<dyn ChannelConfigView>> {
            None
        }

        fn update_account_config(
            &self,
            _account_id: &str,
            _config: serde_json::Value,
        ) -> moltis_channels::Result<()> {
            Ok(())
        }

        fn shared_outbound(&self) -> Arc<dyn ChannelOutbound> {
            Arc::new(NullOutbound)
        }

        fn shared_stream_outbound(&self) -> Arc<dyn ChannelStreamOutbound> {
            Arc::new(NullStreamOutbound)
        }
    }

    // ── Tests ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn returns_early_when_channel_registry_is_none() {
        let services = moltis_gateway::services::GatewayServices::noop();
        let state = build_auth_state(services).await;
        // noop() has no channel_registry — should return without panicking
        start_stored_channels_on_vault_unseal(&state).await;
    }

    #[tokio::test]
    async fn returns_early_when_channel_store_is_none() {
        let registry = Arc::new(ChannelRegistry::new());
        let services =
            moltis_gateway::services::GatewayServices::noop().with_channel_registry(registry);
        // No channel_store set — should return without panicking
        let state = build_auth_state(services).await;
        start_stored_channels_on_vault_unseal(&state).await;
    }

    #[tokio::test]
    async fn logs_warning_when_store_list_fails() {
        let registry = Arc::new(ChannelRegistry::new());
        let store: Arc<dyn ChannelStore> = Arc::new(MockChannelStore::failing());
        let services = moltis_gateway::services::GatewayServices::noop()
            .with_channel_registry(registry)
            .with_channel_store(store);
        let state = build_auth_state(services).await;
        // Should log a warning but not panic
        start_stored_channels_on_vault_unseal(&state).await;
    }

    #[tokio::test]
    async fn returns_when_store_is_empty() {
        let registry = Arc::new(ChannelRegistry::new());
        let store: Arc<dyn ChannelStore> = Arc::new(MockChannelStore::new(vec![]));
        let services = moltis_gateway::services::GatewayServices::noop()
            .with_channel_registry(registry)
            .with_channel_store(store);
        let state = build_auth_state(services).await;
        start_stored_channels_on_vault_unseal(&state).await;
    }

    #[tokio::test]
    async fn skips_channels_with_unsupported_type() {
        let registry = Arc::new(ChannelRegistry::new());
        let store: Arc<dyn ChannelStore> = Arc::new(MockChannelStore::new(vec![StoredChannel {
            account_id: "acct1".to_string(),
            channel_type: "unknown_type".to_string(),
            config: serde_json::json!({}),
            created_at: 0,
            updated_at: 0,
        }]));
        let services = moltis_gateway::services::GatewayServices::noop()
            .with_channel_registry(registry)
            .with_channel_store(store);
        let state = build_auth_state(services).await;
        // Should not panic — logs a warning about unsupported type
        start_stored_channels_on_vault_unseal(&state).await;
    }

    #[tokio::test]
    async fn skips_channels_already_running() {
        let started = Arc::new(Mutex::new(Vec::new()));
        let plugin = RecordingPlugin::new("telegram");
        let started_clone = Arc::clone(&started);
        let plugin_with_tracking = RecordingPluginWithTracking {
            inner: plugin,
            started_accounts: started_clone,
        };

        let mut registry = ChannelRegistry::new();
        registry
            .register(Arc::new(RwLock::new(plugin_with_tracking)))
            .await;

        // Pre-start an account so it's already running
        registry
            .start_account("telegram", "acct1", serde_json::json!({"token": "x"}))
            .await
            .unwrap();

        let registry = Arc::new(registry);
        let store: Arc<dyn ChannelStore> = Arc::new(MockChannelStore::new(vec![StoredChannel {
            account_id: "acct1".to_string(),
            channel_type: "telegram".to_string(),
            config: serde_json::json!({"token": "new"}),
            created_at: 0,
            updated_at: 0,
        }]));
        let services = moltis_gateway::services::GatewayServices::noop()
            .with_channel_registry(registry)
            .with_channel_store(store);
        let state = build_auth_state(services).await;

        start_stored_channels_on_vault_unseal(&state).await;

        // The account was already running, so start_account should NOT have been
        // called again (resolve_channel_type returned Some, so it was skipped).
        // Only the pre-start call should be recorded.
        let calls = started.lock().unwrap();
        assert_eq!(
            calls.len(),
            1,
            "should not re-start already running account"
        );
        assert_eq!(calls[0].0, "acct1");
    }

    #[tokio::test]
    async fn starts_channels_successfully() {
        let plugin = RecordingPlugin::new("telegram");
        let started_accounts = Arc::clone(&plugin.started_accounts);

        let mut registry = ChannelRegistry::new();
        registry.register(Arc::new(RwLock::new(plugin))).await;

        let registry = Arc::new(registry);
        let store: Arc<dyn ChannelStore> = Arc::new(MockChannelStore::new(vec![
            StoredChannel {
                account_id: "bot1".to_string(),
                channel_type: "telegram".to_string(),
                config: serde_json::json!({"token": "abc123"}),
                created_at: 1000,
                updated_at: 2000,
            },
            StoredChannel {
                account_id: "bot2".to_string(),
                channel_type: "telegram".to_string(),
                config: serde_json::json!({"token": "def456"}),
                created_at: 1001,
                updated_at: 2001,
            },
        ]));
        let services = moltis_gateway::services::GatewayServices::noop()
            .with_channel_registry(registry)
            .with_channel_store(store);
        let state = build_auth_state(services).await;

        start_stored_channels_on_vault_unseal(&state).await;

        let calls = started_accounts.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "bot1");
        assert_eq!(calls[1].0, "bot2");
        assert_eq!(calls[0].1["token"], "abc123");
        assert_eq!(calls[1].1["token"], "def456");
    }

    #[tokio::test]
    async fn logs_warning_and_continues_when_start_account_fails() {
        let plugin = RecordingPlugin::failing("telegram");
        let started_accounts = Arc::clone(&plugin.started_accounts);

        let mut registry = ChannelRegistry::new();
        registry.register(Arc::new(RwLock::new(plugin))).await;

        let registry = Arc::new(registry);
        let store: Arc<dyn ChannelStore> = Arc::new(MockChannelStore::new(vec![StoredChannel {
            account_id: "bot1".to_string(),
            channel_type: "telegram".to_string(),
            config: serde_json::json!({"token": "bad"}),
            created_at: 0,
            updated_at: 0,
        }]));
        let services = moltis_gateway::services::GatewayServices::noop()
            .with_channel_registry(registry)
            .with_channel_store(store);
        let state = build_auth_state(services).await;

        // Should not panic — logs a warning about failed start
        start_stored_channels_on_vault_unseal(&state).await;

        // The plugin should have attempted the start (and recorded it before failing)
        let calls = started_accounts.lock().unwrap();
        assert_eq!(
            calls.len(),
            0,
            "failing plugin should not record successful starts"
        );
    }

    #[tokio::test]
    async fn mixed_channels_skips_unsupported_and_starts_supported() {
        let plugin = RecordingPlugin::new("telegram");
        let started_accounts = Arc::clone(&plugin.started_accounts);

        let mut registry = ChannelRegistry::new();
        registry.register(Arc::new(RwLock::new(plugin))).await;

        let registry = Arc::new(registry);
        let store: Arc<dyn ChannelStore> = Arc::new(MockChannelStore::new(vec![
            StoredChannel {
                account_id: "unsupported_acct".to_string(),
                channel_type: "unsupported_type".to_string(),
                config: serde_json::json!({}),
                created_at: 0,
                updated_at: 0,
            },
            StoredChannel {
                account_id: "tg_bot".to_string(),
                channel_type: "telegram".to_string(),
                config: serde_json::json!({"token": "xyz"}),
                created_at: 0,
                updated_at: 0,
            },
        ]));
        let services = moltis_gateway::services::GatewayServices::noop()
            .with_channel_registry(registry)
            .with_channel_store(store);
        let state = build_auth_state(services).await;

        start_stored_channels_on_vault_unseal(&state).await;

        let calls = started_accounts.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "tg_bot");
    }

    // ── Helper wrapper for the "already running" test ───────────────────

    /// Wrapper that tracks start_account calls through the registry path
    /// (which calls the plugin directly).
    struct RecordingPluginWithTracking {
        inner: RecordingPlugin,
        started_accounts: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
    }

    #[async_trait]
    impl ChannelPlugin for RecordingPluginWithTracking {
        fn id(&self) -> &str {
            self.inner.id()
        }

        fn name(&self) -> &str {
            self.inner.name()
        }

        async fn start_account(
            &mut self,
            account_id: &str,
            config: serde_json::Value,
        ) -> moltis_channels::Result<()> {
            self.started_accounts
                .lock()
                .unwrap()
                .push((account_id.to_string(), config.clone()));
            self.inner.start_account(account_id, config).await
        }

        async fn stop_account(&mut self, account_id: &str) -> moltis_channels::Result<()> {
            self.inner.stop_account(account_id).await
        }

        fn outbound(&self) -> Option<&dyn ChannelOutbound> {
            self.inner.outbound()
        }

        fn status(&self) -> Option<&dyn ChannelStatus> {
            self.inner.status()
        }

        fn has_account(&self, account_id: &str) -> bool {
            self.inner.has_account(account_id)
        }

        fn account_ids(&self) -> Vec<String> {
            self.inner.account_ids()
        }

        fn account_config(&self, account_id: &str) -> Option<Box<dyn ChannelConfigView>> {
            self.inner.account_config(account_id)
        }

        fn update_account_config(
            &self,
            account_id: &str,
            config: serde_json::Value,
        ) -> moltis_channels::Result<()> {
            self.inner.update_account_config(account_id, config)
        }

        fn shared_outbound(&self) -> Arc<dyn ChannelOutbound> {
            self.inner.shared_outbound()
        }

        fn shared_stream_outbound(&self) -> Arc<dyn ChannelStreamOutbound> {
            self.inner.shared_stream_outbound()
        }
    }
}
