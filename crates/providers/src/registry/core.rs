#![allow(unused_imports)]

//! Provider registry: model registration, lookup, discovery, and lifecycle.

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    pin::Pin,
    sync::Arc,
};

use {
    moltis_config::schema::{ProviderStreamTransport, ProvidersConfig},
    secrecy::ExposeSecret,
    tokio_stream::Stream,
};

use moltis_agents::model::{ChatMessage, LlmProvider, StreamEvent};

#[allow(unused_imports)]
use crate::{
    anthropic,
    config_helpers::{
        configured_models_for_provider, env_value, normalize_unique_models,
        oauth_discovery_enabled, resolve_api_key, should_fetch_models,
        subscription_preference_rank,
    },
    discovered_model::{
        DiscoveredModel, catalog_to_discovered, merge_discovered_with_fallback_catalog,
        merge_preferred_and_discovered_models,
    },
    model_capabilities::{extract_cw_overrides, ModelCapabilities, ModelInfo},
    model_catalogs::{ANTHROPIC_MODELS, OPENAI_COMPAT_PROVIDERS},
    model_id::{
        REASONING_SUFFIX_SEP, REASONING_SUFFIXES, namespaced_model_id, raw_model_id,
        split_reasoning_suffix,
    },
    ollama::{
        self, OllamaShowResponse, probe_ollama_models_batch, probe_ollama_models_batch_async,
        resolve_ollama_tool_mode,
    },
    openai,
};

struct RegistryModelProvider {
    model_id: String,
    inner: Arc<dyn LlmProvider>,
}

#[async_trait::async_trait]
impl LlmProvider for RegistryModelProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn id(&self) -> &str {
        &self.model_id
    }

    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> anyhow::Result<moltis_agents::model::CompletionResponse> {
        self.inner.complete(messages, tools).await
    }

    fn supports_tools(&self) -> bool {
        self.inner.supports_tools()
    }

    fn tool_mode(&self) -> Option<moltis_config::ToolMode> {
        self.inner.tool_mode()
    }

    fn context_window(&self) -> u32 {
        self.inner.context_window()
    }

    fn supports_vision(&self) -> bool {
        self.inner.supports_vision()
    }

    fn stream(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        self.inner.stream(messages)
    }

    fn stream_with_tools(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<serde_json::Value>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        self.inner.stream_with_tools(messages, tools)
    }

    fn reasoning_effort(&self) -> Option<moltis_agents::model::ReasoningEffort> {
        self.inner.reasoning_effort()
    }

    fn with_reasoning_effort(
        self: Arc<Self>,
        effort: moltis_agents::model::ReasoningEffort,
    ) -> Option<Arc<dyn LlmProvider>> {
        let new_inner = Arc::clone(&self.inner).with_reasoning_effort(effort)?;
        Some(Arc::new(RegistryModelProvider {
            model_id: self.model_id.clone(),
            inner: new_inner,
        }))
    }
}

fn anthropic_fallback_catalog() -> Vec<DiscoveredModel> {
    catalog_to_discovered(ANTHROPIC_MODELS, 3)
}

/// Result of a runtime model rediscovery pass.
///
/// Bundles the discovered model lists together with any Ollama `/api/show`
/// probe results so that registration can proceed without further I/O.
/// Ollama probe data is opaque — callers pass this struct directly to
/// [`ProviderRegistry::register_rediscovered_models`].
pub struct RediscoveryResult {
    /// Models discovered per provider (keyed by config name).
    pub(crate) models: HashMap<String, Vec<DiscoveredModel>>,
    /// Ollama `/api/show` probe metadata (keyed by model ID).
    pub(crate) ollama_probes: HashMap<String, OllamaShowResponse>,
}

impl RediscoveryResult {
    /// Returns `true` when no models were discovered across all providers.
    pub fn is_empty(&self) -> bool {
        self.models.values().all(|v| v.is_empty())
    }
}

/// Asynchronously fetch models from all discoverable provider APIs.
///
/// Runs `/v1/models` (or Ollama `/api/tags`) for each eligible provider
/// concurrently, then batch-probes any discovered Ollama models via
/// `/api/show` for tool-mode metadata. Returns everything needed for
/// lock-free registration.
///
/// `provider_filter` narrows the scope to a single provider name (case-
/// insensitive comparison against config name or alias).
pub async fn fetch_discoverable_models(
    config: &ProvidersConfig,
    env_overrides: &HashMap<String, String>,
    provider_filter: Option<&str>,
) -> RediscoveryResult {
    use futures::future::join_all;

    let filter_matches =
        |name: &str| -> bool { provider_filter.is_none_or(|f| f.eq_ignore_ascii_case(name)) };

    let mut tasks: Vec<(
        String,
        Pin<Box<dyn Future<Output = anyhow::Result<Vec<DiscoveredModel>>> + Send>>,
    )> = Vec::new();

    // ── OpenAI builtin ────────────────────────────────────────────────
    if filter_matches("openai")
        && config.is_enabled("openai")
        && !cfg!(test)
        && let Some(key) = resolve_api_key(config, "openai", "OPENAI_API_KEY", env_overrides)
        && should_fetch_models(config, "openai")
    {
        let base_url = config
            .get("openai")
            .and_then(|e| e.base_url.clone())
            .or_else(|| env_value(env_overrides, "OPENAI_BASE_URL"))
            .unwrap_or_else(|| "https://api.openai.com/v1".into());
        tasks.push((
            "openai".into(),
            Box::pin(openai::fetch_models_from_api(key, base_url)),
        ));
    }

    // ── Anthropic builtin ─────────────────────────────────────────────
    if filter_matches("anthropic")
        && config.is_enabled("anthropic")
        && !cfg!(test)
        && let Some(key) = resolve_api_key(config, "anthropic", "ANTHROPIC_API_KEY", env_overrides)
        && should_fetch_models(config, "anthropic")
    {
        let base_url = config
            .get("anthropic")
            .and_then(|e| e.base_url.clone())
            .or_else(|| env_value(env_overrides, "ANTHROPIC_BASE_URL"))
            .unwrap_or_else(|| "https://api.anthropic.com".into());
        tasks.push((
            "anthropic".into(),
            Box::pin(anthropic::fetch_models_from_api(key, base_url)),
        ));
    }

    // ── OpenAI-compatible providers ───────────────────────────────────
    for def in OPENAI_COMPAT_PROVIDERS {
        if !filter_matches(def.config_name) || !config.is_enabled(def.config_name) {
            continue;
        }

        let key = resolve_api_key(config, def.config_name, def.env_key, env_overrides);
        let key = if !def.requires_api_key {
            key.or_else(|| Some(secrecy::Secret::new(def.config_name.into())))
        } else if def.config_name == "gemini" {
            key.or_else(|| env_value(env_overrides, "GOOGLE_API_KEY").map(secrecy::Secret::new))
        } else {
            key
        };
        let Some(key) = key else {
            continue;
        };

        let base_url = config
            .get(def.config_name)
            .and_then(|e| e.base_url.clone())
            .or_else(|| env_value(env_overrides, def.env_base_url_key))
            .unwrap_or_else(|| def.default_base_url.into());

        if def.local_only {
            let has_explicit_entry = config.get(def.config_name).is_some();
            let has_env_base_url = env_value(env_overrides, def.env_base_url_key).is_some();
            let preferred = configured_models_for_provider(config, def.config_name);
            if !has_explicit_entry && !has_env_base_url && preferred.is_empty() {
                continue;
            }
        }

        let user_opted_in = config
            .get(def.config_name)
            .is_some_and(|entry| entry.fetch_models);
        let try_fetch = def.supports_model_discovery || user_opted_in;
        if !try_fetch || !should_fetch_models(config, def.config_name) {
            continue;
        }

        if def.config_name == "ollama" {
            tasks.push((
                def.config_name.into(),
                Box::pin(ollama::discover_ollama_models_from_api(base_url)),
            ));
        } else {
            tasks.push((
                def.config_name.into(),
                Box::pin(openai::fetch_models_from_api(key, base_url)),
            ));
        }
    }

    // ── Custom providers ──────────────────────────────────────────────
    for (name, entry) in &config.providers {
        if !name.starts_with("custom-") || !entry.enabled {
            continue;
        }
        if !filter_matches(name) {
            continue;
        }
        let Some(api_key) = entry
            .api_key
            .as_ref()
            .filter(|k| !k.expose_secret().is_empty())
        else {
            continue;
        };
        let Some(base_url) = entry.base_url.as_ref().filter(|u| !u.trim().is_empty()) else {
            continue;
        };
        if should_fetch_models(config, name) {
            tasks.push((
                name.clone(),
                Box::pin(openai::fetch_models_from_api(
                    api_key.clone(),
                    base_url.clone(),
                )),
            ));
        }
    }

    // Run all fetches concurrently.
    let names: Vec<String> = tasks.iter().map(|(n, _)| n.clone()).collect();
    let futures: Vec<_> = tasks.into_iter().map(|(_, fut)| fut).collect();
    let results = join_all(futures).await;

    let mut map = HashMap::new();
    for (name, result) in names.into_iter().zip(results) {
        match result {
            Ok(models) => {
                tracing::debug!(
                    provider = %name,
                    model_count = models.len(),
                    "runtime model rediscovery succeeded"
                );
                map.insert(name, models);
            },
            Err(err) => {
                tracing::debug!(
                    provider = %name,
                    error = %err,
                    "runtime model rediscovery failed"
                );
            },
        }
    }

    // Batch-probe any newly discovered Ollama models for `/api/show` metadata
    // (tool capabilities, family info). Runs outside any registry lock.
    let ollama_probes = if let Some(ollama_models) = map.get("ollama") {
        let ollama_base_url = config
            .get("ollama")
            .and_then(|e| e.base_url.clone())
            .or_else(|| env_value(env_overrides, "OLLAMA_BASE_URL"))
            .unwrap_or_else(|| "http://localhost:11434".into());
        probe_ollama_models_batch_async(&ollama_base_url, ollama_models).await
    } else {
        HashMap::new()
    };

    RediscoveryResult {
        models: map,
        ollama_probes,
    }
}

#[cfg(any(feature = "provider-openai-codex", feature = "provider-github-copilot"))]
pub(crate) trait DynamicModelDiscovery {
    fn provider_name(&self) -> &'static str;
    fn is_enabled_and_authenticated(&self, config: &ProvidersConfig) -> bool;
    fn configured_models(&self, config: &ProvidersConfig) -> Vec<String>;
    fn should_fetch_models(&self, config: &ProvidersConfig) -> bool;
    fn live_models(&self) -> anyhow::Result<Vec<DiscoveredModel>>;
    fn build_provider(&self, model_id: String, config: &ProvidersConfig) -> Arc<dyn LlmProvider>;
    fn display_name(&self, model_id: &str, discovered: &str) -> String;
}

#[cfg(feature = "provider-openai-codex")]
pub(crate) struct OpenAiCodexDiscovery;

#[cfg(feature = "provider-openai-codex")]
impl DynamicModelDiscovery for OpenAiCodexDiscovery {
    fn provider_name(&self) -> &'static str {
        "openai-codex"
    }

    fn is_enabled_and_authenticated(&self, config: &ProvidersConfig) -> bool {
        use crate::openai_codex;
        oauth_discovery_enabled(config, self.provider_name()) && openai_codex::has_stored_tokens()
    }

    fn configured_models(&self, config: &ProvidersConfig) -> Vec<String> {
        configured_models_for_provider(config, self.provider_name())
    }

    fn should_fetch_models(&self, config: &ProvidersConfig) -> bool {
        should_fetch_models(config, self.provider_name())
    }

    fn live_models(&self) -> anyhow::Result<Vec<DiscoveredModel>> {
        use crate::openai_codex;
        openai_codex::live_models()
    }

    fn build_provider(&self, model_id: String, config: &ProvidersConfig) -> Arc<dyn LlmProvider> {
        use crate::openai_codex;
        let stream_transport = config
            .get(self.provider_name())
            .map(|entry| entry.stream_transport)
            .unwrap_or(ProviderStreamTransport::Sse);
        Arc::new(openai_codex::OpenAiCodexProvider::new_with_transport(
            model_id,
            stream_transport,
        ))
    }

    fn display_name(&self, _model_id: &str, discovered: &str) -> String {
        format!("{discovered} (Codex/OAuth)")
    }
}

#[cfg(feature = "provider-github-copilot")]
pub(crate) struct GitHubCopilotDiscovery;

#[cfg(feature = "provider-github-copilot")]
impl DynamicModelDiscovery for GitHubCopilotDiscovery {
    fn provider_name(&self) -> &'static str {
        "github-copilot"
    }

    fn is_enabled_and_authenticated(&self, config: &ProvidersConfig) -> bool {
        use crate::github_copilot;
        oauth_discovery_enabled(config, self.provider_name()) && github_copilot::has_stored_tokens()
    }

    fn configured_models(&self, config: &ProvidersConfig) -> Vec<String> {
        configured_models_for_provider(config, self.provider_name())
    }

    fn should_fetch_models(&self, config: &ProvidersConfig) -> bool {
        should_fetch_models(config, self.provider_name())
    }

    fn live_models(&self) -> anyhow::Result<Vec<DiscoveredModel>> {
        use crate::github_copilot;
        github_copilot::live_models()
    }

    fn build_provider(&self, model_id: String, _config: &ProvidersConfig) -> Arc<dyn LlmProvider> {
        use crate::github_copilot;
        Arc::new(github_copilot::GitHubCopilotProvider::new(model_id))
    }

    fn display_name(&self, _model_id: &str, discovered: &str) -> String {
        if discovered.to_ascii_lowercase().contains("copilot") {
            discovered.to_string()
        } else {
            format!("{discovered} (Copilot)")
        }
    }
}

/// Registry of available LLM providers, keyed by namespaced model ID.
pub struct ProviderRegistry {
    pub(crate) providers: HashMap<String, Arc<dyn LlmProvider>>,
    pub(crate) models: Vec<ModelInfo>,
    /// Global per-model context window overrides from `[models.<id>]` config.
    pub(crate) global_cw_overrides: HashMap<String, u32>,
}

/// Pending model discovery handles returned by [`ProviderRegistry::fire_discoveries`].
pub type PendingDiscoveries = Vec<(
    String,
    std::sync::mpsc::Receiver<anyhow::Result<Vec<DiscoveredModel>>>,
)>;

impl ProviderRegistry {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            providers: HashMap::new(),
            models: Vec::new(),
            global_cw_overrides: HashMap::new(),
        }
    }

    pub(crate) fn has_provider_model(&self, provider: &str, model_id: &str) -> bool {
        self.providers
            .contains_key(&namespaced_model_id(provider, model_id))
    }

    /// Check if the raw (un-namespaced) model ID is registered under any provider.
    pub(crate) fn has_model_any_provider(&self, model_id: &str) -> bool {
        let raw = raw_model_id(model_id);
        self.models.iter().any(|m| raw_model_id(&m.id) == raw)
    }

    pub(crate) fn resolve_registry_model_id(
        &self,
        model_id: &str,
        provider_hint: Option<&str>,
    ) -> Option<String> {
        if self.providers.contains_key(model_id) {
            return Some(model_id.to_string());
        }

        let raw = raw_model_id(model_id);
        self.models
            .iter()
            .enumerate()
            .filter(|(_, m)| raw_model_id(&m.id) == raw)
            .filter(|(_, m)| provider_hint.is_none_or(|hint| m.provider == hint))
            .min_by_key(|(idx, m)| (subscription_preference_rank(&m.provider), *idx))
            .map(|(_, m)| m.id.clone())
    }

    #[cfg(any(feature = "provider-openai-codex", feature = "provider-github-copilot"))]
    #[allow(clippy::vec_init_then_push)]
    pub(crate) fn dynamic_discovery_sources() -> Vec<Box<dyn DynamicModelDiscovery>> {
        let mut sources: Vec<Box<dyn DynamicModelDiscovery>> = Vec::new();
        #[cfg(feature = "provider-openai-codex")]
        sources.push(Box::new(OpenAiCodexDiscovery));
        #[cfg(feature = "provider-github-copilot")]
        sources.push(Box::new(GitHubCopilotDiscovery));
        sources
    }

    #[cfg(any(feature = "provider-openai-codex", feature = "provider-github-copilot"))]
    pub(crate) fn desired_models_for_dynamic_source(
        source: &dyn DynamicModelDiscovery,
        config: &ProvidersConfig,
        catalog: Vec<DiscoveredModel>,
    ) -> Option<Vec<DiscoveredModel>> {
        if !source.is_enabled_and_authenticated(config) {
            return None;
        }

        let preferred = source.configured_models(config);
        Some(merge_preferred_and_discovered_models(preferred, catalog))
    }

    #[cfg(any(feature = "provider-openai-codex", feature = "provider-github-copilot"))]
    pub(crate) fn register_dynamic_source_models(
        &mut self,
        source: &dyn DynamicModelDiscovery,
        config: &ProvidersConfig,
        catalog: Vec<DiscoveredModel>,
    ) {
        let Some(models) = Self::desired_models_for_dynamic_source(source, config, catalog) else {
            return;
        };

        for model in models {
            if self.has_provider_model(source.provider_name(), &model.id) {
                continue;
            }
            let provider = source.build_provider(model.id.clone(), config);
            self.register(
                ModelInfo {
                    id: model.id.clone(),
                    provider: source.provider_name().to_string(),
                    display_name: source.display_name(&model.id, &model.display_name),
                    created_at: model.created_at,
                    recommended: model.recommended,
                    capabilities: model
                        .capabilities
                        .unwrap_or_else(|| ModelCapabilities::infer(&model.id)),
                },
                provider,
            );
        }
    }

    #[cfg(any(feature = "provider-openai-codex", feature = "provider-github-copilot"))]
    pub(crate) fn refresh_dynamic_source_models(
        &mut self,
        source: &dyn DynamicModelDiscovery,
        config: &ProvidersConfig,
    ) -> bool {
        if !source.is_enabled_and_authenticated(config) {
            return false;
        }
        if !source.should_fetch_models(config) {
            return false;
        }

        let live_catalog = match source.live_models() {
            Ok(models) => models,
            Err(err) => {
                tracing::warn!(
                    provider = source.provider_name(),
                    error = %err,
                    "skipping dynamic model refresh because live fetch failed"
                );
                return false;
            },
        };

        let Some(next_models) =
            Self::desired_models_for_dynamic_source(source, config, live_catalog)
        else {
            return false;
        };

        let new_entries: Vec<(ModelInfo, Arc<dyn LlmProvider>)> = next_models
            .into_iter()
            .map(|model| {
                let caps = model
                    .capabilities
                    .unwrap_or_else(|| ModelCapabilities::infer(&model.id));
                (
                    ModelInfo {
                        id: model.id.clone(),
                        provider: source.provider_name().to_string(),
                        display_name: source.display_name(&model.id, &model.display_name),
                        created_at: model.created_at,
                        recommended: model.recommended,
                        capabilities: caps,
                    },
                    source.build_provider(model.id, config),
                )
            })
            .collect();

        // Replace stale provider entries atomically only after successful fetch.
        let stale_ids: Vec<String> = self
            .models
            .iter()
            .filter(|m| m.provider == source.provider_name())
            .map(|m| m.id.clone())
            .collect();
        for model_id in &stale_ids {
            self.providers.remove(model_id);
        }
        self.models.retain(|m| m.provider != source.provider_name());
        for (info, provider) in new_entries {
            self.register(info, provider);
        }

        true
    }

    pub(crate) fn desired_anthropic_models(
        config: &ProvidersConfig,
        prefetched: &HashMap<String, Vec<DiscoveredModel>>,
    ) -> Vec<DiscoveredModel> {
        let preferred = configured_models_for_provider(config, "anthropic");
        let discovered = if should_fetch_models(config, "anthropic") {
            match prefetched.get("anthropic") {
                Some(live) => live.clone(),
                None => anthropic_fallback_catalog(),
            }
        } else {
            Vec::new()
        };
        merge_preferred_and_discovered_models(preferred, discovered)
    }

    pub(crate) fn register_anthropic_catalog(
        &mut self,
        models: Vec<DiscoveredModel>,
        key: &secrecy::Secret<String>,
        base_url: &str,
        provider_label: &str,
        alias: Option<String>,
        cache_retention: moltis_config::CacheRetention,
        provider_cw_overrides: HashMap<String, u32>,
    ) -> usize {
        let mut added = 0usize;
        let global = self.global_cw_overrides.clone();

        for model in models {
            let caps = model
                .capabilities
                .unwrap_or_else(|| ModelCapabilities::infer(&model.id));
            let (model_id, display_name, created_at, recommended) = (
                model.id,
                model.display_name,
                model.created_at,
                model.recommended,
            );
            if self.has_provider_model(provider_label, &model_id) {
                continue;
            }
            let provider = Arc::new(
                anthropic::AnthropicProvider::with_alias(
                    key.clone(),
                    model_id.clone(),
                    base_url.to_string(),
                    alias.clone(),
                )
                .with_cache_retention(cache_retention)
                .with_context_window_overrides(global.clone(), provider_cw_overrides.clone()),
            );
            self.register(
                ModelInfo {
                    id: model_id,
                    provider: provider_label.to_string(),
                    display_name,
                    created_at,
                    recommended,
                    capabilities: caps,
                },
                provider,
            );
            added += 1;
        }

        added
    }

    pub(crate) fn replace_anthropic_catalog(
        &mut self,
        models: Vec<DiscoveredModel>,
        key: &secrecy::Secret<String>,
        base_url: &str,
        provider_label: &str,
        alias: Option<String>,
        cache_retention: moltis_config::CacheRetention,
        provider_cw_overrides: HashMap<String, u32>,
    ) -> usize {
        let global = self.global_cw_overrides.clone();
        let new_entries: Vec<(ModelInfo, Arc<dyn LlmProvider>)> = models
            .into_iter()
            .map(|model| {
                let caps = model
                    .capabilities
                    .unwrap_or_else(|| ModelCapabilities::infer(&model.id));
                let provider = Arc::new(
                    anthropic::AnthropicProvider::with_alias(
                        key.clone(),
                        model.id.clone(),
                        base_url.to_string(),
                        alias.clone(),
                    )
                    .with_cache_retention(cache_retention)
                .with_context_window_overrides(global.clone(), provider_cw_overrides.clone()),
                );
                (
                    ModelInfo {
                        id: model.id,
                        provider: provider_label.to_string(),
                        display_name: model.display_name,
                        created_at: model.created_at,
                        recommended: model.recommended,
                        capabilities: caps,
                    },
                    provider as Arc<dyn LlmProvider>,
                )
            })
            .collect();

        let previous_ids: HashSet<String> = self
            .models
            .iter()
            .filter(|m| m.provider == provider_label)
            .map(|m| m.id.clone())
            .collect();

        self.models.retain(|m| m.provider != provider_label);
        self.providers.retain(|id, _| !previous_ids.contains(id));

        let next_ids: HashSet<String> = new_entries
            .iter()
            .map(|(info, _)| namespaced_model_id(provider_label, raw_model_id(&info.id)))
            .collect();

        for (info, provider) in new_entries {
            self.register(info, provider);
        }

        next_ids.difference(&previous_ids).count()
    }

    /// Register a provider manually.
    pub fn register(&mut self, mut info: ModelInfo, provider: Arc<dyn LlmProvider>) {
        let model_id = raw_model_id(&info.id).to_string();
        let registry_model_id = namespaced_model_id(&info.provider, &model_id);
        info.id = registry_model_id.clone();
        let wrapped: Arc<dyn LlmProvider> = Arc::new(RegistryModelProvider {
            model_id: registry_model_id.clone(),
            inner: provider,
        });
        self.providers.insert(registry_model_id, wrapped);
        self.models.push(info);
    }

    /// Unregister a provider by model ID. Returns true if it was removed.
    pub fn unregister(&mut self, model_id: &str) -> bool {
        let resolved_id = self.resolve_registry_model_id(model_id, None);
        let removed = resolved_id
            .as_deref()
            .and_then(|id| self.providers.remove(id))
            .is_some();
        if removed && let Some(id) = resolved_id {
            self.models.retain(|m| m.id != id);
        }
        removed
    }

    /// Auto-discover providers from environment variables.
    /// Uses default config (all providers enabled).
    pub fn from_env() -> Self {
        Self::from_env_with_config(&ProvidersConfig::default(), HashMap::new())
    }

    /// Auto-discover providers from environment variables,
    /// respecting the given config for enable/disable and overrides.
    ///
    /// Provider registration order:
    /// 1. Built-in raw reqwest providers (always available, support tool calling)
    /// 2. async-openai-backed providers (if `provider-async-openai` feature enabled)
    /// 3. genai-backed providers (if `provider-genai` feature enabled, no tool support)
    /// 4. OpenAI Codex OAuth providers (if `provider-openai-codex` feature enabled)
    ///
    /// Model/provider auto-selection preference:
    /// 1. Subscription providers (`openai-codex`, `github-copilot`)
    /// 2. Everything else
    ///
    /// Within the same preference tier, registration order wins.
    pub fn from_env_with_config(
        config: &ProvidersConfig,
        global_cw_overrides: HashMap<String, u32>,
    ) -> Self {
        let env_overrides = HashMap::new();
        Self::from_env_with_config_and_overrides(config, &env_overrides, global_cw_overrides)
    }

    /// Auto-discover providers from config, process env, and optional env
    /// overrides. Process env always wins when both are present.
    ///
    /// Model discovery HTTP requests are fired concurrently in Phase 1,
    /// collected in Phase 2, and the results are used to register providers
    /// in Phase 3. This reduces startup time from `sum(latencies)` to
    /// `max(latencies)`.
    pub fn from_env_with_config_and_overrides(
        config: &ProvidersConfig,
        env_overrides: &HashMap<String, String>,
        global_cw_overrides: HashMap<String, u32>,
    ) -> Self {
        let pending = Self::fire_discoveries(config, env_overrides);
        let prefetched = Self::collect_discoveries(pending);
        Self::from_config_with_prefetched(config, env_overrides, &prefetched, global_cw_overrides)
    }

    /// Register providers without making any discovery HTTP requests.
    ///
    /// This uses static model catalogs plus any explicit/pinned models from
    /// config and env overrides.
    pub fn from_config_with_static_catalogs(
        config: &ProvidersConfig,
        env_overrides: &HashMap<String, String>,
        global_cw_overrides: HashMap<String, u32>,
    ) -> Self {
        let prefetched = HashMap::new();
        Self::from_config_with_prefetched(config, env_overrides, &prefetched, global_cw_overrides)
    }

    /// Register providers using already-collected discovery results.
    ///
    /// `prefetched` should come from [`collect_discoveries`], but callers may
    /// also pass an empty map to register only static catalogs.
    pub fn from_config_with_prefetched(
        config: &ProvidersConfig,
        env_overrides: &HashMap<String, String>,
        prefetched: &HashMap<String, Vec<DiscoveredModel>>,
        global_cw_overrides: HashMap<String, u32>,
    ) -> Self {
        let mut reg = Self::empty();
        reg.global_cw_overrides = global_cw_overrides;

        // Built-in providers first: they support tool calling.
        reg.register_builtin_providers(config, env_overrides, prefetched);
        reg.register_openai_compatible_providers(config, env_overrides, prefetched);
        reg.register_custom_providers(config, prefetched);

        #[cfg(feature = "provider-async-openai")]
        {
            reg.register_async_openai_providers(config, env_overrides);
        }

        // GenAI providers last: they don't support tool calling,
        // so they only fill in models not already covered above.
        #[cfg(feature = "provider-genai")]
        {
            reg.register_genai_providers(config, env_overrides);
        }

        #[cfg(feature = "provider-openai-codex")]
        {
            reg.register_openai_codex_providers(config, prefetched);
        }

        #[cfg(feature = "provider-github-copilot")]
        {
            reg.register_github_copilot_providers(config, prefetched);
        }

        #[cfg(feature = "provider-kimi-code")]
        {
            reg.register_kimi_code_providers(config, env_overrides);
        }

        // Local GGUF providers (no API key needed, model runs locally)
        #[cfg(feature = "local-llm")]
        {
            reg.register_local_gguf_providers(config);
        }

        reg
    }

    /// Fire all provider model discovery HTTP requests concurrently.
    ///
    /// Returns a vec of `(provider_key, Receiver)` handles. Each receiver
    /// will eventually yield the discovered model list. Call
    /// [`collect_discoveries`] to drain them (blocking).
    #[allow(unused_mut)] // `pending` may be unused when features are disabled
    pub fn fire_discoveries(
        config: &ProvidersConfig,
        env_overrides: &HashMap<String, String>,
    ) -> PendingDiscoveries {
        let mut pending: PendingDiscoveries = Vec::new();

        // ── OpenAI builtin ───────────────────────────────────────────────
        if config.is_enabled("openai")
            && !cfg!(test)
            && let Some(key) = resolve_api_key(config, "openai", "OPENAI_API_KEY", env_overrides)
            && should_fetch_models(config, "openai")
        {
            let base_url = config
                .get("openai")
                .and_then(|e| e.base_url.clone())
                .or_else(|| env_value(env_overrides, "OPENAI_BASE_URL"))
                .unwrap_or_else(|| "https://api.openai.com/v1".into());
            pending.push((
                "openai".into(),
                openai::start_model_discovery(key.clone(), base_url),
            ));
        }

        // ── Anthropic builtin ───────────────────────────────────────────
        if config.is_enabled("anthropic")
            && !cfg!(test)
            && let Some(key) =
                resolve_api_key(config, "anthropic", "ANTHROPIC_API_KEY", env_overrides)
            && should_fetch_models(config, "anthropic")
        {
            let base_url = config
                .get("anthropic")
                .and_then(|e| e.base_url.clone())
                .or_else(|| env_value(env_overrides, "ANTHROPIC_BASE_URL"))
                .unwrap_or_else(|| "https://api.anthropic.com".into());
            pending.push((
                "anthropic".into(),
                anthropic::start_model_discovery(key.clone(), base_url),
            ));
        }

        // ── OpenAI-compatible providers ──────────────────────────────────
        for def in OPENAI_COMPAT_PROVIDERS {
            if !config.is_enabled(def.config_name) {
                continue;
            }

            let key = resolve_api_key(config, def.config_name, def.env_key, env_overrides);
            let key = if !def.requires_api_key {
                key.or_else(|| Some(secrecy::Secret::new(def.config_name.into())))
            } else if def.config_name == "gemini" {
                key.or_else(|| env_value(env_overrides, "GOOGLE_API_KEY").map(secrecy::Secret::new))
            } else {
                key
            };

            let Some(key) = key else {
                continue;
            };

            let base_url = config
                .get(def.config_name)
                .and_then(|e| e.base_url.clone())
                .or_else(|| env_value(env_overrides, def.env_base_url_key))
                .unwrap_or_else(|| def.default_base_url.into());

            let preferred = configured_models_for_provider(config, def.config_name);

            if def.local_only {
                let has_explicit_entry = config.get(def.config_name).is_some();
                let has_env_base_url = env_value(env_overrides, def.env_base_url_key).is_some();
                if !has_explicit_entry && !has_env_base_url && preferred.is_empty() {
                    continue;
                }
            }

            let skip_discovery = def.models.is_empty()
                && preferred.is_empty()
                && !def.local_only
                && (def.config_name == "venice" || cfg!(test));
            let user_opted_in = config
                .get(def.config_name)
                .is_some_and(|entry| entry.fetch_models);
            let try_fetch = def.supports_model_discovery || user_opted_in;

            if !skip_discovery && try_fetch && should_fetch_models(config, def.config_name) {
                if def.config_name == "ollama" {
                    pending.push((
                        def.config_name.into(),
                        ollama::start_ollama_discovery(&base_url),
                    ));
                } else {
                    pending.push((
                        def.config_name.into(),
                        openai::start_model_discovery(key.clone(), base_url),
                    ));
                }
            }
        }

        // ── Custom providers ─────────────────────────────────────────────
        for (name, entry) in &config.providers {
            if !name.starts_with("custom-") || !entry.enabled {
                continue;
            }
            let Some(api_key) = entry
                .api_key
                .as_ref()
                .filter(|k| !k.expose_secret().is_empty())
            else {
                continue;
            };
            let Some(base_url) = entry.base_url.as_ref().filter(|u| !u.trim().is_empty()) else {
                continue;
            };
            let has_explicit_models = !configured_models_for_provider(config, name).is_empty();
            if !has_explicit_models && should_fetch_models(config, name) {
                pending.push((
                    name.clone(),
                    openai::start_model_discovery(api_key.clone(), base_url.clone()),
                ));
            }
        }

        // ── OpenAI Codex ─────────────────────────────────────────────────
        #[cfg(feature = "provider-openai-codex")]
        if oauth_discovery_enabled(config, "openai-codex")
            && crate::openai_codex::has_stored_tokens()
            && should_fetch_models(config, "openai-codex")
            && let Some(rx) = crate::openai_codex::start_model_discovery()
        {
            pending.push(("openai-codex".into(), rx));
        }

        // ── GitHub Copilot ───────────────────────────────────────────────
        #[cfg(feature = "provider-github-copilot")]
        if oauth_discovery_enabled(config, "github-copilot")
            && crate::github_copilot::has_stored_tokens()
            && should_fetch_models(config, "github-copilot")
        {
            pending.push((
                "github-copilot".into(),
                crate::github_copilot::start_model_discovery(),
            ));
        }

        pending
    }

    /// Drain all pending discovery receivers (blocking on each `recv()`).
    ///
    /// Returns a map from provider name to discovered models.
    pub fn collect_discoveries(
        pending: PendingDiscoveries,
    ) -> HashMap<String, Vec<DiscoveredModel>> {
        let mut results: HashMap<String, Vec<DiscoveredModel>> = HashMap::new();
        for (key, rx) in pending {
            match rx.recv() {
                Ok(Ok(models)) => {
                    tracing::debug!(
                        provider = %key,
                        model_count = models.len(),
                        "parallel model discovery succeeded"
                    );
                    results.insert(key, models);
                },
                Ok(Err(err)) => {
                    let msg = err.to_string();
                    if msg.contains("not logged in")
                        || msg.contains("tokens not found")
                        || msg.contains("not configured")
                    {
                        tracing::debug!(
                            provider = %key,
                            error = %err,
                            "provider not configured, skipping model discovery"
                        );
                    } else {
                        tracing::warn!(
                            provider = %key,
                            error = %err,
                            "parallel model discovery failed"
                        );
                    }
                },
                Err(err) => {
                    tracing::warn!(
                        provider = %key,
                        error = %err,
                        "parallel model discovery worker crashed"
                    );
                },
            }
        }
        results
    }
}
