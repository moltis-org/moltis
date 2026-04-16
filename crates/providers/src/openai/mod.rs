mod catalog;
pub mod provider;

pub use {
    crate::DiscoveredModel,
    catalog::{
        available_models, default_model_catalog, fetch_models_from_api, live_models,
        start_model_discovery,
    },
};

use moltis_agents::model::ModelMetadata;

pub struct OpenAiProvider {
    api_key: secrecy::Secret<String>,
    model: String,
    base_url: String,
    provider_name: String,
    client: &'static reqwest::Client,
    stream_transport: moltis_config::schema::ProviderStreamTransport,
    wire_api: moltis_config::schema::WireApi,
    metadata_cache: tokio::sync::OnceCell<ModelMetadata>,
    tool_mode_override: Option<moltis_config::ToolMode>,
    /// Optional reasoning effort level for o-series models.
    reasoning_effort: Option<moltis_agents::model::ReasoningEffort>,
    /// Prompt cache retention policy (used for OpenRouter Anthropic passthrough).
    cache_retention: moltis_config::CacheRetention,
    /// Explicit override for strict tool schema mode. `None` = auto-detect.
    strict_tools_override: Option<bool>,
    /// Global per-model context window overrides from `[models.<id>]` config.
    context_window_global: std::collections::HashMap<String, u32>,
    /// Provider-scoped per-model context window overrides from
    /// `[providers.<name>.model_overrides.<id>]` config.
    context_window_provider: std::collections::HashMap<String, u32>,
}
