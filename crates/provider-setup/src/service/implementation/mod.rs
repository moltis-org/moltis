//! `LiveProviderSetupService` — the runtime implementation of
//! `ProviderSetupService` that manages provider credentials, OAuth flows,
//! key validation, and provider registry rebuilds.

#[path = "../support.rs"]
mod support;

mod available;
mod credentials;
mod custom;
mod oauth;
mod validate;

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use secrecy::ExposeSecret;

use {
    async_trait::async_trait,
    serde_json::{Map, Value},
    tokio::sync::{OnceCell, RwLock},
    tracing::{info, warn},
};

use {
    moltis_config::schema::ProvidersConfig,
    moltis_providers::ProviderRegistry,
    moltis_service_traits::{ProviderSetupService, ServiceResult},
};

pub use self::support::ErrorParser;
use {
    self::support::{PendingOAuthFlow, default_error_parser},
    crate::{
        SetupBroadcaster,
        config_helpers::{
            config_with_saved_keys, env_value_with_overrides, home_key_store, home_provider_config,
            home_token_store,
        },
        key_store::KeyStore,
        known_providers::{AuthType, KnownProvider},
        oauth::has_oauth_tokens,
    },
};

// ── LiveProviderSetupService ───────────────────────────────────────────────

pub struct LiveProviderSetupService {
    registry: Arc<RwLock<ProviderRegistry>>,
    config: Arc<Mutex<ProvidersConfig>>,
    broadcaster: Arc<OnceCell<Arc<dyn SetupBroadcaster>>>,
    token_store: TokenStore,
    pub(crate) key_store: KeyStore,
    pending_oauth: Arc<RwLock<HashMap<String, PendingOAuthFlow>>>,
    /// When set, local-only providers (local-llm, ollama) are hidden from
    /// the available list because they cannot run on cloud VMs.
    deploy_platform: Option<String>,
    /// Shared priority models list from `LiveModelService`. Updated by
    /// `save_model` so the dropdown ordering reflects the latest preference.
    priority_models: Option<Arc<RwLock<Vec<String>>>>,
    /// Monotonic sequence used to drop stale async registry refreshes.
    registry_rebuild_seq: Arc<AtomicU64>,
    /// Static env overrides (for example config `[env]`) used when resolving
    /// provider credentials without mutating the process environment.
    env_overrides: HashMap<String, String>,
    /// Injected error parser for interpreting provider API errors.
    error_parser: ErrorParser,
    /// Address the OAuth callback server binds to. Defaults to `127.0.0.1`
    /// for local development; set to `0.0.0.0` in Docker / remote
    /// deployments so the callback port is reachable from the host.
    callback_bind_addr: String,
}

impl LiveProviderSetupService {
    pub fn new(
        registry: Arc<RwLock<ProviderRegistry>>,
        config: ProvidersConfig,
        deploy_platform: Option<String>,
    ) -> Self {
        Self {
            registry,
            config: Arc::new(Mutex::new(config)),
            broadcaster: Arc::new(OnceCell::new()),
            token_store: TokenStore::new(),
            key_store: KeyStore::new(),
            pending_oauth: Arc::new(RwLock::new(HashMap::new())),
            deploy_platform,
            priority_models: None,
            registry_rebuild_seq: Arc::new(AtomicU64::new(0)),
            env_overrides: HashMap::new(),
            error_parser: default_error_parser,
            callback_bind_addr: "127.0.0.1".to_string(),
        }
    }

    pub fn with_env_overrides(mut self, env_overrides: HashMap<String, String>) -> Self {
        self.env_overrides = env_overrides;
        self
    }

    /// Set a custom error parser for interpreting provider API errors.
    pub fn with_error_parser(mut self, parser: ErrorParser) -> Self {
        self.error_parser = parser;
        self
    }

    /// Set the bind address for the OAuth callback server.
    ///
    /// Defaults to `127.0.0.1`. Pass `0.0.0.0` when the gateway is
    /// bound to all interfaces (e.g. Docker) so the OAuth callback port
    /// is reachable from the host.
    pub fn with_callback_bind_addr(mut self, addr: String) -> Self {
        self.callback_bind_addr = addr;
        self
    }

    /// Wire the shared priority models handle from `LiveModelService` so
    /// `save_model` can update dropdown ordering at runtime.
    pub fn set_priority_models(&mut self, handle: Arc<RwLock<Vec<String>>>) {
        self.priority_models = Some(handle);
    }

    /// Set the broadcaster so validation can publish live progress events
    /// to the UI over WebSocket.
    pub fn set_broadcaster(&self, broadcaster: Arc<dyn SetupBroadcaster>) {
        let _ = self.broadcaster.set(broadcaster);
    }

    async fn emit_validation_progress(
        &self,
        provider: &str,
        request_id: Option<&str>,
        phase: &str,
        mut extra: Map<String, Value>,
    ) {
        let Some(broadcaster) = self.broadcaster.get() else {
            return;
        };

        let mut payload = Map::new();
        payload.insert("provider".to_string(), Value::String(provider.to_string()));
        payload.insert("phase".to_string(), Value::String(phase.to_string()));
        if let Some(id) = request_id {
            payload.insert("requestId".to_string(), Value::String(id.to_string()));
        }
        payload.append(&mut extra);

        broadcaster
            .broadcast("providers.validate.progress", Value::Object(payload))
            .await;
    }

    fn queue_registry_rebuild(&self, provider_name: &str, reason: &'static str) {
        let rebuild_seq = self.registry_rebuild_seq.fetch_add(1, Ordering::SeqCst) + 1;
        let latest_seq = Arc::clone(&self.registry_rebuild_seq);
        let registry = Arc::clone(&self.registry);
        let config = Arc::clone(&self.config);
        let key_store = self.key_store.clone();
        let env_overrides = self.env_overrides.clone();
        let provider_name = provider_name.to_string();

        tokio::spawn(async move {
            let started = std::time::Instant::now();
            info!(
                provider = %provider_name,
                reason,
                rebuild_seq,
                "provider registry async rebuild started"
            );

            let effective = {
                let base = config.lock().unwrap_or_else(|e| e.into_inner()).clone();
                config_with_saved_keys(&base, &key_store, &[])
            };

            let new_registry = match tokio::task::spawn_blocking(move || {
                ProviderRegistry::from_env_with_config_and_overrides(&effective, &env_overrides)
            })
            .await
            {
                Ok(registry) => registry,
                Err(error) => {
                    warn!(
                        provider = %provider_name,
                        reason,
                        rebuild_seq,
                        error = %error,
                        "provider registry async rebuild worker failed"
                    );
                    return;
                },
            };

            let current_seq = latest_seq.load(Ordering::Acquire);
            if rebuild_seq != current_seq {
                info!(
                    provider = %provider_name,
                    reason,
                    rebuild_seq,
                    latest_seq = current_seq,
                    elapsed_ms = started.elapsed().as_millis(),
                    "provider registry async rebuild skipped as stale"
                );
                return;
            }

            let provider_summary = new_registry.provider_summary();
            let model_count = new_registry.list_models().len();
            let mut reg = registry.write().await;
            *reg = new_registry;
            info!(
                provider = %provider_name,
                reason,
                rebuild_seq,
                provider_summary = %provider_summary,
                models = model_count,
                elapsed_ms = started.elapsed().as_millis(),
                "provider registry async rebuild finished"
            );
        });
    }

    fn config_snapshot(&self) -> ProvidersConfig {
        self.config
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn set_provider_enabled_in_memory(&self, provider: &str, enabled: bool) {
        let mut cfg = self.config.lock().unwrap_or_else(|e| e.into_inner());
        cfg.providers
            .entry(provider.to_string())
            .or_default()
            .enabled = enabled;
    }

    fn is_provider_configured(
        &self,
        provider: &KnownProvider,
        active_config: &ProvidersConfig,
    ) -> bool {
        // Disabled providers (by offered allowlist or explicit enabled=false)
        // should not show as configured, except subscription-backed OAuth
        // providers with valid local tokens.
        if !active_config.is_enabled(provider.name) {
            let subscription_with_tokens =
                matches!(provider.name, "openai-codex" | "github-copilot")
                    && active_config
                        .get(provider.name)
                        .is_none_or(|entry| entry.enabled)
                    && has_oauth_tokens(provider.name, &self.token_store);
            if !subscription_with_tokens {
                return false;
            }
        }

        // Check if the provider has an API key set via env
        if let Some(env_key) = provider.env_key
            && env_value_with_overrides(&self.env_overrides, env_key).is_some()
        {
            return true;
        }
        if provider.auth_type == AuthType::ApiKey
            && moltis_config::generic_provider_api_key_from_env(provider.name, &self.env_overrides)
                .is_some()
        {
            return true;
        }
        // Check config file
        if let Some(entry) = active_config.get(provider.name)
            && entry
                .api_key
                .as_ref()
                .is_some_and(|k| !k.expose_secret().is_empty())
        {
            return true;
        }
        // Check home/global config file as fallback when using custom config dir.
        if home_provider_config()
            .as_ref()
            .and_then(|(cfg, _)| cfg.get(provider.name))
            .and_then(|entry| entry.api_key.as_ref())
            .is_some_and(|k| !k.expose_secret().is_empty())
        {
            return true;
        }
        // Check persisted key store
        if self.key_store.load(provider.name).is_some() {
            return true;
        }
        // Check persisted key store in user-global config dir.
        if home_key_store()
            .as_ref()
            .is_some_and(|(store, _)| store.load(provider.name).is_some())
        {
            return true;
        }
        // For OAuth providers, check token store
        if provider.auth_type == AuthType::Oauth || provider.name == "kimi-code" {
            if self.token_store.load(provider.name).is_some() {
                return true;
            }
            if home_token_store()
                .as_ref()
                .is_some_and(|(store, _)| store.load(provider.name).is_some())
            {
                return true;
            }
            // Match provider-registry behavior: openai-codex may be inferred from
            // Codex CLI auth at ~/.codex/auth.json.
            if provider.name == "openai-codex"
                && crate::oauth::codex_cli_auth_path()
                    .as_deref()
                    .is_some_and(crate::oauth::codex_cli_auth_has_access_token)
            {
                return true;
            }
            return false;
        }
        // For local providers, check if model is configured in local_llm config
        #[cfg(feature = "local-llm")]
        if provider.auth_type == AuthType::Local && provider.name == "local-llm" {
            // Check if local-llm model config file exists
            if let Some(config_dir) = moltis_config::config_dir() {
                let config_path = config_dir.join("local-llm.json");
                return config_path.exists();
            }
        }
        false
    }

    /// Build a ProvidersConfig that includes saved keys for registry rebuild.
    fn effective_config(&self) -> ProvidersConfig {
        let base = self.config_snapshot();
        config_with_saved_keys(&base, &self.key_store, &[])
    }

    fn build_registry(&self, config: &ProvidersConfig) -> ProviderRegistry {
        ProviderRegistry::from_env_with_config_and_overrides(config, &self.env_overrides)
    }
}

#[async_trait]
impl ProviderSetupService for LiveProviderSetupService {
    async fn available(&self) -> ServiceResult {
        self.available_inner().await
    }

    async fn save_key(&self, params: Value) -> ServiceResult {
        self.save_key_inner(params).await
    }

    async fn oauth_start(&self, params: Value) -> ServiceResult {
        self.oauth_start_inner(params).await
    }

    async fn oauth_complete(&self, params: Value) -> ServiceResult {
        self.oauth_complete_inner(params).await
    }

    async fn remove_key(&self, params: Value) -> ServiceResult {
        self.remove_key_inner(params).await
    }

    async fn oauth_status(&self, params: Value) -> ServiceResult {
        self.oauth_status_inner(params).await
    }

    async fn validate_key(&self, params: Value) -> ServiceResult {
        self.validate_key_inner(params).await
    }

    async fn save_model(&self, params: Value) -> ServiceResult {
        self.save_model_inner(params).await
    }

    async fn save_models(&self, params: Value) -> ServiceResult {
        self.save_models_inner(params).await
    }

    async fn add_custom(&self, params: Value) -> ServiceResult {
        self.add_custom_inner(params).await
    }
}

use moltis_oauth::TokenStore;

// Re-export items needed by tests via `super::*`.
#[cfg(test)]
use {crate::known_providers::known_providers, secrecy::Secret};

#[cfg(test)]
#[path = "../tests.rs"]
mod tests;
