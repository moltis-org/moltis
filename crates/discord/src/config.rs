use std::collections::HashMap;

use {
    moltis_channels::{
        config_view::ChannelConfigView,
        gating::{self, DmPolicy, GroupPolicy, MentionMode},
    },
    moltis_common::secret_serde,
    secrecy::Secret,
    serde::{Deserialize, Serialize, ser::SerializeStruct},
};

/// Per-channel model/provider override.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

/// Per-user model/provider override.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

/// Per-pattern override for guild channels matched by name or category.
///
/// When a guild channel's name matches `channel_name` (glob) or its parent
/// category matches `category_id`, the fields here override the account
/// defaults. The first matching override wins.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PatternOverride {
    /// Glob pattern matched against the Discord channel name (case-insensitive).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_name: Option<String>,
    /// Discord category (parent channel) ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
    /// Override mention mode for matched channels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mention_mode: Option<MentionMode>,
    /// Override model for matched channels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

/// Discord bot activity type for presence display.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityType {
    Playing,
    Listening,
    Watching,
    Competing,
    #[default]
    Custom,
}

/// Bot online status.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnlineStatus {
    #[default]
    Online,
    Idle,
    Dnd,
    Invisible,
}

/// Configuration for a single Discord bot account.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscordAccountConfig {
    /// Discord bot token.
    #[serde(serialize_with = "secret_serde::serialize_secret")]
    pub token: Secret<String>,

    /// DM access policy.
    pub dm_policy: DmPolicy,

    /// Group (guild channel) access policy.
    pub group_policy: GroupPolicy,

    /// Mention activation mode for guild channels.
    pub mention_mode: MentionMode,

    /// User allowlist (Discord user IDs or usernames).
    pub allowlist: Vec<String>,

    /// Guild allowlist (Discord guild/server IDs).
    pub guild_allowlist: Vec<String>,

    /// Default model ID for this channel account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Provider name associated with `model`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,

    /// Default agent ID for this channel account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,

    /// Send bot responses as Discord replies to the user's message.
    /// When false (default), responses are sent as standalone messages.
    pub reply_to_message: bool,

    /// Emoji reaction added to incoming messages while processing.
    /// Set to `null`/omit to disable. Default: disabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ack_reaction: Option<String>,

    /// Bot activity status text (e.g. "with AI").
    /// When set, the bot displays a status like "Playing with AI".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,

    /// Activity type: "playing", "listening", "watching", "competing", or "custom".
    /// Default: "custom" when `activity` is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity_type: Option<ActivityType>,

    /// Bot online status: "online", "idle", "dnd", or "invisible".
    /// Default: "online".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<OnlineStatus>,

    /// Enable OTP self-approval for non-allowlisted DM users (default: true).
    pub otp_self_approval: bool,

    /// Cooldown in seconds after 3 failed OTP attempts (default: 300).
    pub otp_cooldown_secs: u64,

    /// Per-channel model/provider overrides (channel_id -> override).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub channel_overrides: HashMap<String, ChannelOverride>,

    /// Per-user model/provider overrides (user_id -> override).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub user_overrides: HashMap<String, UserOverride>,

    /// Glob patterns for guild channel names. When non-empty, the bot only
    /// responds in guild channels whose name matches at least one pattern.
    /// Matched channels also implicitly use `mention_mode: always`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channel_name_patterns: Vec<String>,

    /// Discord category (parent channel) IDs. When non-empty, the bot only
    /// responds in guild channels under one of these categories. Combined
    /// with `channel_name_patterns` via OR.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub category_allowlist: Vec<String>,

    /// Per-pattern model/agent/mention overrides for guild channels.
    /// First matching override wins. Checked after exact channel_id overrides
    /// but before account defaults during model/agent resolution.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pattern_overrides: Vec<PatternOverride>,
}

impl std::fmt::Debug for DiscordAccountConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscordAccountConfig")
            .field("token", &"[REDACTED]")
            .field("dm_policy", &self.dm_policy)
            .field("group_policy", &self.group_policy)
            .field("mention_mode", &self.mention_mode)
            .field("allowlist", &self.allowlist)
            .field("guild_allowlist", &self.guild_allowlist)
            .field("model", &self.model)
            .field("model_provider", &self.model_provider)
            .field("agent_id", &self.agent_id)
            .field("reply_to_message", &self.reply_to_message)
            .field("ack_reaction", &self.ack_reaction)
            .field("activity", &self.activity)
            .field("activity_type", &self.activity_type)
            .field("status", &self.status)
            .field("otp_self_approval", &self.otp_self_approval)
            .field("otp_cooldown_secs", &self.otp_cooldown_secs)
            .field("channel_overrides", &self.channel_overrides)
            .field("user_overrides", &self.user_overrides)
            .field("channel_name_patterns", &self.channel_name_patterns)
            .field("category_allowlist", &self.category_allowlist)
            .field("pattern_overrides", &self.pattern_overrides)
            .finish()
    }
}

impl DiscordAccountConfig {
    /// Find the first pattern override matching the given channel name/category.
    pub fn find_pattern_override(
        &self,
        channel_name: Option<&str>,
        category_id: Option<&str>,
    ) -> Option<&PatternOverride> {
        self.pattern_overrides.iter().find(|po| {
            let name_ok = match (&po.channel_name, channel_name) {
                (Some(pattern), Some(name)) => {
                    gating::is_allowed(name, std::slice::from_ref(pattern))
                },
                (Some(_), None) => false,
                (None, _) => true,
            };
            let cat_ok = match (&po.category_id, category_id) {
                (Some(cat), Some(cid)) => cat.eq_ignore_ascii_case(cid),
                (Some(_), None) => false,
                (None, _) => true,
            };
            // Both criteria must match AND at least one must be specified.
            name_ok && cat_ok && (po.channel_name.is_some() || po.category_id.is_some())
        })
    }

    /// Resolve effective model with pattern override support.
    ///
    /// Priority: user > channel ID > pattern > account default.
    pub fn resolve_model_with_pattern(
        &self,
        channel_id: &str,
        user_id: &str,
        channel_name: Option<&str>,
        category_id: Option<&str>,
    ) -> Option<&str> {
        self.user_model(user_id)
            .or_else(|| self.channel_model(channel_id))
            .or_else(|| {
                self.find_pattern_override(channel_name, category_id)
                    .and_then(|po| po.model.as_deref())
            })
            .or_else(|| self.model())
    }

    /// Resolve effective agent ID with pattern override support.
    pub fn resolve_agent_with_pattern(
        &self,
        channel_id: &str,
        user_id: &str,
        channel_name: Option<&str>,
        category_id: Option<&str>,
    ) -> Option<&str> {
        self.user_agent_id(user_id)
            .or_else(|| self.channel_agent_id(channel_id))
            .or_else(|| {
                self.find_pattern_override(channel_name, category_id)
                    .and_then(|po| po.agent_id.as_deref())
            })
            .or_else(|| self.agent_id())
    }
}

impl ChannelConfigView for DiscordAccountConfig {
    fn allowlist(&self) -> &[String] {
        &self.allowlist
    }

    fn group_allowlist(&self) -> &[String] {
        &self.guild_allowlist
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

    fn agent_id(&self) -> Option<&str> {
        self.agent_id.as_deref()
    }

    fn channel_model(&self, channel_id: &str) -> Option<&str> {
        self.channel_overrides
            .get(channel_id)
            .and_then(|o| o.model.as_deref())
    }

    fn channel_model_provider(&self, channel_id: &str) -> Option<&str> {
        self.channel_overrides
            .get(channel_id)
            .and_then(|o| o.model_provider.as_deref())
    }

    fn channel_agent_id(&self, channel_id: &str) -> Option<&str> {
        self.channel_overrides
            .get(channel_id)
            .and_then(|o| o.agent_id.as_deref())
    }

    fn user_model(&self, user_id: &str) -> Option<&str> {
        self.user_overrides
            .get(user_id)
            .and_then(|o| o.model.as_deref())
    }

    fn user_model_provider(&self, user_id: &str) -> Option<&str> {
        self.user_overrides
            .get(user_id)
            .and_then(|o| o.model_provider.as_deref())
    }

    fn user_agent_id(&self, user_id: &str) -> Option<&str> {
        self.user_overrides
            .get(user_id)
            .and_then(|o| o.agent_id.as_deref())
    }
}

impl Default for DiscordAccountConfig {
    fn default() -> Self {
        Self {
            token: Secret::new(String::new()),
            dm_policy: DmPolicy::Allowlist,
            group_policy: GroupPolicy::Open,
            mention_mode: MentionMode::Mention,
            allowlist: Vec::new(),
            guild_allowlist: Vec::new(),
            model: None,
            model_provider: None,
            agent_id: None,
            reply_to_message: false,
            ack_reaction: None,
            activity: None,
            activity_type: None,
            status: None,
            otp_self_approval: true,
            otp_cooldown_secs: 300,
            channel_overrides: HashMap::new(),
            user_overrides: HashMap::new(),
            channel_name_patterns: Vec::new(),
            category_allowlist: Vec::new(),
            pattern_overrides: Vec::new(),
        }
    }
}

/// Wrapper that serializes secret fields as `"[REDACTED]"` for API responses.
pub struct RedactedConfig<'a>(pub &'a DiscordAccountConfig);

impl Serialize for RedactedConfig<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let c = self.0;
        let mut count = 9; // always-present fields
        count += c.model.is_some() as usize;
        count += c.model_provider.is_some() as usize;
        count += c.agent_id.is_some() as usize;
        count += c.ack_reaction.is_some() as usize;
        count += c.activity.is_some() as usize;
        count += c.activity_type.is_some() as usize;
        count += c.status.is_some() as usize;
        count += !c.channel_overrides.is_empty() as usize;
        count += !c.user_overrides.is_empty() as usize;
        count += !c.channel_name_patterns.is_empty() as usize;
        count += !c.category_allowlist.is_empty() as usize;
        count += !c.pattern_overrides.is_empty() as usize;
        let mut s = serializer.serialize_struct("DiscordAccountConfig", count)?;
        s.serialize_field("token", secret_serde::REDACTED)?;
        s.serialize_field("dm_policy", &c.dm_policy)?;
        s.serialize_field("group_policy", &c.group_policy)?;
        s.serialize_field("mention_mode", &c.mention_mode)?;
        s.serialize_field("allowlist", &c.allowlist)?;
        s.serialize_field("guild_allowlist", &c.guild_allowlist)?;
        if c.model.is_some() {
            s.serialize_field("model", &c.model)?;
        }
        if c.model_provider.is_some() {
            s.serialize_field("model_provider", &c.model_provider)?;
        }
        if c.agent_id.is_some() {
            s.serialize_field("agent_id", &c.agent_id)?;
        }
        s.serialize_field("reply_to_message", &c.reply_to_message)?;
        if c.ack_reaction.is_some() {
            s.serialize_field("ack_reaction", &c.ack_reaction)?;
        }
        if c.activity.is_some() {
            s.serialize_field("activity", &c.activity)?;
        }
        if c.activity_type.is_some() {
            s.serialize_field("activity_type", &c.activity_type)?;
        }
        if c.status.is_some() {
            s.serialize_field("status", &c.status)?;
        }
        s.serialize_field("otp_self_approval", &c.otp_self_approval)?;
        s.serialize_field("otp_cooldown_secs", &c.otp_cooldown_secs)?;
        if !c.channel_overrides.is_empty() {
            s.serialize_field("channel_overrides", &c.channel_overrides)?;
        }
        if !c.user_overrides.is_empty() {
            s.serialize_field("user_overrides", &c.user_overrides)?;
        }
        if !c.channel_name_patterns.is_empty() {
            s.serialize_field("channel_name_patterns", &c.channel_name_patterns)?;
        }
        if !c.category_allowlist.is_empty() {
            s.serialize_field("category_allowlist", &c.category_allowlist)?;
        }
        if !c.pattern_overrides.is_empty() {
            s.serialize_field("pattern_overrides", &c.pattern_overrides)?;
        }
        s.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trip() {
        let json = serde_json::json!({
            "token": "Bot MTIzNDU2.example",
            "dm_policy": "open",
            "group_policy": "allowlist",
            "mention_mode": "always",
            "allowlist": ["12345", "67890"],
            "guild_allowlist": ["111222333"],
            "model": "gpt-4o",
            "model_provider": "openai",
        });
        let cfg: DiscordAccountConfig =
            serde_json::from_value(json).unwrap_or_else(|e| panic!("parse failed: {e}"));
        assert_eq!(cfg.dm_policy, DmPolicy::Open);
        assert_eq!(cfg.group_policy, GroupPolicy::Allowlist);
        assert_eq!(cfg.mention_mode, MentionMode::Always);
        assert_eq!(cfg.allowlist, vec!["12345", "67890"]);
        assert_eq!(cfg.guild_allowlist, vec!["111222333"]);
        assert_eq!(cfg.model.as_deref(), Some("gpt-4o"));

        // Round-trip through serde
        let value = serde_json::to_value(&cfg).unwrap_or_else(|e| panic!("serialize failed: {e}"));
        let _: DiscordAccountConfig =
            serde_json::from_value(value).unwrap_or_else(|e| panic!("re-parse failed: {e}"));
    }

    #[test]
    fn config_defaults() {
        let cfg = DiscordAccountConfig::default();
        assert_eq!(cfg.dm_policy, DmPolicy::Allowlist);
        assert_eq!(cfg.group_policy, GroupPolicy::Open);
        assert_eq!(cfg.mention_mode, MentionMode::Mention);
        assert!(cfg.allowlist.is_empty());
        assert!(cfg.guild_allowlist.is_empty());
        assert!(cfg.model.is_none());
        assert!(cfg.agent_id.is_none());
        assert!(!cfg.reply_to_message);
        assert!(cfg.ack_reaction.is_none());
        assert!(cfg.activity.is_none());
        assert!(cfg.activity_type.is_none());
        assert!(cfg.status.is_none());
        assert!(cfg.otp_self_approval);
        assert_eq!(cfg.otp_cooldown_secs, 300);
    }

    #[test]
    fn config_with_reply_and_ack() {
        let json = serde_json::json!({
            "token": "Bot test",
            "reply_to_message": true,
            "ack_reaction": "\u{1f440}",
        });
        let cfg: DiscordAccountConfig =
            serde_json::from_value(json).unwrap_or_else(|e| panic!("parse failed: {e}"));
        assert!(cfg.reply_to_message);
        assert_eq!(cfg.ack_reaction.as_deref(), Some("\u{1f440}"));
    }

    #[test]
    fn config_with_presence() {
        let json = serde_json::json!({
            "token": "Bot test",
            "activity": "with AI",
            "activity_type": "playing",
            "status": "dnd",
        });
        let cfg: DiscordAccountConfig =
            serde_json::from_value(json).unwrap_or_else(|e| panic!("parse failed: {e}"));
        assert_eq!(cfg.activity.as_deref(), Some("with AI"));
        assert_eq!(cfg.activity_type, Some(ActivityType::Playing));
        assert_eq!(cfg.status, Some(OnlineStatus::Dnd));
    }

    #[test]
    fn config_with_otp() {
        let json = serde_json::json!({
            "token": "Bot test",
            "otp_self_approval": false,
            "otp_cooldown_secs": 600,
        });
        let cfg: DiscordAccountConfig =
            serde_json::from_value(json).unwrap_or_else(|e| panic!("parse failed: {e}"));
        assert!(!cfg.otp_self_approval);
        assert_eq!(cfg.otp_cooldown_secs, 600);
    }

    #[test]
    fn activity_type_serde_round_trip() {
        for (s, expected) in [
            ("\"playing\"", ActivityType::Playing),
            ("\"listening\"", ActivityType::Listening),
            ("\"watching\"", ActivityType::Watching),
            ("\"competing\"", ActivityType::Competing),
            ("\"custom\"", ActivityType::Custom),
        ] {
            let parsed: ActivityType =
                serde_json::from_str(s).unwrap_or_else(|e| panic!("parse {s}: {e}"));
            assert_eq!(parsed, expected);
            let serialized = serde_json::to_string(&parsed)
                .unwrap_or_else(|e| panic!("serialize {expected:?}: {e}"));
            assert_eq!(serialized, s);
        }
    }

    #[test]
    fn online_status_serde_round_trip() {
        for (s, expected) in [
            ("\"online\"", OnlineStatus::Online),
            ("\"idle\"", OnlineStatus::Idle),
            ("\"dnd\"", OnlineStatus::Dnd),
            ("\"invisible\"", OnlineStatus::Invisible),
        ] {
            let parsed: OnlineStatus =
                serde_json::from_str(s).unwrap_or_else(|e| panic!("parse {s}: {e}"));
            assert_eq!(parsed, expected);
            let serialized = serde_json::to_string(&parsed)
                .unwrap_or_else(|e| panic!("serialize {expected:?}: {e}"));
            assert_eq!(serialized, s);
        }
    }

    #[test]
    fn config_full_round_trip_with_all_fields() {
        let json = serde_json::json!({
            "token": "Bot MTIzNDU2.example",
            "dm_policy": "open",
            "group_policy": "allowlist",
            "mention_mode": "always",
            "allowlist": ["12345"],
            "guild_allowlist": ["111222333"],
            "model": "gpt-4o",
            "model_provider": "openai",
            "reply_to_message": true,
            "ack_reaction": "\u{1f440}",
            "activity": "with AI",
            "activity_type": "watching",
            "status": "idle",
            "otp_self_approval": false,
            "otp_cooldown_secs": 600,
        });
        let cfg: DiscordAccountConfig =
            serde_json::from_value(json).unwrap_or_else(|e| panic!("parse failed: {e}"));
        assert!(cfg.reply_to_message);
        assert_eq!(cfg.ack_reaction.as_deref(), Some("\u{1f440}"));
        assert_eq!(cfg.activity.as_deref(), Some("with AI"));
        assert_eq!(cfg.activity_type, Some(ActivityType::Watching));
        assert_eq!(cfg.status, Some(OnlineStatus::Idle));
        assert!(!cfg.otp_self_approval);
        assert_eq!(cfg.otp_cooldown_secs, 600);

        // Round-trip: serialize and deserialize again.
        let value = serde_json::to_value(&cfg).unwrap_or_else(|e| panic!("serialize failed: {e}"));
        let cfg2: DiscordAccountConfig =
            serde_json::from_value(value).unwrap_or_else(|e| panic!("re-parse failed: {e}"));
        assert_eq!(cfg2.activity.as_deref(), Some("with AI"));
        assert_eq!(cfg2.activity_type, Some(ActivityType::Watching));
        assert_eq!(cfg2.status, Some(OnlineStatus::Idle));
        assert!(!cfg2.otp_self_approval);
    }

    #[test]
    fn presence_fields_serialized_when_set() {
        let cfg = DiscordAccountConfig {
            activity: Some("testing".into()),
            activity_type: Some(ActivityType::Listening),
            status: Some(OnlineStatus::Dnd),
            ..Default::default()
        };
        let value = serde_json::to_value(&cfg).unwrap_or_else(|e| panic!("serialize failed: {e}"));
        assert_eq!(value["activity"], "testing");
        assert_eq!(value["activity_type"], "listening");
        assert_eq!(value["status"], "dnd");
    }

    #[test]
    fn presence_fields_omitted_when_none() {
        let cfg = DiscordAccountConfig::default();
        let value = serde_json::to_value(&cfg).unwrap_or_else(|e| panic!("serialize failed: {e}"));
        assert!(
            value.get("activity").is_none(),
            "activity should be omitted when None"
        );
        assert!(
            value.get("activity_type").is_none(),
            "activity_type should be omitted when None"
        );
        assert!(
            value.get("status").is_none(),
            "status should be omitted when None"
        );
    }

    #[test]
    fn activity_type_default_is_custom() {
        assert_eq!(ActivityType::default(), ActivityType::Custom);
    }

    #[test]
    fn online_status_default_is_online() {
        assert_eq!(OnlineStatus::default(), OnlineStatus::Online);
    }

    #[test]
    fn debug_redacts_token() {
        let cfg = DiscordAccountConfig {
            token: Secret::new("super-secret-bot-token".into()),
            ..Default::default()
        };
        let debug = format!("{cfg:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("super-secret-bot-token"));
    }

    #[test]
    fn debug_includes_presence_fields() {
        let cfg = DiscordAccountConfig {
            activity: Some("chatting".into()),
            activity_type: Some(ActivityType::Playing),
            status: Some(OnlineStatus::Idle),
            ..Default::default()
        };
        let debug = format!("{cfg:?}");
        assert!(debug.contains("activity"), "debug should include activity");
        assert!(
            debug.contains("activity_type"),
            "debug should include activity_type"
        );
        assert!(debug.contains("status"), "debug should include status");
        assert!(
            debug.contains("otp_self_approval"),
            "debug should include otp_self_approval"
        );
    }

    #[test]
    fn debug_includes_otp_fields() {
        let cfg = DiscordAccountConfig {
            otp_self_approval: false,
            otp_cooldown_secs: 600,
            ..Default::default()
        };
        let debug = format!("{cfg:?}");
        assert!(debug.contains("otp_self_approval"));
        assert!(debug.contains("otp_cooldown_secs"));
    }

    #[test]
    fn invalid_activity_type_rejected() {
        let json = serde_json::json!({
            "token": "Bot test",
            "activity_type": "invalid_type",
        });
        let result: Result<DiscordAccountConfig, _> = serde_json::from_value(json);
        assert!(
            result.is_err(),
            "invalid activity_type should fail deserialization"
        );
    }

    #[test]
    fn config_defaults_include_empty_overrides() {
        let cfg = DiscordAccountConfig::default();
        assert!(cfg.channel_overrides.is_empty());
        assert!(cfg.user_overrides.is_empty());
    }

    #[test]
    fn resolve_model_user_overrides_channel() {
        let mut cfg = DiscordAccountConfig {
            model: Some("default-model".into()),
            agent_id: Some("default-agent".into()),
            ..Default::default()
        };
        cfg.channel_overrides
            .insert("C123".into(), ChannelOverride {
                model: Some("channel-model".into()),
                agent_id: Some("channel-agent".into()),
                ..Default::default()
            });
        cfg.user_overrides.insert("U456".into(), UserOverride {
            model: Some("user-model".into()),
            agent_id: Some("user-agent".into()),
            ..Default::default()
        });

        // User override wins
        assert_eq!(cfg.resolve_model("C123", "U456"), Some("user-model"));
        // Channel override wins when no user override
        assert_eq!(cfg.resolve_model("C123", "U999"), Some("channel-model"));
        // Account default when no overrides
        assert_eq!(cfg.resolve_model("C999", "U999"), Some("default-model"));
        assert_eq!(cfg.resolve_agent_id("C123", "U456"), Some("user-agent"));
        assert_eq!(cfg.resolve_agent_id("C123", "U999"), Some("channel-agent"));
        assert_eq!(cfg.resolve_agent_id("C999", "U999"), Some("default-agent"));
    }

    #[test]
    fn overrides_round_trip() {
        let json = serde_json::json!({
            "token": "Bot test",
            "channel_overrides": {
                "C123": { "model": "gpt-4", "agent_id": "channel-agent" }
            },
            "user_overrides": {
                "U456": { "model": "claude-sonnet", "model_provider": "anthropic", "agent_id": "user-agent" }
            }
        });
        let cfg: DiscordAccountConfig =
            serde_json::from_value(json).unwrap_or_else(|e| panic!("parse failed: {e}"));
        assert_eq!(cfg.channel_model("C123"), Some("gpt-4"));
        assert!(cfg.channel_model_provider("C123").is_none());
        assert_eq!(cfg.channel_agent_id("C123"), Some("channel-agent"));
        assert_eq!(cfg.user_model("U456"), Some("claude-sonnet"));
        assert_eq!(cfg.user_model_provider("U456"), Some("anthropic"));
        assert_eq!(cfg.user_agent_id("U456"), Some("user-agent"));

        // Round-trip preserves overrides
        let value = serde_json::to_value(&cfg).unwrap_or_else(|e| panic!("serialize failed: {e}"));
        let cfg2: DiscordAccountConfig =
            serde_json::from_value(value).unwrap_or_else(|e| panic!("re-parse failed: {e}"));
        assert_eq!(cfg2.channel_model("C123"), Some("gpt-4"));
        assert_eq!(cfg2.user_model("U456"), Some("claude-sonnet"));
        assert_eq!(cfg2.channel_agent_id("C123"), Some("channel-agent"));
        assert_eq!(cfg2.user_agent_id("U456"), Some("user-agent"));
    }

    #[test]
    fn redacted_hides_token() {
        let cfg = DiscordAccountConfig {
            token: Secret::new("super-secret-bot-token".into()),
            model: Some("gpt-4o".into()),
            agent_id: Some("research".into()),
            ..Default::default()
        };
        let redacted = serde_json::to_value(RedactedConfig(&cfg))
            .unwrap_or_else(|e| panic!("redacted serialize failed: {e}"));
        assert_eq!(redacted["token"], "[REDACTED]");
        // Non-secret fields preserved
        assert_eq!(redacted["model"], "gpt-4o");
        assert_eq!(redacted["agent_id"], "research");
        assert_eq!(
            redacted["dm_policy"],
            serde_json::to_value(&cfg.dm_policy)
                .unwrap_or_else(|e| panic!("dm_policy serialize failed: {e}"))
        );

        // Storage path still exposes the token
        let storage =
            serde_json::to_value(&cfg).unwrap_or_else(|e| panic!("storage serialize failed: {e}"));
        assert_eq!(storage["token"], "super-secret-bot-token");
    }

    #[test]
    fn invalid_online_status_rejected() {
        let json = serde_json::json!({
            "token": "Bot test",
            "status": "busy",
        });
        let result: Result<DiscordAccountConfig, _> = serde_json::from_value(json);
        assert!(
            result.is_err(),
            "invalid status should fail deserialization"
        );
    }

    // ── Channel name patterns & category allowlist ────────────────

    #[test]
    fn config_with_channel_name_patterns() {
        let json = serde_json::json!({
            "token": "Bot test",
            "channel_name_patterns": ["ticket-*", "support-*"],
            "category_allowlist": ["123456789"],
        });
        let cfg: DiscordAccountConfig =
            serde_json::from_value(json).unwrap_or_else(|e| panic!("parse failed: {e}"));
        assert_eq!(cfg.channel_name_patterns, vec!["ticket-*", "support-*"]);
        assert_eq!(cfg.category_allowlist, vec!["123456789"]);

        // Round-trip
        let value = serde_json::to_value(&cfg).unwrap_or_else(|e| panic!("serialize failed: {e}"));
        assert_eq!(
            value["channel_name_patterns"],
            serde_json::json!(["ticket-*", "support-*"])
        );
        assert_eq!(
            value["category_allowlist"],
            serde_json::json!(["123456789"])
        );
    }

    #[test]
    fn config_defaults_include_empty_pattern_fields() {
        let cfg = DiscordAccountConfig::default();
        assert!(cfg.channel_name_patterns.is_empty());
        assert!(cfg.category_allowlist.is_empty());
        assert!(cfg.pattern_overrides.is_empty());
    }

    #[test]
    fn pattern_fields_omitted_when_empty() {
        let cfg = DiscordAccountConfig::default();
        let value = serde_json::to_value(&cfg).unwrap_or_else(|e| panic!("serialize failed: {e}"));
        assert!(value.get("channel_name_patterns").is_none());
        assert!(value.get("category_allowlist").is_none());
        assert!(value.get("pattern_overrides").is_none());
    }

    // ── Pattern overrides ─────────────────────────────────────────

    #[test]
    fn pattern_overrides_round_trip() {
        let json = serde_json::json!({
            "token": "Bot test",
            "pattern_overrides": [
                {
                    "channel_name": "ticket-*",
                    "mention_mode": "always",
                    "model": "claude-sonnet",
                    "agent_id": "support"
                },
                {
                    "category_id": "999888777",
                    "model": "gpt-4o"
                }
            ]
        });
        let cfg: DiscordAccountConfig =
            serde_json::from_value(json).unwrap_or_else(|e| panic!("parse failed: {e}"));
        assert_eq!(cfg.pattern_overrides.len(), 2);
        assert_eq!(
            cfg.pattern_overrides[0].channel_name.as_deref(),
            Some("ticket-*")
        );
        assert_eq!(
            cfg.pattern_overrides[0].mention_mode,
            Some(MentionMode::Always)
        );
        assert_eq!(
            cfg.pattern_overrides[0].agent_id.as_deref(),
            Some("support")
        );
        assert_eq!(
            cfg.pattern_overrides[1].category_id.as_deref(),
            Some("999888777")
        );

        let value = serde_json::to_value(&cfg).unwrap_or_else(|e| panic!("serialize failed: {e}"));
        let cfg2: DiscordAccountConfig =
            serde_json::from_value(value).unwrap_or_else(|e| panic!("re-parse failed: {e}"));
        assert_eq!(cfg2.pattern_overrides.len(), 2);
    }

    #[test]
    fn find_pattern_override_by_name() {
        let cfg = DiscordAccountConfig {
            pattern_overrides: vec![
                PatternOverride {
                    channel_name: Some("ticket-*".into()),
                    agent_id: Some("support".into()),
                    ..Default::default()
                },
                PatternOverride {
                    channel_name: Some("vip-*".into()),
                    agent_id: Some("vip".into()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let po = cfg.find_pattern_override(Some("ticket-42"), None);
        assert_eq!(po.and_then(|p| p.agent_id.as_deref()), Some("support"));

        let po = cfg.find_pattern_override(Some("vip-lounge"), None);
        assert_eq!(po.and_then(|p| p.agent_id.as_deref()), Some("vip"));

        assert!(cfg.find_pattern_override(Some("general"), None).is_none());
        assert!(cfg.find_pattern_override(None, None).is_none());
    }

    #[test]
    fn find_pattern_override_by_category() {
        let cfg = DiscordAccountConfig {
            pattern_overrides: vec![PatternOverride {
                category_id: Some("CAT123".into()),
                model: Some("gpt-4o".into()),
                ..Default::default()
            }],
            ..Default::default()
        };

        let po = cfg.find_pattern_override(None, Some("CAT123"));
        assert_eq!(po.and_then(|p| p.model.as_deref()), Some("gpt-4o"));

        // Case-insensitive category match
        let po = cfg.find_pattern_override(None, Some("cat123"));
        assert_eq!(po.and_then(|p| p.model.as_deref()), Some("gpt-4o"));

        assert!(cfg.find_pattern_override(None, Some("OTHER")).is_none());
    }

    #[test]
    fn resolve_model_with_pattern_override() {
        let cfg = DiscordAccountConfig {
            model: Some("default-model".into()),
            pattern_overrides: vec![PatternOverride {
                channel_name: Some("ticket-*".into()),
                model: Some("ticket-model".into()),
                ..Default::default()
            }],
            channel_overrides: {
                let mut m = HashMap::new();
                m.insert("C999".into(), ChannelOverride {
                    model: Some("exact-model".into()),
                    ..Default::default()
                });
                m
            },
            ..Default::default()
        };

        // Pattern override
        assert_eq!(
            cfg.resolve_model_with_pattern("C123", "U1", Some("ticket-42"), None),
            Some("ticket-model")
        );
        // Exact channel override wins over pattern
        assert_eq!(
            cfg.resolve_model_with_pattern("C999", "U1", Some("ticket-42"), None),
            Some("exact-model")
        );
        // No match falls to default
        assert_eq!(
            cfg.resolve_model_with_pattern("C123", "U1", Some("general"), None),
            Some("default-model")
        );
    }

    #[test]
    fn resolve_agent_with_pattern_override() {
        let cfg = DiscordAccountConfig {
            agent_id: Some("default-agent".into()),
            pattern_overrides: vec![PatternOverride {
                channel_name: Some("ticket-*".into()),
                agent_id: Some("support".into()),
                ..Default::default()
            }],
            ..Default::default()
        };

        assert_eq!(
            cfg.resolve_agent_with_pattern("C1", "U1", Some("ticket-1"), None),
            Some("support")
        );
        assert_eq!(
            cfg.resolve_agent_with_pattern("C1", "U1", Some("general"), None),
            Some("default-agent")
        );
    }

    #[test]
    fn redacted_includes_pattern_fields() {
        let cfg = DiscordAccountConfig {
            channel_name_patterns: vec!["ticket-*".into()],
            category_allowlist: vec!["123".into()],
            pattern_overrides: vec![PatternOverride {
                channel_name: Some("ticket-*".into()),
                agent_id: Some("support".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let redacted = serde_json::to_value(RedactedConfig(&cfg))
            .unwrap_or_else(|e| panic!("redacted serialize failed: {e}"));
        assert_eq!(
            redacted["channel_name_patterns"],
            serde_json::json!(["ticket-*"])
        );
        assert_eq!(redacted["category_allowlist"], serde_json::json!(["123"]));
        assert_eq!(
            redacted["pattern_overrides"].as_array().map(|a| a.len()),
            Some(1)
        );
    }
}
