/// Config schema types (agents, channels, tools, session, gateway, plugins).
/// Corresponds to `src/config/types.ts` and `zod-schema.*.ts` in the TS codebase.
use std::collections::HashMap;

use {
    secrecy::{ExposeSecret, Secret},
    serde::{Deserialize, Deserializer, Serialize},
};

#[path = "schema/agents.rs"]
mod agents;
#[path = "schema/chat.rs"]
mod chat;
#[path = "schema/code_index.rs"]
mod code_index;
#[path = "schema/hooks.rs"]
mod hooks;
#[path = "schema/memory.rs"]
mod memory;
#[path = "schema/modes.rs"]
mod modes;
#[path = "schema/phone.rs"]
mod phone;
#[path = "schema/providers.rs"]
mod providers;
#[path = "schema/runtime.rs"]
mod runtime;
#[path = "schema/system.rs"]
mod system;
#[path = "schema/tools.rs"]
mod tools;
#[path = "schema/voice.rs"]
mod voice;

pub use {
    agents::*, chat::*, code_index::*, hooks::*, memory::*, modes::*, phone::*, providers::*,
    runtime::*, system::*, tools::*, voice::*,
};

// ── Reasoning effort ──────────────────────────────────────────────────────

/// Reasoning/thinking effort level for models that support extended thinking.
///
/// Maps to provider-specific parameters:
/// - **Anthropic**: `thinking.budget_tokens`
/// - **OpenAI**: `reasoning_effort` field on o-series / GPT-5 models
/// - **Gemini**: `thinkingBudget` or `thinkingLevel`
/// - **Other providers**: ignored if unsupported
///
/// Not all providers support every level. Providers map unsupported levels
/// to their nearest supported equivalent (e.g. `ExtraHigh` → `High` when
/// the provider caps at `High`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    Minimal,
    Low,
    Medium,
    High,
    /// Extra-high reasoning effort. Serializes as `"xhigh"`.
    #[serde(rename = "xhigh")]
    ExtraHigh,
}

impl ReasoningEffort {
    /// All levels in ascending order.
    pub const ALL: &[Self] = &[
        Self::Minimal,
        Self::Low,
        Self::Medium,
        Self::High,
        Self::ExtraHigh,
    ];

    /// Wire / serialization name (matches serde output).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::ExtraHigh => "xhigh",
        }
    }

    /// Human-readable label for UI display.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Minimal => "Minimal",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::ExtraHigh => "Extra High",
        }
    }
}

/// Agent identity (name, emoji, theme).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentIdentity {
    pub name: Option<String>,
    pub emoji: Option<String>,
    pub theme: Option<String>,
}

/// IANA timezone (e.g. `"Europe/Paris"`).
///
/// Wraps [`chrono_tz::Tz`] and (de)serialises as a plain string so it stays
/// compatible with the YAML frontmatter in `USER.md`.
#[derive(Debug, Clone)]
pub struct Timezone(pub chrono_tz::Tz);

#[derive(Debug, thiserror::Error)]
#[error("unknown IANA timezone: {value}")]
pub struct TimezoneParseError {
    value: String,
}

impl Timezone {
    /// The IANA name, e.g. `"Europe/Paris"`.
    #[must_use]
    pub fn name(&self) -> &str {
        self.0.name()
    }

    /// The inner [`chrono_tz::Tz`] value.
    #[must_use]
    pub fn tz(&self) -> chrono_tz::Tz {
        self.0
    }
}

impl std::fmt::Display for Timezone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.name())
    }
}

impl std::str::FromStr for Timezone {
    type Err = TimezoneParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<chrono_tz::Tz>()
            .map(Self)
            .map_err(|_| TimezoneParseError {
                value: s.to_string(),
            })
    }
}

impl From<chrono_tz::Tz> for Timezone {
    fn from(tz: chrono_tz::Tz) -> Self {
        Self(tz)
    }
}

impl Serialize for Timezone {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0.name())
    }
}

impl<'de> Deserialize<'de> for Timezone {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse::<Self>().map_err(serde::de::Error::custom)
    }
}

/// Geographic coordinates (WGS 84).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoLocation {
    pub latitude: f64,
    pub longitude: f64,
    /// Human-readable place name from reverse geocoding (e.g. "Noe Valley, San Francisco, CA").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    /// Unix epoch seconds when the location was last updated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
}

impl GeoLocation {
    /// Create a new `GeoLocation` stamped with the current time.
    pub fn now(latitude: f64, longitude: f64, place: Option<String>) -> Self {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        Self {
            latitude,
            longitude,
            place,
            updated_at: Some(ts),
        }
    }
}

impl std::fmt::Display for GeoLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(ref place) = self.place {
            write!(f, "{place}")?;
        } else {
            write!(f, "{},{}", self.latitude, self.longitude)?;
        }
        if let Some(ts) = self.updated_at {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            let age_secs = now.saturating_sub(ts);
            let age_str = if age_secs < 60 {
                "just now".to_string()
            } else if age_secs < 3600 {
                format!("{}m ago", age_secs / 60)
            } else if age_secs < 86400 {
                format!("{}h ago", age_secs / 3600)
            } else {
                format!("{}d ago", age_secs / 86400)
            };
            write!(f, " (updated {age_str})")?;
        }
        Ok(())
    }
}

/// User profile collected during onboarding.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UserProfile {
    pub name: Option<String>,
    pub timezone: Option<Timezone>,
    pub location: Option<GeoLocation>,
}

/// Resolved identity combining agent identity and user profile.
/// Used as the API response for `identity_get` and in the gon data blob.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedIdentity {
    pub name: String,
    pub emoji: Option<String>,
    pub theme: Option<String>,
    pub soul: Option<String>,
    pub user_name: Option<String>,
}

impl ResolvedIdentity {
    pub fn from_config(cfg: &MoltisConfig) -> Self {
        Self {
            name: cfg.identity.name.clone().unwrap_or_else(|| "moltis".into()),
            emoji: cfg.identity.emoji.clone(),
            theme: cfg.identity.theme.clone(),
            soul: None,
            user_name: cfg.user.name.clone(),
        }
    }
}

impl Default for ResolvedIdentity {
    fn default() -> Self {
        Self {
            name: "moltis".into(),
            emoji: None,
            theme: None,
            soul: None,
            user_name: None,
        }
    }
}

/// Root configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MoltisConfig {
    pub server: ServerConfig,
    pub ngrok: NgrokConfig,
    pub cloudflare_tunnel: CloudflareTunnelConfig,
    pub providers: ProvidersConfig,
    pub chat: ChatConfig,
    pub tools: ToolsConfig,
    pub agents: AgentsConfig,
    pub modes: ModesConfig,
    pub skills: SkillsConfig,
    pub mcp: McpConfig,
    pub channels: ChannelsConfig,
    pub tls: TlsConfig,
    pub auth: AuthConfig,
    pub graphql: GraphqlConfig,
    pub metrics: MetricsConfig,
    pub identity: AgentIdentity,
    pub user: UserProfile,
    pub hooks: Option<HooksConfig>,
    pub memory: MemoryEmbeddingConfig,
    pub tailscale: TailscaleConfig,
    pub netbird: NetbirdConfig,
    pub failover: FailoverConfig,
    pub heartbeat: HeartbeatConfig,
    pub voice: VoiceConfig,
    pub phone: PhoneConfig,
    pub cron: CronConfig,
    pub caldav: CalDavConfig,
    pub home_assistant: HomeAssistantConfig,
    pub webhooks: WebhooksConfig,
    /// Auxiliary model assignments for side tasks (compaction, titles, vision).
    pub auxiliary: AuxiliaryModelsConfig,
    /// Code-index configuration for codebase search tools.
    pub code_index: CodeIndexTomlConfig,
    /// Per-model overrides that apply across all providers.
    ///
    /// Keys are normalized model IDs. Provider-scoped overrides
    /// (`[providers.<name>.models.<id>]`) take precedence over these.
    ///
    /// ```toml
    /// [models.claude-opus-4-6]
    /// context_window = 1_000_000
    /// ```
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub models: HashMap<String, ModelOverride>,
    /// Upstream HTTP/SOCKS proxy for all outbound requests.
    ///
    /// Supports `http://`, `https://`, `socks5://`, and `socks5h://` schemes.
    /// Proxy authentication via URL: `http://user:pass@host:port`.
    /// When set, overrides the `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY` environment
    /// variables for all traffic (providers, channels, tools, OAuth).
    /// Localhost/loopback addresses are automatically excluded (`no_proxy`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::schema::serialize_option_secret",
        deserialize_with = "crate::schema::deserialize_option_secret"
    )]
    pub upstream_proxy: Option<Secret<String>>,
    /// Environment variables injected into the Moltis process at startup.
    /// Useful for API keys in Docker where you can't easily set env vars.
    /// Process env vars take precedence (existing vars are not overwritten).
    #[serde(default)]
    pub env: HashMap<String, String>,
}

impl MoltisConfig {
    /// Returns `true` when both the agent name and user name have been set
    /// (i.e. the onboarding wizard has been completed).
    pub fn is_onboarded(&self) -> bool {
        self.identity.name.is_some() && self.user.name.is_some()
    }
}

fn serialize_option_secret<S: serde::Serializer>(
    secret: &Option<Secret<String>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match secret {
        Some(s) => serializer.serialize_some(s.expose_secret()),
        None => serializer.serialize_none(),
    }
}

fn deserialize_option_secret<'de, D>(deserializer: D) -> Result<Option<Secret<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<String> = Option::deserialize(deserializer)?;
    Ok(opt.map(Secret::new))
}

fn default_true() -> bool {
    true
}

const fn is_true(value: &bool) -> bool {
    *value
}

const fn is_false(value: &bool) -> bool {
    !*value
}

const fn is_default_provider_stream_transport(value: &ProviderStreamTransport) -> bool {
    matches!(value, ProviderStreamTransport::Sse)
}

const fn is_default_wire_api(value: &WireApi) -> bool {
    matches!(value, WireApi::ChatCompletions)
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
#[path = "schema/tests.rs"]
mod tests;
