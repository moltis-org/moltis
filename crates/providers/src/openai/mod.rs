mod catalog;
pub mod provider;

pub use {
    crate::DiscoveredModel,
    catalog::{
        available_models, default_model_catalog, fetch_models_from_api, live_models,
        start_model_discovery,
    },
};

use {crate::ModelCapabilities, moltis_agents::model::ModelMetadata};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RateLimitPolicy {
    None,
    Mistral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReasoningEffortPolicy {
    OpenAi,
    DeepSeek,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CacheControlPolicy {
    None,
    OpenRouterAnthropic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProbeFallbackPolicy {
    None,
    OllamaNativeShow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResponsesWebSocketPolicy {
    Unsupported,
    OpenAiPlatform,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OpenAiProviderCapabilities {
    pub(crate) supports_user_name: bool,
    pub(crate) default_strict_tools: bool,
    pub(crate) omits_strict_tool_field: bool,
    pub(crate) rejects_null_in_enums: bool,
    pub(crate) requires_gemini_tool_call_extra_content: bool,
    pub(crate) rate_limit_policy: RateLimitPolicy,
    pub(crate) reasoning_effort_policy: ReasoningEffortPolicy,
    pub(crate) cache_control_policy: CacheControlPolicy,
    pub(crate) probe_fallback_policy: ProbeFallbackPolicy,
    pub(crate) responses_websocket_policy: ResponsesWebSocketPolicy,
}

impl OpenAiProviderCapabilities {
    pub(crate) const DEFAULT: Self = Self {
        supports_user_name: true,
        default_strict_tools: true,
        omits_strict_tool_field: false,
        rejects_null_in_enums: false,
        requires_gemini_tool_call_extra_content: false,
        rate_limit_policy: RateLimitPolicy::None,
        reasoning_effort_policy: ReasoningEffortPolicy::OpenAi,
        cache_control_policy: CacheControlPolicy::None,
        probe_fallback_policy: ProbeFallbackPolicy::None,
        responses_websocket_policy: ResponsesWebSocketPolicy::Unsupported,
    };
}

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
    /// Explicit provider behavior policies. Never inferred from provider name or URL.
    capabilities: OpenAiProviderCapabilities,
    /// Resolved model capabilities. Never inferred from provider name or URL.
    model_capabilities: ModelCapabilities,
    /// Whether assistant tool-call messages need `reasoning_content` on replay.
    default_reasoning_content_on_tool_messages: bool,
    /// Raw model-id prefixes that need `reasoning_content` on tool-call replay.
    reasoning_content_model_prefixes: &'static [&'static str],
    /// Provider-specific system-message rewrite behavior.
    system_message_rewrite_strategy: SystemMessageRewriteStrategy,
    /// Whether Qwen-family models on this provider need one leading system message.
    qwen_models_require_single_leading_system: bool,
    /// Global per-model context window overrides from `[models.<id>]` config.
    context_window_global: std::collections::HashMap<String, u32>,
    /// Provider-scoped per-model context window overrides from
    /// `[providers.<name>.model_overrides.<id>]` config.
    context_window_provider: std::collections::HashMap<String, u32>,
    /// Optional override for the completion-based probe timeout (seconds).
    /// `None` uses the trait default (30s).
    probe_timeout_secs: Option<u64>,
}
