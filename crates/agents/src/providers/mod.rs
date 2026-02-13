pub mod anthropic;
pub mod openai;
pub mod openai_compat;

#[cfg(feature = "provider-genai")]
pub mod genai_provider;

#[cfg(feature = "provider-async-openai")]
pub mod async_openai_provider;

#[cfg(feature = "provider-openai-codex")]
pub mod openai_codex;

#[cfg(feature = "provider-github-copilot")]
pub mod github_copilot;

#[cfg(feature = "provider-kimi-code")]
pub mod kimi_code;

#[cfg(feature = "local-llm")]
pub mod local_gguf;

#[cfg(feature = "local-llm")]
pub mod local_llm;

use std::{
    collections::{HashMap, HashSet},
    pin::Pin,
    sync::Arc,
};

use {moltis_config::schema::ProvidersConfig, secrecy::ExposeSecret, tokio_stream::Stream};

use crate::model::{ChatMessage, LlmProvider, StreamEvent};

/// A model discovered from a provider API (e.g. `/v1/models`).
///
/// Replaces bare `(String, String)` tuples so that optional metadata
/// such as `created_at` can travel alongside the id/display_name pair.
#[derive(Debug, Clone)]
pub struct DiscoveredModel {
    pub id: String,
    pub display_name: String,
    /// Unix timestamp from the API (e.g. OpenAI `created` field).
    /// Used to sort models newest-first. `None` for static catalog entries.
    pub created_at: Option<i64>,
}

impl DiscoveredModel {
    pub fn new(id: impl Into<String>, display_name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            display_name: display_name.into(),
            created_at: None,
        }
    }

    pub fn with_created_at(mut self, created_at: Option<i64>) -> Self {
        self.created_at = created_at;
        self
    }
}

const MODEL_ID_NAMESPACE_SEP: &str = "::";

#[must_use]
pub fn namespaced_model_id(provider: &str, model_id: &str) -> String {
    if model_id.contains(MODEL_ID_NAMESPACE_SEP) {
        return model_id.to_string();
    }
    format!("{provider}{MODEL_ID_NAMESPACE_SEP}{model_id}")
}

#[must_use]
pub fn raw_model_id(model_id: &str) -> &str {
    model_id
        .rsplit_once(MODEL_ID_NAMESPACE_SEP)
        .map(|(_, raw)| raw)
        .unwrap_or(model_id)
}

fn configured_model_for_provider<'a>(provider: &str, model_id: &'a str) -> &'a str {
    match model_id.split_once(MODEL_ID_NAMESPACE_SEP) {
        Some((cfg_provider, raw)) if cfg_provider == provider => raw,
        _ => model_id,
    }
}

fn configured_models_for_provider(config: &ProvidersConfig, provider: &str) -> Vec<String> {
    let configured = config
        .get(provider)
        .map(|entry| entry.models.clone())
        .unwrap_or_default();

    normalize_unique_models(
        configured
            .into_iter()
            .map(|model| configured_model_for_provider(provider, model.trim()).to_string()),
    )
}

fn normalize_unique_models(models: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut normalized_models = Vec::new();
    let mut seen = HashSet::new();
    for model in models {
        let normalized = model.trim().to_string();
        if normalized.is_empty() || !seen.insert(normalized.clone()) {
            continue;
        }
        normalized_models.push(normalized);
    }
    normalized_models
}

fn should_fetch_models(config: &ProvidersConfig, provider: &str) -> bool {
    config.get(provider).is_none_or(|entry| entry.fetch_models)
}

fn merge_preferred_and_discovered_models(
    preferred: Vec<String>,
    discovered: Vec<DiscoveredModel>,
) -> Vec<DiscoveredModel> {
    let discovered_by_id: HashMap<String, &DiscoveredModel> =
        discovered.iter().map(|m| (m.id.clone(), m)).collect();
    let mut merged = Vec::new();
    let mut seen = HashSet::new();

    for model_id in preferred {
        if !seen.insert(model_id.clone()) {
            continue;
        }
        let model = if let Some(d) = discovered_by_id.get(&model_id) {
            DiscoveredModel {
                id: model_id,
                display_name: d.display_name.clone(),
                created_at: d.created_at,
            }
        } else {
            DiscoveredModel::new(model_id.clone(), model_id)
        };
        merged.push(model);
    }

    for model in discovered {
        if !seen.insert(model.id.clone()) {
            continue;
        }
        merged.push(model);
    }

    merged
}

fn merge_discovered_with_fallback_catalog(
    discovered: Vec<DiscoveredModel>,
    fallback: Vec<DiscoveredModel>,
) -> Vec<DiscoveredModel> {
    if discovered.is_empty() {
        return fallback;
    }

    let fallback_by_id: HashMap<String, DiscoveredModel> =
        fallback.into_iter().map(|m| (m.id.clone(), m)).collect();
    discovered
        .into_iter()
        .map(|m| {
            let display_name = if m.display_name.trim().is_empty() {
                fallback_by_id
                    .get(&m.id)
                    .map(|fb| fb.display_name.clone())
                    .unwrap_or_else(|| m.id.clone())
            } else {
                m.display_name
            };
            DiscoveredModel {
                id: m.id,
                display_name,
                created_at: m.created_at,
            }
        })
        .collect()
}

fn normalize_ollama_api_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    trimmed.strip_suffix("/v1").unwrap_or(trimmed).to_string()
}

#[derive(Debug, serde::Deserialize)]
struct OllamaTagEntry {
    name: String,
}

#[derive(Debug, serde::Deserialize)]
struct OllamaTagsPayload {
    #[serde(default)]
    models: Vec<OllamaTagEntry>,
}

async fn discover_ollama_models_from_api(base_url: String) -> anyhow::Result<Vec<DiscoveredModel>> {
    let api_base = normalize_ollama_api_base_url(&base_url);
    let endpoint = format!("{}/api/tags", api_base.trim_end_matches('/'));
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()?
        .get(&endpoint)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("ollama model discovery failed HTTP {status}");
    }

    let payload: OllamaTagsPayload = response.json().await?;
    let mut models: Vec<DiscoveredModel> = payload
        .models
        .into_iter()
        .map(|entry| entry.name.trim().to_string())
        .filter(|model| !model.is_empty())
        .map(|model| DiscoveredModel::new(model.clone(), model))
        .collect();
    models.sort_by(|left, right| left.id.cmp(&right.id));
    models.dedup_by(|left, right| left.id == right.id);
    Ok(models)
}

fn discover_ollama_models(base_url: &str) -> anyhow::Result<Vec<DiscoveredModel>> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let base_url = base_url.to_string();
    std::thread::spawn(move || {
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(anyhow::Error::from)
            .and_then(|rt| rt.block_on(discover_ollama_models_from_api(base_url)));
        let _ = tx.send(result);
    });

    rx.recv()
        .map_err(|err| anyhow::anyhow!("ollama model discovery worker failed: {err}"))?
}

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
    ) -> anyhow::Result<crate::model::CompletionResponse> {
        self.inner.complete(messages, tools).await
    }

    fn supports_tools(&self) -> bool {
        self.inner.supports_tools()
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
}

/// Resolve an API key from config (Secret) or environment variable,
/// keeping the value wrapped in `Secret<String>` to avoid leaking it.
fn resolve_api_key(
    config: &ProvidersConfig,
    provider: &str,
    env_key: &str,
) -> Option<secrecy::Secret<String>> {
    config
        .get(provider)
        .and_then(|e| e.api_key.clone())
        .or_else(|| {
            std::env::var(env_key)
                .ok()
                .filter(|k| !k.is_empty())
                .map(secrecy::Secret::new)
        })
        .filter(|s| !s.expose_secret().is_empty())
}

/// Return the known context window size (in tokens) for a model ID.
/// Falls back to 200,000 for unknown models.
pub fn context_window_for_model(model_id: &str) -> u32 {
    let model_id = raw_model_id(model_id);
    // Codestral has the largest window at 256k.
    if model_id.starts_with("codestral") {
        return 256_000;
    }
    // Claude models: 200k.
    if model_id.starts_with("claude-") {
        return 200_000;
    }
    // OpenAI o3/o4-mini: 200k.
    if model_id.starts_with("o3") || model_id.starts_with("o4-mini") {
        return 200_000;
    }
    // GPT-4o, GPT-4-turbo, GPT-5 series: 128k.
    if model_id.starts_with("gpt-4") || model_id.starts_with("gpt-5") {
        return 128_000;
    }
    // Mistral Large: 128k.
    if model_id.starts_with("mistral-large") {
        return 128_000;
    }
    // Gemini: 1M context.
    if model_id.starts_with("gemini-") {
        return 1_000_000;
    }
    // Kimi K2.5: 128k.
    if model_id.starts_with("kimi-") {
        return 128_000;
    }
    // Default fallback.
    200_000
}

/// Check if a model supports vision (image inputs).
///
/// Vision-capable models can process images in tool results and user messages.
/// When true, the runner sends images as multimodal content blocks rather than
/// stripping them from the context.
pub fn supports_vision_for_model(model_id: &str) -> bool {
    let model_id = raw_model_id(model_id);
    // Claude models: all modern Claude models support vision
    if model_id.starts_with("claude-") {
        return true;
    }
    // GPT-4o and variants support vision
    if model_id.starts_with("gpt-4o") {
        return true;
    }
    // GPT-4 turbo supports vision
    if model_id.starts_with("gpt-4-turbo") {
        return true;
    }
    // GPT-5 series supports vision
    if model_id.starts_with("gpt-5") {
        return true;
    }
    // o3/o4 series supports vision
    if model_id.starts_with("o3") || model_id.starts_with("o4") {
        return true;
    }
    // Gemini models support vision
    if model_id.starts_with("gemini-") {
        return true;
    }
    // Default: no vision support
    false
}

/// Info about an available model.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub provider: String,
    pub display_name: String,
    /// Unix timestamp from the provider API (e.g. OpenAI `created` field).
    /// `None` for static catalog entries.
    pub created_at: Option<i64>,
}

/// Known Anthropic Claude models (model_id, display_name).
/// Current models listed first, then legacy models.
const ANTHROPIC_MODELS: &[(&str, &str)] = &[
    ("claude-opus-4-5-20251101", "Claude Opus 4.5"),
    ("claude-sonnet-4-5-20250929", "Claude Sonnet 4.5"),
    ("claude-haiku-4-5-20251001", "Claude Haiku 4.5"),
    ("claude-opus-4-1-20250805", "Claude Opus 4.1"),
    ("claude-sonnet-4-20250514", "Claude Sonnet 4"),
    ("claude-opus-4-20250514", "Claude Opus 4"),
    ("claude-3-7-sonnet-20250219", "Claude 3.7 Sonnet"),
    ("claude-3-haiku-20240307", "Claude 3 Haiku"),
];

/// Known Mistral models.
const MISTRAL_MODELS: &[(&str, &str)] = &[
    ("mistral-large-latest", "Mistral Large"),
    ("codestral-latest", "Codestral"),
];

/// Known Cerebras models.
const CEREBRAS_MODELS: &[(&str, &str)] =
    &[("llama-4-scout-17b-16e-instruct", "Llama 4 Scout (Cerebras)")];

/// Known MiniMax models.
const MINIMAX_MODELS: &[(&str, &str)] = &[("MiniMax-M2.1", "MiniMax M2.1")];

/// Known Moonshot models.
const MOONSHOT_MODELS: &[(&str, &str)] = &[("kimi-k2.5", "Kimi K2.5")];

/// OpenAI-compatible provider definition for table-driven registration.
struct OpenAiCompatDef {
    config_name: &'static str,
    env_key: &'static str,
    env_base_url_key: &'static str,
    default_base_url: &'static str,
    models: &'static [(&'static str, &'static str)],
}

const OPENAI_COMPAT_PROVIDERS: &[OpenAiCompatDef] = &[
    OpenAiCompatDef {
        config_name: "mistral",
        env_key: "MISTRAL_API_KEY",
        env_base_url_key: "MISTRAL_BASE_URL",
        default_base_url: "https://api.mistral.ai/v1",
        models: MISTRAL_MODELS,
    },
    OpenAiCompatDef {
        config_name: "openrouter",
        env_key: "OPENROUTER_API_KEY",
        env_base_url_key: "OPENROUTER_BASE_URL",
        default_base_url: "https://openrouter.ai/api/v1",
        models: &[],
    },
    OpenAiCompatDef {
        config_name: "cerebras",
        env_key: "CEREBRAS_API_KEY",
        env_base_url_key: "CEREBRAS_BASE_URL",
        default_base_url: "https://api.cerebras.ai/v1",
        models: CEREBRAS_MODELS,
    },
    OpenAiCompatDef {
        config_name: "minimax",
        env_key: "MINIMAX_API_KEY",
        env_base_url_key: "MINIMAX_BASE_URL",
        default_base_url: "https://api.minimax.chat/v1",
        models: MINIMAX_MODELS,
    },
    OpenAiCompatDef {
        config_name: "moonshot",
        env_key: "MOONSHOT_API_KEY",
        env_base_url_key: "MOONSHOT_BASE_URL",
        default_base_url: "https://api.moonshot.ai/v1",
        models: MOONSHOT_MODELS,
    },
    OpenAiCompatDef {
        config_name: "venice",
        env_key: "VENICE_API_KEY",
        env_base_url_key: "VENICE_BASE_URL",
        default_base_url: "https://api.venice.ai/api/v1",
        models: &[],
    },
    OpenAiCompatDef {
        config_name: "ollama",
        env_key: "OLLAMA_API_KEY",
        env_base_url_key: "OLLAMA_BASE_URL",
        default_base_url: "http://127.0.0.1:11434/v1",
        models: &[],
    },
];

#[cfg(any(feature = "provider-openai-codex", feature = "provider-github-copilot"))]
trait DynamicModelDiscovery {
    fn provider_name(&self) -> &'static str;
    fn is_enabled_and_authenticated(&self, config: &ProvidersConfig) -> bool;
    fn configured_models(&self, config: &ProvidersConfig) -> Vec<String>;
    fn should_fetch_models(&self, config: &ProvidersConfig) -> bool;
    fn available_models(&self) -> Vec<DiscoveredModel>;
    fn live_models(&self) -> anyhow::Result<Vec<DiscoveredModel>>;
    fn build_provider(&self, model_id: String) -> Arc<dyn LlmProvider>;
    fn display_name(&self, model_id: &str, discovered: &str) -> String;
}

#[cfg(feature = "provider-openai-codex")]
struct OpenAiCodexDiscovery;

#[cfg(feature = "provider-openai-codex")]
impl DynamicModelDiscovery for OpenAiCodexDiscovery {
    fn provider_name(&self) -> &'static str {
        "openai-codex"
    }

    fn is_enabled_and_authenticated(&self, config: &ProvidersConfig) -> bool {
        config.is_enabled(self.provider_name()) && openai_codex::has_stored_tokens()
    }

    fn configured_models(&self, config: &ProvidersConfig) -> Vec<String> {
        configured_models_for_provider(config, self.provider_name())
    }

    fn should_fetch_models(&self, config: &ProvidersConfig) -> bool {
        should_fetch_models(config, self.provider_name())
    }

    fn available_models(&self) -> Vec<DiscoveredModel> {
        openai_codex::available_models()
    }

    fn live_models(&self) -> anyhow::Result<Vec<DiscoveredModel>> {
        openai_codex::live_models()
    }

    fn build_provider(&self, model_id: String) -> Arc<dyn LlmProvider> {
        Arc::new(openai_codex::OpenAiCodexProvider::new(model_id))
    }

    fn display_name(&self, _model_id: &str, discovered: &str) -> String {
        format!("{discovered} (Codex/OAuth)")
    }
}

#[cfg(feature = "provider-github-copilot")]
struct GitHubCopilotDiscovery;

#[cfg(feature = "provider-github-copilot")]
impl DynamicModelDiscovery for GitHubCopilotDiscovery {
    fn provider_name(&self) -> &'static str {
        "github-copilot"
    }

    fn is_enabled_and_authenticated(&self, config: &ProvidersConfig) -> bool {
        config.is_enabled(self.provider_name()) && github_copilot::has_stored_tokens()
    }

    fn configured_models(&self, config: &ProvidersConfig) -> Vec<String> {
        configured_models_for_provider(config, self.provider_name())
    }

    fn should_fetch_models(&self, config: &ProvidersConfig) -> bool {
        should_fetch_models(config, self.provider_name())
    }

    fn available_models(&self) -> Vec<DiscoveredModel> {
        github_copilot::available_models()
    }

    fn live_models(&self) -> anyhow::Result<Vec<DiscoveredModel>> {
        github_copilot::live_models()
    }

    fn build_provider(&self, model_id: String) -> Arc<dyn LlmProvider> {
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
    providers: HashMap<String, Arc<dyn LlmProvider>>,
    models: Vec<ModelInfo>,
}

impl ProviderRegistry {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            providers: HashMap::new(),
            models: Vec::new(),
        }
    }

    fn has_provider_model(&self, provider: &str, model_id: &str) -> bool {
        self.providers
            .contains_key(&namespaced_model_id(provider, model_id))
    }

    fn resolve_registry_model_id(
        &self,
        model_id: &str,
        provider_hint: Option<&str>,
    ) -> Option<String> {
        if self.providers.contains_key(model_id) {
            return Some(model_id.to_string());
        }

        let raw = raw_model_id(model_id);
        let mut matches = self
            .models
            .iter()
            .filter(|m| raw_model_id(&m.id) == raw)
            .filter(|m| provider_hint.is_none_or(|hint| m.provider == hint))
            .map(|m| m.id.clone());

        matches.next()
    }

    #[cfg(any(feature = "provider-openai-codex", feature = "provider-github-copilot"))]
    #[allow(clippy::vec_init_then_push)]
    fn dynamic_discovery_sources() -> Vec<Box<dyn DynamicModelDiscovery>> {
        let mut sources: Vec<Box<dyn DynamicModelDiscovery>> = Vec::new();
        #[cfg(feature = "provider-openai-codex")]
        sources.push(Box::new(OpenAiCodexDiscovery));
        #[cfg(feature = "provider-github-copilot")]
        sources.push(Box::new(GitHubCopilotDiscovery));
        sources
    }

    #[cfg(any(feature = "provider-openai-codex", feature = "provider-github-copilot"))]
    fn desired_models_for_dynamic_source(
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
    fn register_dynamic_source_models(
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
            let provider = source.build_provider(model.id.clone());
            self.register(
                ModelInfo {
                    id: model.id.clone(),
                    provider: source.provider_name().to_string(),
                    display_name: source.display_name(&model.id, &model.display_name),
                    created_at: model.created_at,
                },
                provider,
            );
        }
    }

    #[cfg(any(feature = "provider-openai-codex", feature = "provider-github-copilot"))]
    fn refresh_dynamic_source_models(
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
                (
                    ModelInfo {
                        id: model.id.clone(),
                        provider: source.provider_name().to_string(),
                        display_name: source.display_name(&model.id, &model.display_name),
                        created_at: model.created_at,
                    },
                    source.build_provider(model.id),
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
        Self::from_env_with_config(&ProvidersConfig::default())
    }

    /// Auto-discover providers from environment variables,
    /// respecting the given config for enable/disable and overrides.
    ///
    /// Provider priority (first registered wins for a given model ID):
    /// 1. Built-in raw reqwest providers (always available, support tool calling)
    /// 2. async-openai-backed providers (if `provider-async-openai` feature enabled)
    /// 3. genai-backed providers (if `provider-genai` feature enabled, no tool support)
    /// 4. OpenAI Codex OAuth providers (if `provider-openai-codex` feature enabled)
    pub fn from_env_with_config(config: &ProvidersConfig) -> Self {
        let mut reg = Self::empty();

        // Built-in providers first: they support tool calling.
        reg.register_builtin_providers(config);
        reg.register_openai_compatible_providers(config);

        #[cfg(feature = "provider-async-openai")]
        {
            reg.register_async_openai_providers(config);
        }

        // GenAI providers last: they don't support tool calling,
        // so they only fill in models not already covered above.
        #[cfg(feature = "provider-genai")]
        {
            reg.register_genai_providers(config);
        }

        #[cfg(feature = "provider-openai-codex")]
        {
            reg.register_openai_codex_providers(config);
        }

        #[cfg(feature = "provider-github-copilot")]
        {
            reg.register_github_copilot_providers(config);
        }

        #[cfg(feature = "provider-kimi-code")]
        {
            reg.register_kimi_code_providers(config);
        }

        // Local GGUF providers (no API key needed, model runs locally)
        #[cfg(feature = "local-llm")]
        {
            reg.register_local_gguf_providers(config);
        }

        reg
    }

    #[cfg(feature = "provider-genai")]
    fn register_genai_providers(&mut self, config: &ProvidersConfig) {
        // (env_key, provider_config_name, model_id, display_name)
        let genai_models: &[(&str, &str, &str, &str)] = &[
            (
                "ANTHROPIC_API_KEY",
                "anthropic",
                "claude-sonnet-4-20250514",
                "Claude Sonnet 4 (genai)",
            ),
            ("OPENAI_API_KEY", "openai", "gpt-4o", "GPT-4o (genai)"),
            (
                "GEMINI_API_KEY",
                "gemini",
                "gemini-2.0-flash",
                "Gemini 2.0 Flash (genai)",
            ),
            (
                "GROQ_API_KEY",
                "groq",
                "llama-3.1-8b-instant",
                "Llama 3.1 8B (genai/groq)",
            ),
            ("XAI_API_KEY", "xai", "grok-3-mini", "Grok 3 Mini (genai)"),
            (
                "DEEPSEEK_API_KEY",
                "deepseek",
                "deepseek-chat",
                "DeepSeek Chat (genai)",
            ),
        ];

        for &(env_key, provider_name, default_model_id, display_name) in genai_models {
            if !config.is_enabled(provider_name) {
                continue;
            }

            // Use config api_key or fall back to env var.
            let Some(resolved_key) = resolve_api_key(config, provider_name, env_key) else {
                continue;
            };

            let model_id = configured_models_for_provider(config, provider_name)
                .into_iter()
                .next()
                .unwrap_or_else(|| default_model_id.to_string());

            // Get alias if configured (for metrics differentiation).
            let alias = config.get(provider_name).and_then(|e| e.alias.clone());
            let genai_provider_name = alias.unwrap_or_else(|| format!("genai/{provider_name}"));
            if self.has_provider_model(&genai_provider_name, &model_id) {
                continue;
            }

            let provider = Arc::new(genai_provider::GenaiProvider::new(
                model_id.clone(),
                genai_provider_name.clone(),
                resolved_key,
            ));
            self.register(
                ModelInfo {
                    id: model_id,
                    provider: genai_provider_name,
                    display_name: display_name.into(),
                    created_at: None,
                },
                provider,
            );
        }
    }

    #[cfg(feature = "provider-async-openai")]
    fn register_async_openai_providers(&mut self, config: &ProvidersConfig) {
        if !config.is_enabled("openai") {
            return;
        }

        let Some(key) = resolve_api_key(config, "openai", "OPENAI_API_KEY") else {
            return;
        };

        let base_url = config
            .get("openai")
            .and_then(|e| e.base_url.clone())
            .or_else(|| std::env::var("OPENAI_BASE_URL").ok())
            .unwrap_or_else(|| "https://api.openai.com/v1".into());

        let model_id = configured_models_for_provider(config, "openai")
            .into_iter()
            .next()
            .unwrap_or_else(|| "gpt-4o".to_string());

        // Get alias if configured (for metrics differentiation).
        let alias = config.get("openai").and_then(|e| e.alias.clone());
        let provider_label = alias.clone().unwrap_or_else(|| "async-openai".into());
        if self.has_provider_model(&provider_label, &model_id) {
            return;
        }

        let provider = Arc::new(async_openai_provider::AsyncOpenAiProvider::with_alias(
            key,
            model_id.clone(),
            base_url,
            alias,
        ));
        self.register(
            ModelInfo {
                id: model_id,
                provider: provider_label,
                display_name: "GPT-4o (async-openai)".into(),
                created_at: None,
            },
            provider,
        );
    }

    #[cfg(feature = "provider-openai-codex")]
    fn register_openai_codex_providers(&mut self, config: &ProvidersConfig) {
        let source = OpenAiCodexDiscovery;
        let catalog = if source.should_fetch_models(config) {
            source.available_models()
        } else {
            Vec::new()
        };
        self.register_dynamic_source_models(&source, config, catalog);
    }

    pub fn refresh_openai_codex_models(&mut self, config: &ProvidersConfig) -> bool {
        #[cfg(feature = "provider-openai-codex")]
        {
            let source = OpenAiCodexDiscovery;
            self.refresh_dynamic_source_models(&source, config)
        }

        #[cfg(not(feature = "provider-openai-codex"))]
        {
            let _ = config;
            false
        }
    }

    #[cfg(feature = "provider-github-copilot")]
    fn register_github_copilot_providers(&mut self, config: &ProvidersConfig) {
        let source = GitHubCopilotDiscovery;
        let catalog = if source.should_fetch_models(config) {
            source.available_models()
        } else {
            Vec::new()
        };
        self.register_dynamic_source_models(&source, config, catalog);
    }

    pub fn refresh_github_copilot_models(&mut self, config: &ProvidersConfig) -> bool {
        #[cfg(feature = "provider-github-copilot")]
        {
            let source = GitHubCopilotDiscovery;
            self.refresh_dynamic_source_models(&source, config)
        }

        #[cfg(not(feature = "provider-github-copilot"))]
        {
            let _ = config;
            false
        }
    }

    pub fn refresh_dynamic_models(&mut self, config: &ProvidersConfig) -> Vec<(String, bool)> {
        #[cfg(any(feature = "provider-openai-codex", feature = "provider-github-copilot"))]
        {
            let mut results = Vec::new();
            for source in Self::dynamic_discovery_sources() {
                let refreshed = self.refresh_dynamic_source_models(source.as_ref(), config);
                results.push((source.provider_name().to_string(), refreshed));
            }
            results
        }

        #[cfg(not(any(feature = "provider-openai-codex", feature = "provider-github-copilot")))]
        {
            let _ = config;
            Vec::new()
        }
    }

    #[cfg(feature = "provider-kimi-code")]
    fn register_kimi_code_providers(&mut self, config: &ProvidersConfig) {
        if !config.is_enabled("kimi-code") {
            return;
        }

        let api_key = resolve_api_key(config, "kimi-code", "KIMI_API_KEY");
        let has_oauth_tokens = kimi_code::has_stored_tokens();
        if api_key.is_none() && !has_oauth_tokens {
            return;
        }

        let base_url = config
            .get("kimi-code")
            .and_then(|e| e.base_url.clone())
            .or_else(|| std::env::var("KIMI_BASE_URL").ok())
            .unwrap_or_else(|| "https://api.kimi.com/coding/v1".into());

        let build_provider = |model_id: &str| -> Arc<dyn LlmProvider> {
            if let Some(api_key) = api_key.as_ref() {
                Arc::new(kimi_code::KimiCodeProvider::new_with_api_key(
                    api_key.clone(),
                    model_id.into(),
                    base_url.clone(),
                ))
            } else {
                Arc::new(kimi_code::KimiCodeProvider::new(model_id.into()))
            }
        };

        let preferred = configured_models_for_provider(config, "kimi-code");
        let discovered = if should_fetch_models(config, "kimi-code") {
            kimi_code::KIMI_CODE_MODELS
                .iter()
                .map(|(id, name)| DiscoveredModel::new(*id, *name))
                .collect()
        } else {
            Vec::new()
        };
        let models = merge_preferred_and_discovered_models(preferred, discovered);
        for model in models {
            let (model_id, display_name, created_at) =
                (model.id, model.display_name, model.created_at);
            if self.has_provider_model("kimi-code", &model_id) {
                continue;
            }
            let provider = build_provider(&model_id);
            self.register(
                ModelInfo {
                    id: model_id,
                    provider: "kimi-code".into(),
                    display_name,
                    created_at,
                },
                provider,
            );
        }
    }

    #[cfg(feature = "local-llm")]
    fn register_local_gguf_providers(&mut self, config: &ProvidersConfig) {
        use std::path::PathBuf;

        if !config.is_enabled("local") {
            return;
        }

        // Log system info once
        local_gguf::log_system_info_and_suggestions();

        // Collect all model IDs to register:
        // 1. From local_models (multi-model config from local-llm.json)
        // 2. From provider models in config (preferred pins)
        let mut model_ids: Vec<String> = config.local_models.clone();
        model_ids.extend(configured_models_for_provider(config, "local"));
        model_ids = normalize_unique_models(model_ids);

        if model_ids.is_empty() {
            tracing::info!(
                "local-llm enabled but no models configured. Add [providers.local] models = [\"...\"] to config."
            );
            return;
        }

        // Build config from provider entry for user overrides
        let entry = config.get("local");
        let user_model_path = entry
            .and_then(|e| e.base_url.as_deref()) // Reuse base_url for model_path
            .map(PathBuf::from);

        // Register each model
        for model_id in model_ids {
            if self.has_provider_model("local-llm", &model_id) {
                continue;
            }

            // Look up model in registries to get display name
            let display_name = if let Some(def) = local_llm::models::find_model(&model_id) {
                def.display_name.to_string()
            } else if let Some(def) = local_gguf::models::find_model(&model_id) {
                def.display_name.to_string()
            } else {
                format!("{} (local)", model_id)
            };

            // Use LocalLlmProvider which auto-detects backend based on model type
            let llm_config = local_llm::LocalLlmConfig {
                model_id: model_id.clone(),
                model_path: user_model_path.clone(),
                backend: None, // Auto-detect based on model type
                context_size: None,
                gpu_layers: 0,
                temperature: 0.7,
                cache_dir: local_llm::models::default_models_dir(),
            };

            tracing::info!(
                model = %model_id,
                display_name = %display_name,
                "local-llm model configured (will load on first use)"
            );

            // Use LocalLlmProvider which properly routes to GGUF or MLX backend
            let provider = Arc::new(local_llm::LocalLlmProvider::new(llm_config));
            self.register(
                ModelInfo {
                    id: model_id,
                    provider: "local-llm".into(),
                    display_name,
                    created_at: None,
                },
                provider,
            );
        }
    }

    fn register_builtin_providers(&mut self, config: &ProvidersConfig) {
        // Anthropic — register all known Claude models when API key is available.
        if config.is_enabled("anthropic")
            && let Some(key) = resolve_api_key(config, "anthropic", "ANTHROPIC_API_KEY")
        {
            let base_url = config
                .get("anthropic")
                .and_then(|e| e.base_url.clone())
                .or_else(|| std::env::var("ANTHROPIC_BASE_URL").ok())
                .unwrap_or_else(|| "https://api.anthropic.com".into());

            // Get alias if configured (for metrics differentiation).
            let alias = config.get("anthropic").and_then(|e| e.alias.clone());
            let provider_label = alias.clone().unwrap_or_else(|| "anthropic".into());
            let preferred = configured_models_for_provider(config, "anthropic");
            let discovered = if should_fetch_models(config, "anthropic") {
                ANTHROPIC_MODELS
                    .iter()
                    .map(|(id, name)| DiscoveredModel::new(*id, *name))
                    .collect()
            } else {
                Vec::new()
            };
            let models = merge_preferred_and_discovered_models(preferred, discovered);

            for model in models {
                let (model_id, display_name, created_at) =
                    (model.id, model.display_name, model.created_at);
                if self.has_provider_model(&provider_label, &model_id) {
                    continue;
                }
                let provider = Arc::new(anthropic::AnthropicProvider::with_alias(
                    key.clone(),
                    model_id.clone(),
                    base_url.clone(),
                    alias.clone(),
                ));
                self.register(
                    ModelInfo {
                        id: model_id,
                        provider: provider_label.clone(),
                        display_name,
                        created_at,
                    },
                    provider,
                );
            }
        }

        // OpenAI — register all known OpenAI models when API key is available.
        if config.is_enabled("openai")
            && let Some(key) = resolve_api_key(config, "openai", "OPENAI_API_KEY")
        {
            let base_url = config
                .get("openai")
                .and_then(|e| e.base_url.clone())
                .or_else(|| std::env::var("OPENAI_BASE_URL").ok())
                .unwrap_or_else(|| "https://api.openai.com/v1".into());

            // Get alias if configured (for metrics differentiation).
            let alias = config.get("openai").and_then(|e| e.alias.clone());
            let provider_label = alias.clone().unwrap_or_else(|| "openai".into());
            let preferred = configured_models_for_provider(config, "openai");
            let discovered = if should_fetch_models(config, "openai") {
                openai::available_models(&key, &base_url)
            } else {
                Vec::new()
            };
            let models = merge_preferred_and_discovered_models(preferred, discovered);

            for model in models {
                let (model_id, display_name, created_at) =
                    (model.id, model.display_name, model.created_at);
                if self.has_provider_model(&provider_label, &model_id) {
                    continue;
                }
                let provider = Arc::new(openai::OpenAiProvider::new_with_name(
                    key.clone(),
                    model_id.clone(),
                    base_url.clone(),
                    provider_label.clone(),
                ));
                self.register(
                    ModelInfo {
                        id: model_id,
                        provider: provider_label.clone(),
                        display_name,
                        created_at,
                    },
                    provider,
                );
            }
        }
    }

    fn register_openai_compatible_providers(&mut self, config: &ProvidersConfig) {
        for def in OPENAI_COMPAT_PROVIDERS {
            if !config.is_enabled(def.config_name) {
                continue;
            }

            let key = resolve_api_key(config, def.config_name, def.env_key);

            // Ollama doesn't require an API key — use a dummy value.
            let key = if def.config_name == "ollama" {
                key.or_else(|| Some(secrecy::Secret::new("ollama".into())))
            } else {
                key
            };

            let Some(key) = key else {
                continue;
            };

            let base_url = config
                .get(def.config_name)
                .and_then(|e| e.base_url.clone())
                .or_else(|| std::env::var(def.env_base_url_key).ok())
                .unwrap_or_else(|| def.default_base_url.into());

            // Get alias if configured (for metrics differentiation).
            let alias = config.get(def.config_name).and_then(|e| e.alias.clone());
            let provider_label = alias.unwrap_or_else(|| def.config_name.into());
            let preferred = configured_models_for_provider(config, def.config_name);
            if def.config_name == "ollama" {
                let has_explicit_entry = config.get("ollama").is_some();
                let has_env_base_url = std::env::var(def.env_base_url_key)
                    .ok()
                    .is_some_and(|value| !value.trim().is_empty());
                if !has_explicit_entry && !has_env_base_url && preferred.is_empty() {
                    continue;
                }
            }
            // "Bring your own model" providers (empty static catalog, no
            // configured models) skip discovery — the user must pick a model.
            let skip_discovery =
                def.models.is_empty() && preferred.is_empty() && def.config_name != "ollama";
            let discovered = if !skip_discovery && should_fetch_models(config, def.config_name) {
                if def.config_name == "ollama" {
                    match discover_ollama_models(&base_url) {
                        Ok(models) => models,
                        Err(err) => {
                            tracing::warn!(
                                provider = def.config_name,
                                error = %err,
                                "failed to fetch live models for provider"
                            );
                            def.models
                                .iter()
                                .map(|(id, name)| DiscoveredModel::new(*id, *name))
                                .collect()
                        },
                    }
                } else {
                    match openai::live_models(&key, &base_url) {
                        Ok(models) => models,
                        Err(err) => {
                            tracing::warn!(
                                provider = def.config_name,
                                error = %err,
                                "failed to fetch live models for provider"
                            );
                            def.models
                                .iter()
                                .map(|(id, name)| DiscoveredModel::new(*id, *name))
                                .collect()
                        },
                    }
                }
            } else {
                Vec::new()
            };
            let models = merge_preferred_and_discovered_models(preferred, discovered);
            for model in models {
                let (model_id, display_name, created_at) =
                    (model.id, model.display_name, model.created_at);
                if self.has_provider_model(&provider_label, &model_id) {
                    continue;
                }
                let provider = Arc::new(openai::OpenAiProvider::new_with_name(
                    key.clone(),
                    model_id.clone(),
                    base_url.clone(),
                    provider_label.clone(),
                ));
                self.register(
                    ModelInfo {
                        id: model_id,
                        provider: provider_label.clone(),
                        display_name,
                        created_at,
                    },
                    provider,
                );
            }
        }
    }

    pub fn get(&self, model_id: &str) -> Option<Arc<dyn LlmProvider>> {
        self.resolve_registry_model_id(model_id, None)
            .as_deref()
            .and_then(|id| self.providers.get(id))
            .cloned()
    }

    pub fn first(&self) -> Option<Arc<dyn LlmProvider>> {
        self.models
            .first()
            .and_then(|m| self.providers.get(&m.id))
            .cloned()
    }

    /// Return the first provider that supports tool calling,
    /// falling back to the first provider overall.
    pub fn first_with_tools(&self) -> Option<Arc<dyn LlmProvider>> {
        self.models
            .iter()
            .filter_map(|m| self.providers.get(&m.id))
            .find(|p| p.supports_tools())
            .cloned()
            .or_else(|| self.first())
    }

    pub fn list_models(&self) -> &[ModelInfo] {
        &self.models
    }

    /// Return all registered providers in registration order.
    pub fn all_providers(&self) -> Vec<Arc<dyn LlmProvider>> {
        self.models
            .iter()
            .filter_map(|m| self.providers.get(&m.id).cloned())
            .collect()
    }

    /// Return providers for the given model IDs (in order), skipping unknown IDs.
    pub fn providers_for_models(&self, model_ids: &[String]) -> Vec<Arc<dyn LlmProvider>> {
        model_ids
            .iter()
            .filter_map(|id| {
                self.resolve_registry_model_id(id, None)
                    .as_deref()
                    .and_then(|rid| self.providers.get(rid))
                    .cloned()
            })
            .collect()
    }

    /// Return fallback providers ordered by affinity to the given primary:
    ///
    /// 1. Same model ID on a different provider backend (e.g. `gpt-4o` via openrouter)
    /// 2. Other models from the same provider (e.g. `claude-opus-4` when primary is `claude-sonnet-4`)
    /// 3. Models from other providers
    ///
    /// The primary itself is excluded from the result.
    pub fn fallback_providers_for(
        &self,
        primary_model_id: &str,
        primary_provider_name: &str,
    ) -> Vec<Arc<dyn LlmProvider>> {
        let primary_raw_model_id = raw_model_id(primary_model_id);
        let mut same_model_diff_provider = Vec::new();
        let mut same_provider_diff_model = Vec::new();
        let mut other = Vec::new();

        for info in &self.models {
            if info.id == primary_model_id && info.provider == primary_provider_name {
                continue; // skip the primary itself
            }
            let Some(p) = self.providers.get(&info.id).cloned() else {
                continue;
            };
            if raw_model_id(&info.id) == primary_raw_model_id {
                same_model_diff_provider.push(p);
            } else if info.provider == primary_provider_name {
                same_provider_diff_model.push(p);
            } else {
                other.push(p);
            }
        }

        same_model_diff_provider.extend(same_provider_diff_model);
        same_model_diff_provider.extend(other);
        same_model_diff_provider
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    pub fn provider_summary(&self) -> String {
        if self.providers.is_empty() {
            return "no LLM providers configured".into();
        }
        let provider_count = self
            .models
            .iter()
            .map(|m| m.provider.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len();
        let model_count = self.models.len();
        format!(
            "{} provider{}, {} model{}",
            provider_count,
            if provider_count == 1 {
                ""
            } else {
                "s"
            },
            model_count,
            if model_count == 1 {
                ""
            } else {
                "s"
            },
        )
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    fn secret(s: &str) -> secrecy::Secret<String> {
        secrecy::Secret::new(s.into())
    }

    #[test]
    fn context_window_for_known_models() {
        assert_eq!(
            super::context_window_for_model("claude-sonnet-4-20250514"),
            200_000
        );
        assert_eq!(
            super::context_window_for_model("claude-opus-4-5-20251101"),
            200_000
        );
        assert_eq!(super::context_window_for_model("gpt-4o"), 128_000);
        assert_eq!(super::context_window_for_model("gpt-4o-mini"), 128_000);
        assert_eq!(super::context_window_for_model("gpt-4-turbo"), 128_000);
        assert_eq!(super::context_window_for_model("o3"), 200_000);
        assert_eq!(super::context_window_for_model("o3-mini"), 200_000);
        assert_eq!(super::context_window_for_model("o4-mini"), 200_000);
        assert_eq!(super::context_window_for_model("codestral-latest"), 256_000);
        assert_eq!(
            super::context_window_for_model("mistral-large-latest"),
            128_000
        );
        assert_eq!(
            super::context_window_for_model("gemini-2.0-flash"),
            1_000_000
        );
        assert_eq!(super::context_window_for_model("kimi-k2.5"), 128_000);
    }

    #[test]
    fn context_window_fallback_for_unknown_model() {
        assert_eq!(
            super::context_window_for_model("some-unknown-model"),
            200_000
        );
    }

    #[test]
    fn provider_context_window_uses_lookup() {
        let provider = openai::OpenAiProvider::new(secret("k"), "gpt-4o".into(), "u".into());
        assert_eq!(provider.context_window(), 128_000);

        let anthropic = anthropic::AnthropicProvider::new(
            secret("k"),
            "claude-sonnet-4-20250514".into(),
            "u".into(),
        );
        assert_eq!(anthropic.context_window(), 200_000);
    }

    #[test]
    fn supports_vision_for_known_models() {
        // Claude models support vision
        assert!(super::supports_vision_for_model("claude-sonnet-4-20250514"));
        assert!(super::supports_vision_for_model("claude-opus-4-5-20251101"));
        assert!(super::supports_vision_for_model("claude-3-haiku-20240307"));

        // GPT-4o variants support vision
        assert!(super::supports_vision_for_model("gpt-4o"));
        assert!(super::supports_vision_for_model("gpt-4o-mini"));

        // GPT-4 turbo supports vision
        assert!(super::supports_vision_for_model("gpt-4-turbo"));

        // GPT-5 supports vision
        assert!(super::supports_vision_for_model("gpt-5.2-codex"));

        // o3/o4 series supports vision
        assert!(super::supports_vision_for_model("o3"));
        assert!(super::supports_vision_for_model("o3-mini"));
        assert!(super::supports_vision_for_model("o4-mini"));

        // Gemini supports vision
        assert!(super::supports_vision_for_model("gemini-2.0-flash"));
    }

    #[test]
    fn supports_vision_false_for_non_vision_models() {
        // Codestral is code-focused, no vision
        assert!(!super::supports_vision_for_model("codestral-latest"));

        // Mistral Large - no vision
        assert!(!super::supports_vision_for_model("mistral-large-latest"));

        // Kimi - no vision
        assert!(!super::supports_vision_for_model("kimi-k2.5"));

        // Unknown models default to no vision
        assert!(!super::supports_vision_for_model("some-unknown-model"));
    }

    #[test]
    fn provider_supports_vision_uses_lookup() {
        let provider = openai::OpenAiProvider::new(secret("k"), "gpt-4o".into(), "u".into());
        assert!(provider.supports_vision());

        let anthropic = anthropic::AnthropicProvider::new(
            secret("k"),
            "claude-sonnet-4-20250514".into(),
            "u".into(),
        );
        assert!(anthropic.supports_vision());

        // Non-vision model
        let mistral = openai::OpenAiProvider::new_with_name(
            secret("k"),
            "codestral-latest".into(),
            "u".into(),
            "mistral".into(),
        );
        assert!(!mistral.supports_vision());
    }

    #[test]
    fn default_context_window_trait() {
        // OpenAiProvider with unknown model should get the fallback
        let provider =
            openai::OpenAiProvider::new(secret("k"), "unknown-model-xyz".into(), "u".into());
        assert_eq!(provider.context_window(), 200_000);
    }

    #[test]
    fn merge_discovered_with_fallback_keeps_discovered_when_non_empty() {
        let merged = merge_discovered_with_fallback_catalog(
            vec![
                DiscoveredModel::new("live-a", "Live A"),
                DiscoveredModel::new("live-b", "Live B"),
            ],
            vec![
                DiscoveredModel::new("live-a", "Fallback A"),
                DiscoveredModel::new("fallback-only", "Fallback Only"),
            ],
        );

        let ids: Vec<&str> = merged.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["live-a", "live-b"]);
    }

    #[test]
    fn merge_discovered_with_fallback_uses_fallback_when_discovered_empty() {
        let merged = merge_discovered_with_fallback_catalog(Vec::new(), vec![
            DiscoveredModel::new("fallback-a", "Fallback A"),
            DiscoveredModel::new("fallback-b", "Fallback B"),
        ]);

        let ids: Vec<&str> = merged.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["fallback-a", "fallback-b"]);
    }

    #[test]
    fn model_lists_not_empty() {
        assert!(!ANTHROPIC_MODELS.is_empty());
        assert!(!openai::default_model_catalog().is_empty());
        assert!(!MISTRAL_MODELS.is_empty());
        assert!(!CEREBRAS_MODELS.is_empty());
        assert!(!MINIMAX_MODELS.is_empty());
        assert!(!MOONSHOT_MODELS.is_empty());
    }

    #[test]
    fn model_lists_have_unique_ids() {
        let openai_models = openai::default_model_catalog();
        let mut openai_ids: Vec<&str> = openai_models.iter().map(|m| m.id.as_str()).collect();
        openai_ids.sort();
        openai_ids.dedup();
        assert_eq!(
            openai_ids.len(),
            openai_models.len(),
            "duplicate OpenAI model IDs found"
        );

        for models in [
            ANTHROPIC_MODELS,
            MISTRAL_MODELS,
            CEREBRAS_MODELS,
            MINIMAX_MODELS,
            MOONSHOT_MODELS,
        ] {
            let mut ids: Vec<&str> = models.iter().map(|(id, _)| *id).collect();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), models.len(), "duplicate model IDs found");
        }
    }

    #[test]
    fn openai_compat_providers_have_unique_names() {
        let mut names: Vec<&str> = OPENAI_COMPAT_PROVIDERS
            .iter()
            .map(|d| d.config_name)
            .collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), OPENAI_COMPAT_PROVIDERS.len());
    }

    #[test]
    fn openai_compat_providers_have_valid_urls() {
        for def in OPENAI_COMPAT_PROVIDERS {
            assert!(
                def.default_base_url.starts_with("http://")
                    || def.default_base_url.starts_with("https://"),
                "{}: invalid base URL: {}",
                def.config_name,
                def.default_base_url
            );
        }
    }

    #[test]
    fn openai_compat_providers_env_keys_not_empty() {
        for def in OPENAI_COMPAT_PROVIDERS {
            assert!(
                !def.env_key.is_empty(),
                "{}: env_key is empty",
                def.config_name
            );
            assert!(
                !def.env_base_url_key.is_empty(),
                "{}: env_base_url_key is empty",
                def.config_name
            );
        }
    }

    #[test]
    fn registry_from_env_does_not_panic() {
        // Just ensure it doesn't panic with no env vars set.
        let reg = ProviderRegistry::from_env();
        let _ = reg.provider_summary();
    }

    #[test]
    fn registry_register_and_get() {
        let mut reg = ProviderRegistry::from_env_with_config(&ProvidersConfig::default());
        let initial_count = reg.list_models().len();

        let provider = Arc::new(openai::OpenAiProvider::new(
            secret("test-key"),
            "test-model".into(),
            "https://example.com".into(),
        ));
        reg.register(
            ModelInfo {
                id: "test-model".into(),
                provider: "test".into(),
                display_name: "Test Model".into(),
                created_at: None,
            },
            provider,
        );

        assert_eq!(reg.list_models().len(), initial_count + 1);
        assert!(reg.get("test-model").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[cfg(feature = "provider-openai-codex")]
    #[test]
    fn refresh_openai_codex_models_is_noop_when_disabled() {
        let mut reg = ProviderRegistry {
            providers: HashMap::new(),
            models: Vec::new(),
        };
        let provider = Arc::new(openai::OpenAiProvider::new_with_name(
            secret("k"),
            "gpt-5.2-codex".into(),
            "https://example.com/v1".into(),
            "openai-codex".into(),
        ));
        reg.register(
            ModelInfo {
                id: "gpt-5.2-codex".into(),
                provider: "openai-codex".into(),
                display_name: "GPT-5.2 Codex (Codex/OAuth)".into(),
                created_at: None,
            },
            provider,
        );

        let mut config = ProvidersConfig::default();
        config.providers.insert(
            "openai-codex".into(),
            moltis_config::schema::ProviderEntry {
                enabled: false,
                ..Default::default()
            },
        );

        let refreshed = reg.refresh_openai_codex_models(&config);
        assert!(!refreshed);
        assert!(
            reg.list_models()
                .iter()
                .any(|m| m.provider == "openai-codex")
        );
    }

    #[test]
    fn mistral_registers_with_api_key() {
        let mut config = ProvidersConfig::default();
        config
            .providers
            .insert("mistral".into(), moltis_config::schema::ProviderEntry {
                api_key: Some(secrecy::Secret::new("sk-test-mistral".into())),
                ..Default::default()
            });

        let reg = ProviderRegistry::from_env_with_config(&config);
        // Should have registered Mistral models
        let mistral_models: Vec<_> = reg
            .list_models()
            .iter()
            .filter(|m| m.provider == "mistral")
            .collect();
        assert!(
            !mistral_models.is_empty(),
            "expected Mistral models to be registered"
        );
        for m in &mistral_models {
            assert!(reg.get(&m.id).is_some());
            assert_eq!(reg.get(&m.id).unwrap().name(), "mistral");
        }
    }

    #[test]
    fn cerebras_registers_with_api_key() {
        let mut config = ProvidersConfig::default();
        config
            .providers
            .insert("cerebras".into(), moltis_config::schema::ProviderEntry {
                api_key: Some(secrecy::Secret::new("sk-test-cerebras".into())),
                ..Default::default()
            });

        let reg = ProviderRegistry::from_env_with_config(&config);
        let cerebras_models: Vec<_> = reg
            .list_models()
            .iter()
            .filter(|m| m.provider == "cerebras")
            .collect();
        assert!(!cerebras_models.is_empty());
    }

    #[test]
    fn minimax_registers_with_api_key() {
        let mut config = ProvidersConfig::default();
        config
            .providers
            .insert("minimax".into(), moltis_config::schema::ProviderEntry {
                api_key: Some(secrecy::Secret::new("sk-test-minimax".into())),
                ..Default::default()
            });

        let reg = ProviderRegistry::from_env_with_config(&config);
        assert!(reg.list_models().iter().any(|m| m.provider == "minimax"));
    }

    #[test]
    fn moonshot_registers_with_api_key() {
        let mut config = ProvidersConfig::default();
        config
            .providers
            .insert("moonshot".into(), moltis_config::schema::ProviderEntry {
                api_key: Some(secrecy::Secret::new("sk-test-moonshot".into())),
                ..Default::default()
            });

        let reg = ProviderRegistry::from_env_with_config(&config);
        assert!(reg.list_models().iter().any(|m| m.provider == "moonshot"));
    }

    #[test]
    fn openrouter_requires_model_in_config() {
        // OpenRouter has no default models — without configured models it registers nothing.
        let mut config = ProvidersConfig::default();
        config
            .providers
            .insert("openrouter".into(), moltis_config::schema::ProviderEntry {
                api_key: Some(secrecy::Secret::new("sk-test-or".into())),
                ..Default::default()
            });

        let reg = ProviderRegistry::from_env_with_config(&config);
        assert!(!reg.list_models().iter().any(|m| m.provider == "openrouter"));
    }

    #[test]
    fn openrouter_registers_with_model_in_config() {
        let mut config = ProvidersConfig::default();
        config
            .providers
            .insert("openrouter".into(), moltis_config::schema::ProviderEntry {
                api_key: Some(secrecy::Secret::new("sk-test-or".into())),
                models: vec!["anthropic/claude-3-haiku".into()],
                ..Default::default()
            });

        let reg = ProviderRegistry::from_env_with_config(&config);
        let or_models: Vec<_> = reg
            .list_models()
            .iter()
            .filter(|m| m.provider == "openrouter")
            .collect();
        assert_eq!(or_models.len(), 1);
        assert_eq!(or_models[0].id, "openrouter::anthropic/claude-3-haiku");
    }

    #[test]
    fn ollama_registers_without_api_key_env() {
        // Ollama should use a dummy key if no env var is set.
        let mut config = ProvidersConfig::default();
        config
            .providers
            .insert("ollama".into(), moltis_config::schema::ProviderEntry {
                models: vec!["llama3".into()],
                ..Default::default()
            });

        let reg = ProviderRegistry::from_env_with_config(&config);
        assert!(reg.list_models().iter().any(|m| m.provider == "ollama"));
        assert!(reg.get("llama3").is_some());
    }

    #[test]
    fn venice_requires_model_in_config() {
        let mut config = ProvidersConfig::default();
        config
            .providers
            .insert("venice".into(), moltis_config::schema::ProviderEntry {
                api_key: Some(secrecy::Secret::new("sk-test-venice".into())),
                ..Default::default()
            });

        let reg = ProviderRegistry::from_env_with_config(&config);
        assert!(!reg.list_models().iter().any(|m| m.provider == "venice"));
    }

    #[test]
    fn disabled_provider_not_registered() {
        let mut config = ProvidersConfig::default();
        config
            .providers
            .insert("mistral".into(), moltis_config::schema::ProviderEntry {
                api_key: Some(secrecy::Secret::new("sk-test".into())),
                enabled: false,
                ..Default::default()
            });

        let reg = ProviderRegistry::from_env_with_config(&config);
        assert!(!reg.list_models().iter().any(|m| m.provider == "mistral"));
    }

    #[test]
    fn provider_name_returned_by_openai_provider() {
        let provider = openai::OpenAiProvider::new_with_name(
            secret("k"),
            "m".into(),
            "u".into(),
            "mistral".into(),
        );
        assert_eq!(provider.name(), "mistral");
    }

    #[test]
    fn custom_base_url_from_config() {
        let mut config = ProvidersConfig::default();
        config
            .providers
            .insert("mistral".into(), moltis_config::schema::ProviderEntry {
                api_key: Some(secrecy::Secret::new("sk-test".into())),
                base_url: Some("https://custom.mistral.example.com/v1".into()),
                ..Default::default()
            });

        let reg = ProviderRegistry::from_env_with_config(&config);
        assert!(reg.list_models().iter().any(|m| m.provider == "mistral"));
    }

    #[test]
    fn provider_models_can_disable_fetch_and_pin_single_model() {
        let mut config = ProvidersConfig::default();
        config
            .providers
            .insert("mistral".into(), moltis_config::schema::ProviderEntry {
                api_key: Some(secrecy::Secret::new("sk-test".into())),
                models: vec!["mistral-small-latest".into()],
                fetch_models: false,
                ..Default::default()
            });

        let reg = ProviderRegistry::from_env_with_config(&config);
        let mistral_models: Vec<_> = reg
            .list_models()
            .iter()
            .filter(|m| m.provider == "mistral")
            .collect();
        // With fetch disabled, only pinned models should be registered.
        assert_eq!(mistral_models.len(), 1);
        assert_eq!(mistral_models[0].id, "mistral::mistral-small-latest");
    }

    #[test]
    fn provider_models_are_ordered_before_discovered_catalog() {
        let mut config = ProvidersConfig::default();
        config
            .providers
            .insert("mistral".into(), moltis_config::schema::ProviderEntry {
                api_key: Some(secrecy::Secret::new("sk-test".into())),
                models: vec!["codestral-latest".into()],
                ..Default::default()
            });

        let reg = ProviderRegistry::from_env_with_config(&config);
        let mistral_models: Vec<&str> = reg
            .list_models()
            .iter()
            .filter(|m| m.provider == "mistral")
            .map(|m| m.id.as_str())
            .collect();
        assert!(!mistral_models.is_empty());
        assert_eq!(mistral_models[0], "mistral::codestral-latest");
    }

    #[test]
    fn fallback_providers_ordering() {
        // Build a registry with:
        // - gpt-4o on "openai"
        // - gpt-4o on "openrouter" (same model, different provider)
        // - claude-sonnet on "anthropic" (different model, different provider)
        // - gpt-4o-mini on "openai" (different model, same provider)
        let mut reg = ProviderRegistry {
            providers: HashMap::new(),
            models: Vec::new(),
        };

        // Register in arbitrary order.
        let mk = |id: &str, prov: &str| {
            (
                ModelInfo {
                    id: id.into(),
                    provider: prov.into(),
                    display_name: id.into(),
                    created_at: None,
                },
                Arc::new(openai::OpenAiProvider::new_with_name(
                    secret("k"),
                    id.into(),
                    "u".into(),
                    prov.into(),
                )) as Arc<dyn LlmProvider>,
            )
        };

        let (info, prov) = mk("gpt-4o", "openai");
        reg.register(info, prov);
        let (info, prov) = mk("gpt-4o-mini", "openai");
        reg.register(info, prov);
        let (info, prov) = mk("claude-sonnet", "anthropic");
        reg.register(info, prov);
        // Simulate same model on different provider (openrouter).
        // The registry key is model_id so we need a distinct key; use a composite.
        // In practice the registry is keyed by model ID, so same model from
        // different provider would need a different registration approach.
        // For this test, use a unique key but same model info pattern.
        let provider_or = Arc::new(openai::OpenAiProvider::new_with_name(
            secret("k"),
            "gpt-4o".into(),
            "u".into(),
            "openrouter".into(),
        ));
        // We can't register same model ID twice, so test the ordering
        // with what we have: primary is gpt-4o/openai.
        let fallbacks = reg.fallback_providers_for("openai::gpt-4o", "openai");
        let ids: Vec<&str> = fallbacks.iter().map(|p| p.id()).collect();

        // gpt-4o-mini (same provider) should come before claude-sonnet (other provider).
        assert_eq!(ids, vec!["openai::gpt-4o-mini", "anthropic::claude-sonnet"]);

        // Now test with primary being claude-sonnet/anthropic — both openai models should follow.
        let fallbacks = reg.fallback_providers_for("anthropic::claude-sonnet", "anthropic");
        let ids: Vec<&str> = fallbacks.iter().map(|p| p.id()).collect();
        assert_eq!(ids, vec!["openai::gpt-4o", "openai::gpt-4o-mini"]);

        // Verify we don't use the openrouter provider we created (not registered).
        drop(provider_or);
    }

    #[cfg(feature = "local-llm")]
    #[test]
    fn local_llm_requires_model_in_config() {
        // local-llm is a "bring your own model" provider — without configured models it registers nothing.
        let mut config = ProvidersConfig::default();
        config
            .providers
            .insert("local".into(), moltis_config::schema::ProviderEntry {
                ..Default::default()
            });

        let reg = ProviderRegistry::from_env_with_config(&config);
        assert!(!reg.list_models().iter().any(|m| m.provider == "local-llm"));
    }

    #[cfg(feature = "local-llm")]
    #[test]
    fn local_llm_registers_with_model_in_config() {
        let mut config = ProvidersConfig::default();
        config
            .providers
            .insert("local".into(), moltis_config::schema::ProviderEntry {
                models: vec!["qwen2.5-coder-7b-q4_k_m".into()],
                ..Default::default()
            });

        let reg = ProviderRegistry::from_env_with_config(&config);
        let local_models: Vec<_> = reg
            .list_models()
            .iter()
            .filter(|m| m.provider == "local-llm")
            .collect();
        assert_eq!(local_models.len(), 1);
        assert_eq!(local_models[0].id, "local-llm::qwen2.5-coder-7b-q4_k_m");
    }

    #[cfg(feature = "local-llm")]
    #[test]
    fn local_llm_disabled_not_registered() {
        let mut config = ProvidersConfig::default();
        config
            .providers
            .insert("local".into(), moltis_config::schema::ProviderEntry {
                enabled: false,
                models: vec!["qwen2.5-coder-7b-q4_k_m".into()],
                ..Default::default()
            });

        let reg = ProviderRegistry::from_env_with_config(&config);
        assert!(!reg.list_models().iter().any(|m| m.provider == "local-llm"));
    }

    #[cfg(feature = "local-llm")]
    #[test]
    fn local_llm_alias_key_registers_model() {
        let mut config = ProvidersConfig::default();
        config
            .providers
            .insert("local-llm".into(), moltis_config::schema::ProviderEntry {
                models: vec!["qwen2.5-coder-7b-q4_k_m".into()],
                ..Default::default()
            });

        let reg = ProviderRegistry::from_env_with_config(&config);
        assert!(
            reg.list_models().iter().any(|m| m.provider == "local-llm"),
            "local-llm alias config key should register local models"
        );
    }

    #[cfg(feature = "local-llm")]
    #[test]
    fn local_llm_alias_key_respects_disabled_flag() {
        let mut config = ProvidersConfig::default();
        config
            .providers
            .insert("local-llm".into(), moltis_config::schema::ProviderEntry {
                enabled: false,
                models: vec!["qwen2.5-coder-7b-q4_k_m".into()],
                ..Default::default()
            });

        let reg = ProviderRegistry::from_env_with_config(&config);
        assert!(
            !reg.list_models().iter().any(|m| m.provider == "local-llm"),
            "disabled local-llm alias config should suppress local model registration"
        );
    }

    // ── Vision Support Tests (Extended) ────────────────────────────────

    #[test]
    fn supports_vision_for_all_claude_variants() {
        // All Claude model variants should support vision
        let claude_models = [
            "claude-3-opus-20240229",
            "claude-3-sonnet-20240229",
            "claude-3-haiku-20240307",
            "claude-sonnet-4-20250514",
            "claude-opus-4-20250514",
            "claude-opus-4-5-20251101",
            "claude-sonnet-4-5-20250929",
            "claude-haiku-4-5-20251001",
            "claude-3-7-sonnet-20250219",
        ];
        for model in claude_models {
            assert!(
                super::supports_vision_for_model(model),
                "expected {} to support vision",
                model
            );
        }
    }

    #[test]
    fn supports_vision_for_all_gpt4o_variants() {
        // All GPT-4o variants should support vision
        let gpt4o_models = [
            "gpt-4o",
            "gpt-4o-mini",
            "gpt-4o-2024-05-13",
            "gpt-4o-2024-08-06",
            "gpt-4o-audio-preview",
            "gpt-4o-mini-2024-07-18",
        ];
        for model in gpt4o_models {
            assert!(
                super::supports_vision_for_model(model),
                "expected {} to support vision",
                model
            );
        }
    }

    #[test]
    fn supports_vision_for_gpt5_series() {
        // GPT-5 series (including Codex variants) should support vision
        let gpt5_models = [
            "gpt-5",
            "gpt-5-turbo",
            "gpt-5.2-codex",
            "gpt-5.2",
            "gpt-5-preview",
        ];
        for model in gpt5_models {
            assert!(
                super::supports_vision_for_model(model),
                "expected {} to support vision",
                model
            );
        }
    }

    #[test]
    fn supports_vision_for_o3_o4_series() {
        // o3 and o4 reasoning models should support vision
        let reasoning_models = ["o3", "o3-mini", "o3-preview", "o4", "o4-mini", "o4-preview"];
        for model in reasoning_models {
            assert!(
                super::supports_vision_for_model(model),
                "expected {} to support vision",
                model
            );
        }
    }

    #[test]
    fn supports_vision_for_gemini_variants() {
        // All Gemini model variants should support vision
        let gemini_models = [
            "gemini-1.0-pro-vision",
            "gemini-1.5-pro",
            "gemini-1.5-flash",
            "gemini-2.0-flash",
            "gemini-2.0-pro",
            "gemini-ultra",
        ];
        for model in gemini_models {
            assert!(
                super::supports_vision_for_model(model),
                "expected {} to support vision",
                model
            );
        }
    }

    #[test]
    fn no_vision_for_text_only_models() {
        // Models known to NOT support vision
        let text_only_models = [
            "codestral-latest",
            "mistral-large-latest",
            "mistral-small-latest",
            "mistral-7b",
            "kimi-k2.5",
            "llama-4-scout-17b-16e-instruct",
            "MiniMax-M2.1",
            "gpt-3.5-turbo", // old model without vision
            "text-davinci-003",
        ];
        for model in text_only_models {
            assert!(
                !super::supports_vision_for_model(model),
                "expected {} to NOT support vision",
                model
            );
        }
    }

    #[test]
    fn vision_support_is_case_sensitive() {
        // Model IDs are case-sensitive - uppercase should not match
        assert!(!super::supports_vision_for_model("CLAUDE-SONNET-4"));
        assert!(!super::supports_vision_for_model("GPT-4O"));
        assert!(!super::supports_vision_for_model("Gemini-2.0-flash"));
    }

    #[test]
    fn vision_support_requires_exact_prefix() {
        // Vision support is based on prefix matching - partial matches shouldn't work
        assert!(!super::supports_vision_for_model("my-claude-model"));
        assert!(!super::supports_vision_for_model("custom-gpt-4o-wrapper"));
        assert!(!super::supports_vision_for_model("not-gemini-model"));
    }
}
