use {
    super::*,
    secrecy::Secret,
    serde::{Deserialize, Serialize},
    std::collections::HashMap,
};

/// OAuth provider configuration (e.g. openai-codex).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthProviderConfig {
    pub client_id: String,
    pub auth_url: String,
    pub token_url: String,
    pub redirect_uri: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub callback_port: u16,
}

/// Override configuration for a specific model.
///
/// Used in both `[models.<id>]` (global) and `[providers.<name>.model_overrides.<id>]`
/// (provider-scoped) sections of `moltis.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelOverride {
    /// Override the context window size (in tokens) for this model.
    ///
    /// When set, this value takes precedence over the built-in heuristic.
    /// Must be between 1 and 10,000,000 (inclusive).
    ///
    /// Provider-scoped overrides (`[providers.<name>.model_overrides.<id>]`)
    /// take precedence over global overrides (`[models.<id>]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
}

/// LLM provider configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProvidersConfig {
    /// Optional allowlist of enabled providers. This also controls which
    /// providers are offered in web UI pickers (onboarding and "add provider"
    /// modal). Empty means all providers are enabled.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub offered: Vec<String>,

    /// Show models older than one year in the chat model selector.
    /// By default only recent models are shown; legacy models remain
    /// accessible in the settings page regardless of this flag.
    #[serde(default, skip_serializing_if = "is_false")]
    pub show_legacy_models: bool,

    /// Provider-specific settings keyed by provider name.
    /// Known keys: "anthropic", "openai", "gemini", "groq", "xai", "deepseek"
    #[serde(flatten)]
    pub providers: HashMap<String, ProviderEntry>,

    /// Additional local model IDs to register (from local-llm.json).
    /// This is populated at runtime by the gateway and not persisted.
    #[serde(skip)]
    pub local_models: Vec<String>,
}

/// How tool calling is handled for a provider.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolMode {
    /// Detect automatically: native tool API if supported, else text-based fallback.
    #[default]
    Auto,
    /// Force native tool calling API (provider must support it).
    Native,
    /// Force text-based tool calling (prompt injection + parse).
    Text,
    /// Disable all tool support for this provider.
    Off,
}

const fn is_default_tool_mode(v: &ToolMode) -> bool {
    matches!(v, ToolMode::Auto)
}

const fn is_default_cache_retention(v: &CacheRetention) -> bool {
    matches!(v, CacheRetention::Short)
}

/// Wire format for provider HTTP API.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WireApi {
    /// Standard OpenAI Chat Completions format (`/chat/completions`).
    #[default]
    ChatCompletions,
    /// OpenAI Responses API format (`/responses`).
    Responses,
}

/// Prompt cache retention policy for providers that support client-controlled
/// caching (Anthropic direct, Anthropic via OpenRouter/Bedrock).
///
/// - `none`: disable prompt caching (no `cache_control` breakpoints sent).
/// - `short` (default for Anthropic): 5-minute ephemeral cache.
/// - `long`: same as `short` today (Anthropic only supports ephemeral), but
///   signals intent for longer retention when providers add TTL tiers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheRetention {
    /// No prompt caching — skip `cache_control` breakpoints entirely.
    None,
    /// Short-lived ephemeral cache (5 min TTL on Anthropic). Default for Anthropic.
    #[default]
    Short,
    /// Long-lived cache. Currently equivalent to `short` (ephemeral), but
    /// reserved for future provider support of extended TTL tiers.
    Long,
}

/// Streaming transport for provider response streams.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderStreamTransport {
    /// Use HTTP + SSE streaming (current default).
    #[default]
    Sse,
    /// Use WebSocket mode when supported by the provider API.
    Websocket,
    /// Try WebSocket first, then fall back to SSE on transport/setup failure.
    Auto,
}

/// Configuration for a single LLM provider.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderEntry {
    /// Whether this provider is enabled. Defaults to true.
    pub enabled: bool,

    /// Override the API key (optional; env var still takes precedence if set).
    #[serde(
        default,
        serialize_with = "serialize_option_secret",
        skip_serializing_if = "Option::is_none"
    )]
    pub api_key: Option<Secret<String>>,

    /// Override the base URL.
    /// Accepts legacy `url` as an alias for compatibility.
    #[serde(alias = "url")]
    pub base_url: Option<String>,

    /// Preferred model IDs for this provider.
    /// These are shown first in model pickers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,

    /// Whether to fetch provider model catalogs dynamically when available.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub fetch_models: bool,

    /// Streaming transport for this provider (`sse`, `websocket`, `auto`).
    ///
    /// Defaults to `sse` for compatibility.
    #[serde(default, skip_serializing_if = "is_default_provider_stream_transport")]
    pub stream_transport: ProviderStreamTransport,

    /// Wire format for this provider (`chat-completions`, `responses`).
    ///
    /// - `chat-completions` (default): standard `/chat/completions` endpoint.
    /// - `responses`: OpenAI Responses API (`/responses`) format.
    #[serde(default, skip_serializing_if = "is_default_wire_api")]
    pub wire_api: WireApi,

    /// Optional alias for this provider instance.
    ///
    /// When set, this alias is used in metrics labels instead of the provider name.
    /// Useful when configuring multiple instances of the same provider type
    /// (e.g., "anthropic-work", "anthropic-personal").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,

    /// How tool calling is handled for this provider.
    ///
    /// - `auto` (default): use native tool API if the provider supports it,
    ///   otherwise fall back to text-based prompt injection.
    /// - `native`: force native tool calling.
    /// - `text`: force text-based tool calling.
    /// - `off`: disable all tools for this provider.
    #[serde(default, skip_serializing_if = "is_default_tool_mode")]
    pub tool_mode: ToolMode,

    /// Prompt cache retention policy.
    ///
    /// - `none`: disable prompt caching entirely.
    /// - `short` (default): ephemeral 5-minute cache (Anthropic).
    /// - `long`: same as `short` today, reserved for future extended TTL.
    ///
    /// Only affects providers that support client-controlled caching
    /// (Anthropic direct, Anthropic via OpenRouter). Has no effect on
    /// providers with automatic server-side caching (OpenAI, DeepSeek, Ollama).
    #[serde(default, skip_serializing_if = "is_default_cache_retention")]
    pub cache_retention: CacheRetention,

    /// Whether to use OpenAI strict mode for tool schemas.
    ///
    /// - `true`: force strict mode (`additionalProperties: false`, all properties
    ///   required, optional properties made nullable via array-form types).
    /// - `false`: skip strict-mode patching. Use this for providers that reject
    ///   array-form types like `["boolean", "null"]` (e.g. Google/Gemini).
    /// - unset (default): auto-detect based on provider. OpenRouter and Gemini
    ///   default to non-strict; all others default to strict.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict_tools: Option<bool>,

    /// Tool policy override for this provider. When set, these allow/deny
    /// rules are merged on top of the global `[tools.policy]` for requests
    /// routed through this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ToolPolicyConfig>,

    /// Per-model overrides for context window and other model-specific settings.
    ///
    /// Keys are normalized model IDs (as displayed in the chat UI).
    /// These take precedence over global `[models.<id>]` overrides.
    ///
    /// ```toml
    /// [providers.zai-code.models.glm-5-turbo]
    /// context_window = 200_000
    /// ```
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub model_overrides: HashMap<String, ModelOverride>,
}

impl std::fmt::Debug for ProviderEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderEntry")
            .field("enabled", &self.enabled)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("base_url", &self.base_url)
            .field("models", &self.models)
            .field("fetch_models", &self.fetch_models)
            .field("stream_transport", &self.stream_transport)
            .field("wire_api", &self.wire_api)
            .field("alias", &self.alias)
            .field("tool_mode", &self.tool_mode)
            .field("cache_retention", &self.cache_retention)
            .field("strict_tools", &self.strict_tools)
            .field("policy", &self.policy)
            .field("model_overrides", &self.model_overrides)
            .finish()
    }
}

impl Default for ProviderEntry {
    fn default() -> Self {
        Self {
            enabled: true,
            api_key: None,
            base_url: None,
            models: Vec::new(),
            fetch_models: true,
            stream_transport: ProviderStreamTransport::Sse,
            wire_api: WireApi::ChatCompletions,
            alias: None,
            tool_mode: ToolMode::Auto,
            cache_retention: CacheRetention::Short,
            strict_tools: None,
            policy: None,
            model_overrides: HashMap::new(),
        }
    }
}

impl ProvidersConfig {
    fn normalize_provider_name(value: &str) -> String {
        value.trim().to_ascii_lowercase()
    }

    fn provider_name_matches(left: &str, right: &str) -> bool {
        if left == right {
            return true;
        }
        matches!(
            (left, right),
            ("local", "local-llm") | ("local-llm", "local")
        )
    }

    fn is_offered(&self, name: &str) -> bool {
        if self.offered.is_empty() {
            return true;
        }
        let normalized = Self::normalize_provider_name(name);
        self.offered.iter().any(|entry| {
            let offered = Self::normalize_provider_name(entry);
            Self::provider_name_matches(&offered, &normalized)
        })
    }

    fn provider_entry(&self, name: &str) -> Option<&ProviderEntry> {
        match name {
            "local" => self
                .providers
                .get("local")
                .or_else(|| self.providers.get("local-llm")),
            "local-llm" => self
                .providers
                .get("local-llm")
                .or_else(|| self.providers.get("local")),
            _ => self.providers.get(name),
        }
    }

    /// Check if a provider is enabled (defaults to true if not configured).
    pub fn is_enabled(&self, name: &str) -> bool {
        if !self.is_offered(name) {
            return false;
        }
        self.provider_entry(name).is_none_or(|e| e.enabled)
    }

    /// Get the configured entry for a provider, if any.
    pub fn get(&self, name: &str) -> Option<&ProviderEntry> {
        self.provider_entry(name)
    }
}
