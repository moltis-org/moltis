use {
    super::*,
    serde::{Deserialize, Serialize},
    std::collections::HashMap,
};

/// Authentication configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AuthConfig {
    /// When true, authentication is explicitly disabled (no login required).
    pub disabled: bool,
}

/// Runtime GraphQL server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphqlConfig {
    /// Whether GraphQL HTTP/WS handlers accept requests.
    pub enabled: bool,
}

impl Default for GraphqlConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Metrics and observability configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MetricsConfig {
    /// Whether metrics collection is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Whether to expose the `/metrics` Prometheus endpoint.
    #[serde(default = "default_true")]
    pub prometheus_endpoint: bool,
    /// Maximum number of in-memory history points for time-series charts.
    /// Points are sampled every 30 seconds. Defaults to 360 (3 hours).
    /// Historical data is persisted to SQLite regardless of this setting.
    #[serde(default = "default_metrics_history_points")]
    pub history_points: usize,
    /// Additional labels to add to all metrics.
    #[serde(default)]
    pub labels: HashMap<String, String>,
}

fn default_metrics_history_points() -> usize {
    360
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            prometheus_endpoint: true,
            history_points: default_metrics_history_points(),
            labels: HashMap::new(),
        }
    }
}

/// Skills configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SkillsConfig {
    /// Whether the skills system is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Extra directories to search for skills.
    #[serde(default)]
    pub search_paths: Vec<String>,
    /// Skills to always load (by name) without explicit activation.
    #[serde(default)]
    pub auto_load: Vec<String>,
    /// Whether agents may write supplementary files inside personal skill directories.
    #[serde(default)]
    pub enable_agent_sidecar_files: bool,
    /// Inject system-prompt guidance encouraging the agent to autonomously create
    /// and update skills after complex tasks. Default: true.
    #[serde(default = "default_true")]
    pub enable_self_improvement: bool,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            search_paths: Vec::new(),
            auto_load: Vec::new(),
            enable_agent_sidecar_files: false,
            enable_self_improvement: true,
        }
    }
}

/// MCP (Model Context Protocol) server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct McpConfig {
    /// Default timeout for MCP requests in seconds.
    #[serde(default = "default_mcp_request_timeout_secs")]
    pub request_timeout_secs: u64,
    /// Configured MCP servers, keyed by server name.
    #[serde(default)]
    pub servers: HashMap<String, McpServerEntry>,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            request_timeout_secs: default_mcp_request_timeout_secs(),
            servers: HashMap::new(),
        }
    }
}

fn default_mcp_request_timeout_secs() -> u64 {
    30
}

/// Configuration for a single MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerEntry {
    /// Command to spawn the server process (stdio transport).
    #[serde(default)]
    pub command: String,
    /// Arguments to the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables to set for the process.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Whether this server is enabled. Defaults to true.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Optional per-server MCP request timeout override in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_timeout_secs: Option<u64>,
    /// Transport type: "stdio" (default), "sse", or "streamable-http".
    #[serde(default)]
    pub transport: String,
    /// URL for SSE/Streamable HTTP transport. Required when `transport` is "sse" or "streamable-http".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Custom headers for remote HTTP/SSE transport.
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Manual OAuth override for servers that don't support standard discovery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth: Option<McpOAuthOverrideEntry>,
    /// Custom display name for the server (shown in UI instead of technical ID).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// Manual OAuth configuration override for an MCP server.
///
/// Used when the server doesn't implement RFC 9728/8414 discovery or
/// when dynamic client registration is not available.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpOAuthOverrideEntry {
    /// The OAuth client ID.
    pub client_id: String,
    /// The authorization endpoint URL.
    pub auth_url: String,
    /// The token endpoint URL.
    pub token_url: String,
    /// OAuth scopes to request.
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// Built-in channel type identifiers recognised by the validator.
///
/// Kept in `moltis-config` (not `moltis-channels`) so the config crate stays
/// independent of the channels crate while still validating channel names.
pub const KNOWN_CHANNEL_TYPES: &[&str] = &[
    "telegram", "whatsapp", "msteams", "discord", "slack", "matrix",
];

/// Per-chat-type tool policy for a channel account.
///
/// Keyed by chat type (e.g. `"private"`, `"group"`, `"channel"`).
/// Each entry can have an allow/deny policy and per-sender overrides.
///
/// Example TOML:
/// ```toml
/// [channels.telegram.my-bot.tools.groups.group]
/// deny = ["exec"]
///
/// [channels.telegram.my-bot.tools.groups.group.by_sender]
/// "123456" = { allow = ["*"], deny = [] }
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupToolPolicy {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    /// Per-sender overrides within this group, keyed by sender/peer ID.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub by_sender: HashMap<String, ToolPolicyConfig>,
}

/// Tool policy overrides for a channel account.
///
/// Lives at `channels.<type>.<account_id>.tools` in the config.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelToolPolicyOverride {
    /// Per-chat-type policies, keyed by chat type (`"private"`, `"group"`, etc.).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub groups: HashMap<String, GroupToolPolicy>,
}

/// Channel configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelsConfig {
    /// Which channel types are offered in the web UI (onboarding + channels page).
    /// Defaults to `["telegram", "whatsapp", "msteams", "discord", "slack", "matrix", "nostr"]`.
    #[serde(
        default = "default_channels_offered",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub offered: Vec<String>,
    /// Telegram bot accounts, keyed by account ID.
    #[serde(default)]
    pub telegram: HashMap<String, serde_json::Value>,
    /// WhatsApp linked-device accounts, keyed by account ID.
    #[serde(default)]
    pub whatsapp: HashMap<String, serde_json::Value>,
    /// Microsoft Teams bot accounts, keyed by account ID.
    #[serde(default)]
    pub msteams: HashMap<String, serde_json::Value>,
    /// Discord bot accounts, keyed by account ID.
    #[serde(default)]
    pub discord: HashMap<String, serde_json::Value>,
    /// Slack bot accounts, keyed by account ID.
    #[serde(default)]
    pub slack: HashMap<String, serde_json::Value>,
    /// Nostr DM accounts, keyed by account ID.
    #[serde(default)]
    pub nostr: HashMap<String, serde_json::Value>,
    /// Additional channel types not covered by the named fields above.
    ///
    /// This allows new channel plugins to be configured without changing
    /// this struct.
    #[serde(flatten, default)]
    pub extra: HashMap<String, HashMap<String, serde_json::Value>>,
}

impl ChannelsConfig {
    /// All named channel fields as `(channel_type, accounts)` pairs.
    ///
    /// This is the single source of truth for the set of named channel types.
    /// Keep in sync with the struct fields.
    fn named_fields(&self) -> [(&str, &HashMap<String, serde_json::Value>); 6] {
        [
            ("telegram", &self.telegram),
            ("whatsapp", &self.whatsapp),
            ("msteams", &self.msteams),
            ("discord", &self.discord),
            ("slack", &self.slack),
            ("nostr", &self.nostr),
        ]
    }

    /// Iterate all channel configs (named + extra) as `(channel_type, accounts)` pairs.
    pub fn all_channel_configs(&self) -> Vec<(&str, &HashMap<String, serde_json::Value>)> {
        let mut v: Vec<(&str, &HashMap<String, serde_json::Value>)> =
            self.named_fields().into_iter().collect();
        for (ct, accounts) in &self.extra {
            v.push((ct.as_str(), accounts));
        }
        v
    }

    /// Extract the `tools` sub-object for a specific channel account.
    ///
    /// Channel accounts are stored as `serde_json::Value`, so we deserialize
    /// just the `tools` key on demand.
    pub fn tool_policy_for_account(
        &self,
        channel_type: &str,
        account_id: &str,
    ) -> Option<ChannelToolPolicyOverride> {
        let accounts = self
            .all_channel_configs()
            .into_iter()
            .find(|(ct, _)| *ct == channel_type)
            .map(|(_, accounts)| accounts)?;
        let account_val = accounts.get(account_id)?;
        let tools_val = account_val.get("tools")?;
        serde_json::from_value::<ChannelToolPolicyOverride>(tools_val.clone()).ok()
    }
}

fn default_channels_offered() -> Vec<String> {
    vec![
        "telegram".into(),
        "whatsapp".into(),
        "msteams".into(),
        "discord".into(),
        "slack".into(),
        "matrix".into(),
        "nostr".into(),
    ]
}

impl Default for ChannelsConfig {
    fn default() -> Self {
        Self {
            offered: default_channels_offered(),
            telegram: HashMap::new(),
            whatsapp: HashMap::new(),
            msteams: HashMap::new(),
            discord: HashMap::new(),
            slack: HashMap::new(),
            nostr: HashMap::new(),
            extra: HashMap::new(),
        }
    }
}

/// TLS configuration for the gateway HTTPS server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TlsConfig {
    /// Enable HTTPS with auto-generated certificates. Defaults to true.
    pub enabled: bool,
    /// Auto-generate a local CA and server certificate on first run.
    pub auto_generate: bool,
    /// Path to a custom server certificate (PEM). Overrides auto-generation.
    pub cert_path: Option<String>,
    /// Path to a custom server private key (PEM). Overrides auto-generation.
    pub key_path: Option<String>,
    /// Path to the CA certificate (PEM) used for trust instructions.
    pub ca_cert_path: Option<String>,
    /// Port for the plain-HTTP redirect/CA-download server.
    /// Defaults to the gateway port + 1 when not set.
    pub http_redirect_port: Option<u16>,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_generate: true,
            cert_path: None,
            key_path: None,
            ca_cert_path: None,
            http_redirect_port: None,
        }
    }
}
