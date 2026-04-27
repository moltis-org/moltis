use {
    super::*,
    secrecy::Secret,
    serde::{Deserialize, Serialize},
};

/// Gateway server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// Address to bind to. Defaults to "127.0.0.1".
    pub bind: String,
    /// Port to listen on. When a new config is created, a random available port
    /// is generated so each installation gets a unique port.
    pub port: u16,
    /// Enable verbose Axum/Tower HTTP request logs (`http_request` spans).
    /// Useful for debugging redirects and request flow.
    pub http_request_logs: bool,
    /// Enable WebSocket request/response logs (`ws:` entries).
    /// Useful for debugging RPC calls from the web UI.
    pub ws_request_logs: bool,
    /// Maximum number of log entries kept in the in-memory ring buffer.
    /// Older entries are persisted to disk and available via the web UI.
    /// Defaults to 1000. Increase for busy servers, decrease for memory-constrained devices.
    #[serde(default = "default_log_buffer_size")]
    pub log_buffer_size: usize,
    /// URL of the releases manifest (`releases.json`) used by the update checker.
    ///
    /// Defaults to `https://www.moltis.org/releases.json` when unset.
    pub update_releases_url: Option<String>,
    /// Maximum number of SQLite pool connections. Lower values reduce memory
    /// usage for personal gateways. Defaults to 5.
    #[serde(default = "default_db_pool_max_connections")]
    pub db_pool_max_connections: u32,
    /// Base URL for the Shiki syntax-highlighting library loaded by the web UI.
    ///
    /// Defaults to `https://esm.sh/shiki@3.2.1?bundle` when unset.
    /// Set to an alternative CDN or a self-hosted URL to override.
    pub shiki_cdn_url: Option<String>,
    /// Enable or disable the host terminal in the web UI.
    ///
    /// Defaults to `true`. Set to `false` to prevent the web UI from
    /// offering an unsandboxed shell. The `MOLTIS_TERMINAL_DISABLED`
    /// environment variable (set to `1` or `true`) takes precedence over
    /// this field and cannot be changed from the web UI config editor.
    #[serde(default = "default_terminal_enabled")]
    pub terminal_enabled: bool,
    /// Public URL when running behind a reverse proxy (e.g.
    /// `https://moltis.example.com`). Used to derive the WebAuthn RP ID
    /// and origin so passkey auth works with the proxy's public hostname.
    ///
    /// The `MOLTIS_EXTERNAL_URL` environment variable takes precedence
    /// over this field.
    pub external_url: Option<String>,
}

fn default_log_buffer_size() -> usize {
    1000
}

fn default_db_pool_max_connections() -> u32 {
    5
}

fn default_terminal_enabled() -> bool {
    true
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".into(),
            port: 0, // Will be replaced with a random port when config is created
            http_request_logs: false,
            ws_request_logs: false,
            log_buffer_size: default_log_buffer_size(),
            update_releases_url: None,
            db_pool_max_connections: default_db_pool_max_connections(),
            shiki_cdn_url: None,
            terminal_enabled: default_terminal_enabled(),
            external_url: None,
        }
    }
}

impl ServerConfig {
    /// Returns whether the web UI terminal is enabled, accounting for the
    /// `MOLTIS_TERMINAL_DISABLED` env-var override. When the env var is set
    /// to `"1"` or `"true"` (case-insensitive), the terminal is disabled
    /// regardless of the config file value.
    pub fn is_terminal_enabled(&self) -> bool {
        if let Ok(val) = std::env::var("MOLTIS_TERMINAL_DISABLED")
            && (val.eq_ignore_ascii_case("true") || val == "1")
        {
            return false;
        }
        self.terminal_enabled
    }

    /// Returns the effective external URL, checking `MOLTIS_EXTERNAL_URL`
    /// env var first, then falling back to the config field. Trailing
    /// slashes are stripped so the result can be used directly as a
    /// WebAuthn origin.
    pub fn effective_external_url(&self) -> Option<String> {
        let env_val = std::env::var("MOLTIS_EXTERNAL_URL")
            .ok()
            .filter(|v| !v.is_empty());
        Self::resolve_external_url(env_val.as_deref(), self.external_url.as_deref())
    }

    /// Pure helper for [`effective_external_url`] — exposed for testing
    /// without mutating process env vars.
    pub fn resolve_external_url(env_val: Option<&str>, config_val: Option<&str>) -> Option<String> {
        let raw = env_val.or(config_val)?;
        Some(raw.trim_end_matches('/').to_string())
    }
}

/// ngrok public HTTPS tunnel configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NgrokConfig {
    /// Whether the ngrok tunnel is enabled.
    pub enabled: bool,
    /// ngrok authtoken. If unset, `NGROK_AUTHTOKEN` is used.
    #[serde(
        default,
        serialize_with = "serialize_option_secret",
        deserialize_with = "deserialize_option_secret"
    )]
    pub authtoken: Option<Secret<String>>,
    /// Optional reserved/static domain to request from ngrok.
    pub domain: Option<String>,
}

/// Failover configuration for automatic model/provider failover.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FailoverConfig {
    /// Whether failover is enabled. Defaults to true.
    pub enabled: bool,
    /// Ordered list of fallback model IDs to try when the primary fails.
    /// If empty, the chain is built from all registered models.
    #[serde(default)]
    pub fallback_models: Vec<String>,
}

impl Default for FailoverConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            fallback_models: Vec::new(),
        }
    }
}

/// Heartbeat configuration — periodic health-check agent turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HeartbeatConfig {
    /// Whether the heartbeat is enabled. Defaults to true.
    pub enabled: bool,
    /// Interval between heartbeats (e.g. "30m", "1h"). Defaults to "30m".
    pub every: String,
    /// Provider/model override for heartbeat turns (e.g. "anthropic/claude-sonnet-4-20250514").
    pub model: Option<String>,
    /// Agent ID override for heartbeat turns.
    pub agent_id: Option<String>,
    /// Custom prompt override. If empty, the built-in default is used.
    pub prompt: Option<String>,
    /// Max characters for an acknowledgment reply before truncation. Defaults to 300.
    pub ack_max_chars: usize,
    /// Active hours window — heartbeats only run during this window.
    pub active_hours: ActiveHoursConfig,
    /// Whether heartbeat replies should be delivered to a channel account.
    #[serde(default)]
    pub deliver: bool,
    /// Channel account identifier for heartbeat delivery (e.g. a Telegram bot account id).
    pub channel: Option<String>,
    /// Destination chat/recipient id for heartbeat delivery.
    pub to: Option<String>,
    /// Whether heartbeat runs inside a sandbox. Defaults to true.
    #[serde(default = "default_true")]
    pub sandbox_enabled: bool,
    /// Override sandbox image for heartbeat. If `None`, uses the default image.
    pub sandbox_image: Option<String>,
    /// Minimum duration between exec-triggered heartbeat wakes (e.g. "5m", "0").
    /// Prevents exec-completion callbacks from re-waking the heartbeat in a tight loop.
    /// Defaults to "5m". Set to "0" to disable.
    pub wake_cooldown: String,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            every: "30m".into(),
            model: None,
            agent_id: None,
            prompt: None,
            ack_max_chars: 300,
            active_hours: ActiveHoursConfig::default(),
            deliver: false,
            channel: None,
            to: None,
            sandbox_enabled: true,
            sandbox_image: None,
            wake_cooldown: "5m".into(),
        }
    }
}

/// Active hours window for heartbeats.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ActiveHoursConfig {
    /// Start time in HH:MM format. Defaults to "08:00".
    pub start: String,
    /// End time in HH:MM format. Defaults to "24:00" (midnight = always on until end of day).
    pub end: String,
    /// IANA timezone (e.g. "Europe/Paris") or "local". Defaults to "local".
    pub timezone: String,
}

impl Default for ActiveHoursConfig {
    fn default() -> Self {
        Self {
            start: "08:00".into(),
            end: "24:00".into(),
            timezone: "local".into(),
        }
    }
}

/// Cron scheduler configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CronConfig {
    /// Maximum number of jobs that can be created within the rate limit window.
    /// Defaults to 10.
    pub rate_limit_max: usize,
    /// Rate limit window in seconds. Defaults to 60 (1 minute).
    pub rate_limit_window_secs: u64,
    /// Number of days to retain cron session data before auto-cleanup.
    /// Set to `None` (or 0) to disable retention pruning. Defaults to 7 days.
    pub session_retention_days: Option<u64>,
    /// Whether to auto-prune sandbox containers after cron job completion.
    /// Per-job `auto_prune_container` overrides this global default.
    /// Defaults to true.
    #[serde(default = "default_true")]
    pub auto_prune_cron_containers: bool,
}

impl Default for CronConfig {
    fn default() -> Self {
        Self {
            rate_limit_max: 10,
            rate_limit_window_secs: 60,
            session_retention_days: Some(7),
            auto_prune_cron_containers: true,
        }
    }
}

/// Channel webhook middleware configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WebhooksConfig {
    /// Per-account rate limiting settings.
    pub rate_limit: WebhookRateLimitConfig,
}

/// Rate limiting configuration for channel webhooks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebhookRateLimitConfig {
    /// Whether rate limiting is enabled (default: true).
    pub enabled: bool,
    /// Override max requests per minute per account. When set, overrides the
    /// channel's built-in default. Leave unset to use per-channel defaults
    /// (Slack: 30/min, Teams: 60/min).
    pub requests_per_minute: Option<u32>,
    /// Override burst allowance per account.
    pub burst: Option<u32>,
    /// Interval in seconds between stale bucket cleanup (default: 300).
    pub cleanup_interval_secs: u64,
}

impl Default for WebhookRateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            requests_per_minute: None,
            burst: None,
            cleanup_interval_secs: 300,
        }
    }
}

/// CalDAV integration configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CalDavConfig {
    /// Whether CalDAV integration is enabled.
    pub enabled: bool,
    /// Default account name to use when none is specified.
    pub default_account: Option<String>,
    /// Named CalDAV accounts.
    #[serde(default)]
    pub accounts: HashMap<String, CalDavAccountConfig>,
}

/// Configuration for a single CalDAV account.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CalDavAccountConfig {
    /// CalDAV server URL (e.g. "https://caldav.fastmail.com/dav/calendars").
    pub url: Option<String>,
    /// Username for authentication.
    pub username: Option<String>,
    /// Password or app-specific password.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_option_secret",
        deserialize_with = "deserialize_option_secret"
    )]
    pub password: Option<Secret<String>>,
    /// Provider hint: "fastmail", "icloud", or "generic".
    pub provider: Option<String>,
    /// HTTP request timeout in seconds.
    #[serde(default = "default_caldav_timeout")]
    pub timeout_seconds: u64,
}

impl Default for CalDavAccountConfig {
    fn default() -> Self {
        Self {
            url: None,
            username: None,
            password: None,
            provider: None,
            timeout_seconds: default_caldav_timeout(),
        }
    }
}

impl std::fmt::Debug for CalDavAccountConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CalDavAccountConfig")
            .field("url", &self.url)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .field("provider", &self.provider)
            .field("timeout_seconds", &self.timeout_seconds)
            .finish()
    }
}

fn default_caldav_timeout() -> u64 {
    30
}

/// Home Assistant integration configuration.
///
/// ```toml
/// [home_assistant]
/// enabled = true
/// default_instance = "home"
///
/// [home_assistant.instances.home]
/// url = "http://homeassistant.local:8123"
/// token = "eyJ..."
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HomeAssistantConfig {
    /// Whether Home Assistant integration is enabled.
    pub enabled: bool,
    /// Default instance name when none is specified.
    pub default_instance: Option<String>,
    /// Named HA instances.
    #[serde(default)]
    pub instances: HashMap<String, HomeAssistantAccountConfig>,
}

impl HomeAssistantConfig {
    /// Resolve which account to use given an optional explicit name.
    ///
    /// Resolution order: explicit `instance` → `default_instance` →
    /// sole instance if only one exists.
    #[must_use]
    pub fn resolve_account<'a>(
        &'a self,
        instance: Option<&'a str>,
    ) -> Option<(&'a str, &'a HomeAssistantAccountConfig)> {
        let name = instance.or(self.default_instance.as_deref()).or_else(|| {
            if self.instances.len() == 1 {
                self.instances.keys().next().map(String::as_str)
            } else {
                None
            }
        })?;

        self.instances.get(name).map(|a| (name, a))
    }
}

/// Configuration for a single Home Assistant instance.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HomeAssistantAccountConfig {
    /// HA instance URL (e.g. "http://homeassistant.local:8123").
    pub url: Option<String>,
    /// Long-lived access token.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_option_secret",
        deserialize_with = "deserialize_option_secret"
    )]
    pub token: Option<Secret<String>>,
    /// HTTP request timeout in seconds.
    #[serde(default = "default_ha_timeout")]
    pub timeout_seconds: u64,
}

impl Default for HomeAssistantAccountConfig {
    fn default() -> Self {
        Self {
            url: None,
            token: None,
            timeout_seconds: default_ha_timeout(),
        }
    }
}

impl std::fmt::Debug for HomeAssistantAccountConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HomeAssistantAccountConfig")
            .field("url", &self.url)
            .field("token", &self.token.as_ref().map(|_| "[REDACTED]"))
            .field("timeout_seconds", &self.timeout_seconds)
            .finish()
    }
}

fn default_ha_timeout() -> u64 {
    10
}

/// Tailscale Serve/Funnel configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TailscaleConfig {
    /// Tailscale mode: "off", "serve", or "funnel".
    pub mode: String,
    /// Reset tailscale serve/funnel when the gateway shuts down.
    pub reset_on_exit: bool,
}

impl Default for TailscaleConfig {
    fn default() -> Self {
        Self {
            mode: "off".into(),
            reset_on_exit: true,
        }
    }
}
