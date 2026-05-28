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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SystemMessageRewriteStrategy {
    None,
    MergeLeadingSystem,
    InlineIntoFirstUser,
}

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
    /// Explicit override for reasoning_content requirement. `None` = auto-detect.
    reasoning_content_override: Option<bool>,
    /// Default strict tool schema mode for this provider.
    default_strict_tools: bool,
    /// Whether assistant tool-call messages need `reasoning_content` on replay.
    default_reasoning_content_on_tool_messages: bool,
    /// Raw model-id prefixes that need `reasoning_content` on tool-call replay.
    reasoning_content_model_prefixes: &'static [&'static str],
    /// Whether this provider rejects `null` entries in JSON Schema enum arrays.
    rejects_null_in_enums: bool,
    /// Whether tool-call metadata should be nested as Gemini extra_content.
    requires_gemini_tool_call_extra_content: bool,
    /// Provider-specific system-message rewrite behavior.
    system_message_rewrite_strategy: SystemMessageRewriteStrategy,
    /// Whether Qwen-family models on this provider need one leading system message.
    qwen_models_require_single_leading_system: bool,
    /// Global per-model context window overrides from `[models.<id>]` config.
    context_window_global: std::collections::HashMap<String, u32>,
    /// Provider-scoped per-model context window overrides from
    /// `[providers.<name>.model_overrides.<id>]` config.
    context_window_provider: std::collections::HashMap<String, u32>,
    /// Whether this provider accepts the `name` field on user messages.
    ///
    /// OpenAI and most compatible providers accept it (with character
    /// restrictions handled by sanitization).  Mistral rejects it outright.
    /// Defaults to `true`; set to `false` via `with_supports_user_name(false)`.
    pub(crate) supports_user_name: bool,
    /// Optional override for the completion-based probe timeout (seconds).
    /// `None` uses the trait default (30s).
    probe_timeout_secs: Option<u64>,
}
