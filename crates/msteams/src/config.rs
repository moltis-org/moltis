use {
    moltis_channels::{
        config_view::ChannelConfigView,
        gating::{DmPolicy, GroupPolicy, MentionMode},
    },
    moltis_common::secret_serde,
    secrecy::Secret,
    serde::{Deserialize, Serialize, ser::SerializeStruct},
    std::collections::HashMap,
};

/// Streaming mode for outbound messages.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamMode {
    /// Edit a posted message in-place as tokens arrive.
    #[default]
    EditInPlace,
    /// Accumulate all tokens and send once complete.
    Off,
}

/// Reply threading style for group conversations.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyStyle {
    /// Reply as a new top-level message.
    #[default]
    TopLevel,
    /// Reply in the same thread as the original message.
    Thread,
}

/// Per-team configuration overrides.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TeamConfig {
    /// Override mention mode for this team.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mention_mode: Option<MentionMode>,

    /// Override reply style for this team.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_style: Option<ReplyStyle>,

    /// Per-channel overrides within this team.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub channels: HashMap<String, ChannelOverride>,
}

/// Per-channel configuration overrides.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelOverride {
    /// Override mention mode for this channel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mention_mode: Option<MentionMode>,

    /// Override reply style for this channel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_style: Option<ReplyStyle>,

    /// Override model for this channel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Override model provider for this channel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
}

/// Configuration for a single Microsoft Teams bot account.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MsTeamsAccountConfig {
    /// Microsoft App ID (bot registration client ID).
    pub app_id: String,

    /// Microsoft App Password (client secret).
    #[serde(serialize_with = "secret_serde::serialize_secret")]
    pub app_password: Secret<String>,

    /// Azure AD tenant ID for JWT validation. Set to a specific tenant for
    /// single-tenant bots. Defaults to `"botframework.com"`.
    pub tenant_id: String,

    /// OAuth tenant segment for Bot Framework token issuance.
    pub oauth_tenant: String,

    /// OAuth scope for Bot Framework connector API.
    pub oauth_scope: String,

    /// DM access policy.
    pub dm_policy: DmPolicy,

    /// Group access policy.
    pub group_policy: GroupPolicy,

    /// Mention activation mode for group chats.
    pub mention_mode: MentionMode,

    /// User allowlist (AAD object IDs or channel user IDs).
    pub allowlist: Vec<String>,

    /// Group/team allowlist.
    pub group_allowlist: Vec<String>,

    /// Optional shared secret validated against `?secret=...` on webhook calls.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "secret_serde::serialize_option_secret"
    )]
    pub webhook_secret: Option<Secret<String>>,

    /// Default model ID for this channel account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Provider name associated with `model`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,

    // ── Retry configuration ─────────────────────────────────────────────
    /// Maximum retry attempts for failed sends (default: 3).
    pub max_retries: u32,

    /// Base delay in milliseconds for exponential backoff (default: 250).
    pub retry_base_delay_ms: u64,

    /// Maximum delay in milliseconds for exponential backoff (default: 10000).
    pub retry_max_delay_ms: u64,

    // ── Streaming configuration ─────────────────────────────────────────
    /// Streaming mode for outbound messages (default: edit_in_place).
    pub stream_mode: StreamMode,

    /// Minimum milliseconds between streaming edits (default: 1500).
    pub edit_throttle_ms: u64,

    // ── Message chunking ────────────────────────────────────────────────
    /// Maximum characters per message chunk (default: 4000).
    pub text_chunk_limit: usize,

    // ── Welcome cards ───────────────────────────────────────────────────
    /// Show a welcome card in DMs when user first messages (default: true).
    pub welcome_card: bool,

    /// Show welcome text in group chats when bot is added (default: false).
    pub group_welcome_card: bool,

    /// Bot display name for welcome cards. Falls back to "Moltis".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_name: Option<String>,

    /// Prompt starter buttons shown on the welcome card.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompt_starters: Vec<String>,

    // ── Threading and per-channel ───────────────────────────────────────
    /// Default reply style for group conversations.
    pub reply_style: ReplyStyle,

    /// Per-team configuration overrides.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub teams: HashMap<String, TeamConfig>,

    // ── Graph API (optional, for thread context and reactions) ───────────
    /// Graph API tenant ID for acquiring app-only Graph tokens.
    /// Required for thread context, reactions, and attachment downloads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_tenant_id: Option<String>,

    /// Maximum messages to fetch for thread context (default: 50).
    pub history_limit: usize,
}

impl std::fmt::Debug for MsTeamsAccountConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MsTeamsAccountConfig")
            .field("app_id", &self.app_id)
            .field("app_password", &"[REDACTED]")
            .field("tenant_id", &self.tenant_id)
            .field("oauth_tenant", &self.oauth_tenant)
            .field("oauth_scope", &self.oauth_scope)
            .field("dm_policy", &self.dm_policy)
            .field("group_policy", &self.group_policy)
            .field("mention_mode", &self.mention_mode)
            .field("allowlist", &self.allowlist)
            .field("group_allowlist", &self.group_allowlist)
            .field(
                "webhook_secret",
                &self.webhook_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("model", &self.model)
            .field("model_provider", &self.model_provider)
            .field("stream_mode", &self.stream_mode)
            .field("reply_style", &self.reply_style)
            .field("welcome_card", &self.welcome_card)
            .finish()
    }
}

/// Wrapper that serializes secret fields as `"[REDACTED]"` for API responses.
pub struct RedactedConfig<'a>(pub &'a MsTeamsAccountConfig);

impl Serialize for RedactedConfig<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let c = self.0;
        // Count: 14 always-present + conditional optional fields.
        let mut count = 14;
        count += c.webhook_secret.is_some() as usize;
        count += c.model.is_some() as usize;
        count += c.model_provider.is_some() as usize;
        count += c.bot_name.is_some() as usize;
        count += c.graph_tenant_id.is_some() as usize;
        count += !c.prompt_starters.is_empty() as usize;
        count += !c.teams.is_empty() as usize;
        let mut s = serializer.serialize_struct("MsTeamsAccountConfig", count)?;
        s.serialize_field("app_id", &c.app_id)?;
        s.serialize_field("app_password", secret_serde::REDACTED)?;
        s.serialize_field("tenant_id", &c.tenant_id)?;
        s.serialize_field("oauth_tenant", &c.oauth_tenant)?;
        s.serialize_field("oauth_scope", &c.oauth_scope)?;
        s.serialize_field("dm_policy", &c.dm_policy)?;
        s.serialize_field("group_policy", &c.group_policy)?;
        s.serialize_field("mention_mode", &c.mention_mode)?;
        s.serialize_field("allowlist", &c.allowlist)?;
        s.serialize_field("group_allowlist", &c.group_allowlist)?;
        if c.webhook_secret.is_some() {
            s.serialize_field("webhook_secret", secret_serde::REDACTED)?;
        }
        if c.model.is_some() {
            s.serialize_field("model", &c.model)?;
        }
        if c.model_provider.is_some() {
            s.serialize_field("model_provider", &c.model_provider)?;
        }
        s.serialize_field("stream_mode", &c.stream_mode)?;
        s.serialize_field("reply_style", &c.reply_style)?;
        s.serialize_field("welcome_card", &c.welcome_card)?;
        s.serialize_field("text_chunk_limit", &c.text_chunk_limit)?;
        if c.bot_name.is_some() {
            s.serialize_field("bot_name", &c.bot_name)?;
        }
        if !c.prompt_starters.is_empty() {
            s.serialize_field("prompt_starters", &c.prompt_starters)?;
        }
        if c.graph_tenant_id.is_some() {
            s.serialize_field("graph_tenant_id", &c.graph_tenant_id)?;
        }
        if !c.teams.is_empty() {
            s.serialize_field("teams", &c.teams)?;
        }
        s.end()
    }
}

impl ChannelConfigView for MsTeamsAccountConfig {
    fn allowlist(&self) -> &[String] {
        &self.allowlist
    }

    fn group_allowlist(&self) -> &[String] {
        &self.group_allowlist
    }

    fn dm_policy(&self) -> DmPolicy {
        self.dm_policy.clone()
    }

    fn group_policy(&self) -> GroupPolicy {
        self.group_policy.clone()
    }

    fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    fn model_provider(&self) -> Option<&str> {
        self.model_provider.as_deref()
    }

    fn channel_model(&self, channel_id: &str) -> Option<&str> {
        // Check per-team/per-channel overrides.
        for team_config in self.teams.values() {
            if let Some(ch_config) = team_config.channels.get(channel_id)
                && ch_config.model.is_some()
            {
                return ch_config.model.as_deref();
            }
        }
        None
    }

    fn channel_model_provider(&self, channel_id: &str) -> Option<&str> {
        for team_config in self.teams.values() {
            if let Some(ch_config) = team_config.channels.get(channel_id)
                && ch_config.model_provider.is_some()
            {
                return ch_config.model_provider.as_deref();
            }
        }
        None
    }
}

impl Default for MsTeamsAccountConfig {
    fn default() -> Self {
        Self {
            app_id: String::new(),
            app_password: Secret::new(String::new()),
            tenant_id: "botframework.com".into(),
            oauth_tenant: "botframework.com".into(),
            oauth_scope: "https://api.botframework.com/.default".into(),
            dm_policy: DmPolicy::Allowlist,
            group_policy: GroupPolicy::Open,
            mention_mode: MentionMode::Mention,
            allowlist: Vec::new(),
            group_allowlist: Vec::new(),
            webhook_secret: None,
            model: None,
            model_provider: None,
            max_retries: 3,
            retry_base_delay_ms: 250,
            retry_max_delay_ms: 10_000,
            stream_mode: StreamMode::default(),
            edit_throttle_ms: 1500,
            text_chunk_limit: 4000,
            welcome_card: true,
            group_welcome_card: false,
            bot_name: None,
            prompt_starters: Vec::new(),
            reply_style: ReplyStyle::default(),
            teams: HashMap::new(),
            graph_tenant_id: None,
            history_limit: 50,
        }
    }
}

/// Resolve the effective reply style for a given team/channel.
pub fn resolve_reply_style(
    config: &MsTeamsAccountConfig,
    team_id: Option<&str>,
    channel_id: Option<&str>,
) -> ReplyStyle {
    // Check channel-level override first.
    if let (Some(tid), Some(cid)) = (team_id, channel_id) {
        if let Some(team_config) = config.teams.get(tid) {
            if let Some(ch_config) = team_config.channels.get(cid)
                && let Some(ref style) = ch_config.reply_style
            {
                return style.clone();
            }
            // Then team-level override.
            if let Some(ref style) = team_config.reply_style {
                return style.clone();
            }
        }
    } else if let Some(tid) = team_id
        && let Some(team_config) = config.teams.get(tid)
        && let Some(ref style) = team_config.reply_style
    {
        return style.clone();
    }
    // Fall back to account-level default.
    config.reply_style.clone()
}

/// Resolve the effective mention mode for a given team/channel.
pub fn resolve_mention_mode(
    config: &MsTeamsAccountConfig,
    team_id: Option<&str>,
    channel_id: Option<&str>,
) -> MentionMode {
    if let (Some(tid), Some(cid)) = (team_id, channel_id) {
        if let Some(team_config) = config.teams.get(tid) {
            if let Some(ch_config) = team_config.channels.get(cid)
                && let Some(ref mode) = ch_config.mention_mode
            {
                return mode.clone();
            }
            if let Some(ref mode) = team_config.mention_mode {
                return mode.clone();
            }
        }
    } else if let Some(tid) = team_id
        && let Some(team_config) = config.teams.get(tid)
        && let Some(ref mode) = team_config.mention_mode
    {
        return mode.clone();
    }
    config.mention_mode.clone()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn redacted_hides_secrets() {
        let cfg = MsTeamsAccountConfig {
            app_id: "my-app-id".into(),
            app_password: Secret::new("super-secret-pw".into()),
            webhook_secret: Some(Secret::new("webhook-sec".into())),
            model: Some("gpt-4o".into()),
            ..Default::default()
        };
        let redacted = serde_json::to_value(RedactedConfig(&cfg)).unwrap();
        assert_eq!(redacted["app_password"], "[REDACTED]");
        assert_eq!(redacted["webhook_secret"], "[REDACTED]");
        // Non-secret fields preserved
        assert_eq!(redacted["app_id"], "my-app-id");
        assert_eq!(redacted["model"], "gpt-4o");

        // Storage path still exposes secrets
        let storage = serde_json::to_value(&cfg).unwrap();
        assert_eq!(storage["app_password"], "super-secret-pw");
        assert_eq!(storage["webhook_secret"], "webhook-sec");
    }

    #[test]
    fn redacted_omits_none_webhook_secret() {
        let cfg = MsTeamsAccountConfig::default();
        let redacted = serde_json::to_value(RedactedConfig(&cfg)).unwrap();
        assert!(redacted.get("webhook_secret").is_none());
    }

    #[test]
    fn config_round_trip() {
        let json = serde_json::json!({
            "app_id": "test-id",
            "app_password": "test-pw",
            "dm_policy": "open",
        });
        let cfg: MsTeamsAccountConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.app_id, "test-id");
        assert_eq!(cfg.dm_policy, DmPolicy::Open);
        let value = serde_json::to_value(&cfg).unwrap();
        let _: MsTeamsAccountConfig = serde_json::from_value(value).unwrap();
    }

    #[test]
    fn new_fields_have_defaults() {
        let cfg = MsTeamsAccountConfig::default();
        assert_eq!(cfg.max_retries, 3);
        assert_eq!(cfg.retry_base_delay_ms, 250);
        assert_eq!(cfg.retry_max_delay_ms, 10_000);
        assert_eq!(cfg.stream_mode, StreamMode::EditInPlace);
        assert_eq!(cfg.edit_throttle_ms, 1500);
        assert_eq!(cfg.text_chunk_limit, 4000);
        assert!(cfg.welcome_card);
        assert!(!cfg.group_welcome_card);
        assert_eq!(cfg.reply_style, ReplyStyle::TopLevel);
        assert_eq!(cfg.history_limit, 50);
        assert_eq!(cfg.tenant_id, "botframework.com");
    }

    #[test]
    fn stream_mode_serde() {
        let json = serde_json::json!("edit_in_place");
        let mode: StreamMode = serde_json::from_value(json).unwrap();
        assert_eq!(mode, StreamMode::EditInPlace);

        let json = serde_json::json!("off");
        let mode: StreamMode = serde_json::from_value(json).unwrap();
        assert_eq!(mode, StreamMode::Off);
    }

    #[test]
    fn reply_style_serde() {
        let json = serde_json::json!("thread");
        let style: ReplyStyle = serde_json::from_value(json).unwrap();
        assert_eq!(style, ReplyStyle::Thread);

        let json = serde_json::json!("top_level");
        let style: ReplyStyle = serde_json::from_value(json).unwrap();
        assert_eq!(style, ReplyStyle::TopLevel);
    }

    #[test]
    fn resolve_reply_style_channel_override() {
        let mut cfg = MsTeamsAccountConfig {
            reply_style: ReplyStyle::TopLevel,
            ..Default::default()
        };
        cfg.teams.insert("team-1".into(), TeamConfig {
            reply_style: Some(ReplyStyle::Thread),
            channels: {
                let mut m = HashMap::new();
                m.insert("ch-a".into(), ChannelOverride {
                    reply_style: Some(ReplyStyle::TopLevel),
                    ..Default::default()
                });
                m
            },
            ..Default::default()
        });

        // Channel-level override takes precedence.
        assert_eq!(
            resolve_reply_style(&cfg, Some("team-1"), Some("ch-a")),
            ReplyStyle::TopLevel
        );
        // Team-level override for unknown channel.
        assert_eq!(
            resolve_reply_style(&cfg, Some("team-1"), Some("ch-b")),
            ReplyStyle::Thread
        );
        // Account-level default for unknown team.
        assert_eq!(
            resolve_reply_style(&cfg, Some("team-2"), None),
            ReplyStyle::TopLevel
        );
    }

    #[test]
    fn channel_model_override() {
        let mut cfg = MsTeamsAccountConfig {
            model: Some("default-model".into()),
            ..Default::default()
        };
        cfg.teams.insert("team-1".into(), TeamConfig {
            channels: {
                let mut m = HashMap::new();
                m.insert("ch-1".into(), ChannelOverride {
                    model: Some("channel-model".into()),
                    ..Default::default()
                });
                m
            },
            ..Default::default()
        });

        assert_eq!(cfg.channel_model("ch-1"), Some("channel-model"));
        assert_eq!(cfg.channel_model("ch-2"), None);
    }

    #[test]
    fn per_team_config_round_trip() {
        let json = serde_json::json!({
            "app_id": "test",
            "app_password": "pw",
            "teams": {
                "team-1": {
                    "reply_style": "thread",
                    "channels": {
                        "general": {
                            "model": "gpt-4o",
                            "mention_mode": "always",
                        }
                    }
                }
            }
        });
        let cfg: MsTeamsAccountConfig = serde_json::from_value(json).unwrap();
        let team = cfg.teams.get("team-1").unwrap();
        assert_eq!(team.reply_style, Some(ReplyStyle::Thread));
        let ch = team.channels.get("general").unwrap();
        assert_eq!(ch.model.as_deref(), Some("gpt-4o"));
        assert_eq!(ch.mention_mode, Some(MentionMode::Always));
    }
}
