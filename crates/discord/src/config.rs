use {
    moltis_channels::gating::{DmPolicy, GroupPolicy, MentionMode},
    secrecy::{ExposeSecret, Secret},
    serde::{Deserialize, Serialize},
};

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
    #[serde(serialize_with = "serialize_secret")]
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
            .field("reply_to_message", &self.reply_to_message)
            .field("ack_reaction", &self.ack_reaction)
            .field("activity", &self.activity)
            .field("activity_type", &self.activity_type)
            .field("status", &self.status)
            .field("otp_self_approval", &self.otp_self_approval)
            .field("otp_cooldown_secs", &self.otp_cooldown_secs)
            .finish()
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
            reply_to_message: false,
            ack_reaction: None,
            activity: None,
            activity_type: None,
            status: None,
            otp_self_approval: true,
            otp_cooldown_secs: 300,
        }
    }
}

fn serialize_secret<S: serde::Serializer>(
    secret: &Secret<String>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(secret.expose_secret())
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
}
