use std::collections::HashMap;

use {
    moltis_channels::{
        config_view::ChannelConfigView,
        gating::{DmPolicy, GroupPolicy, MentionMode},
    },
    moltis_common::secret_serde,
    secrecy::Secret,
    serde::{Deserialize, Serialize, ser::SerializeStruct},
};

/// Per-room model/provider override.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
}

/// Per-user model/provider override.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
}

/// How streaming responses are delivered.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StreamMode {
    /// Edit a placeholder message in place as tokens arrive.
    #[default]
    EditInPlace,
    /// Disable streaming and send only the final response.
    Off,
}

/// How Moltis handles room invites.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutoJoinPolicy {
    /// Auto-join every invite.
    #[default]
    Always,
    /// Auto-join only when the inviter or room is already allowlisted.
    Allowlist,
    /// Never auto-join invites.
    Off,
}

/// Who manages the Matrix account's encryption ownership.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MatrixOwnershipMode {
    /// Moltis should not attempt to bootstrap cross-signing or recovery.
    #[default]
    UserManaged,
    /// Moltis should bootstrap and manage this dedicated bot account.
    MoltisOwned,
}

/// Configuration for a single Matrix account.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MatrixAccountConfig {
    /// Homeserver URL (e.g. "https://matrix.ponderosa.co").
    pub homeserver: String,

    /// Access token for authentication.
    #[serde(serialize_with = "secret_serde::serialize_secret")]
    pub access_token: Secret<String>,

    /// Password for interactive login when an access token is not available.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "secret_serde::serialize_option_secret"
    )]
    pub password: Option<Secret<String>>,

    /// Matrix user ID (auto-detected from whoami if not set).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,

    /// Device ID for session persistence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,

    /// Optional display name for new login devices.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_display_name: Option<String>,

    /// Who manages the account's encryption ownership.
    pub ownership_mode: MatrixOwnershipMode,

    /// DM access policy.
    pub dm_policy: DmPolicy,

    /// Room (group) access policy.
    pub room_policy: GroupPolicy,

    /// Mention activation mode for rooms.
    pub mention_mode: MentionMode,

    /// Room allowlist (room IDs or aliases).
    pub room_allowlist: Vec<String>,

    /// User allowlist (Matrix user IDs).
    pub user_allowlist: Vec<String>,

    /// Auto-join rooms on invite.
    pub auto_join: AutoJoinPolicy,

    /// Default model ID for this account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Provider name associated with `model`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,

    /// How streaming responses are delivered.
    pub stream_mode: StreamMode,

    /// Minimum interval between edit-in-place updates (ms).
    pub edit_throttle_ms: u64,

    /// Minimum number of characters to accumulate before sending the first
    /// streamed message. Helps avoid early push notifications with tiny drafts.
    pub stream_min_initial_chars: usize,

    /// Per-room model/provider overrides (room_id -> override).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub channel_overrides: HashMap<String, ChannelOverride>,

    /// Per-user model/provider overrides (user_id -> override).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub user_overrides: HashMap<String, UserOverride>,

    /// Send responses as replies to the original message.
    pub reply_to_message: bool,

    /// Emoji reaction added while processing. None = disabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ack_reaction: Option<String>,

    /// OTP self-approval for non-allowlisted DM users.
    pub otp_self_approval: bool,

    /// Cooldown in seconds after 3 failed OTP attempts.
    pub otp_cooldown_secs: u64,
}

impl Default for MatrixAccountConfig {
    fn default() -> Self {
        Self {
            homeserver: String::new(),
            access_token: Secret::new(String::new()),
            password: None,
            user_id: None,
            device_id: None,
            device_display_name: None,
            ownership_mode: MatrixOwnershipMode::UserManaged,
            dm_policy: DmPolicy::Allowlist,
            room_policy: GroupPolicy::Allowlist,
            mention_mode: MentionMode::Mention,
            room_allowlist: Vec::new(),
            user_allowlist: Vec::new(),
            auto_join: AutoJoinPolicy::Always,
            model: None,
            model_provider: None,
            stream_mode: StreamMode::EditInPlace,
            edit_throttle_ms: 500,
            stream_min_initial_chars: 30,
            channel_overrides: HashMap::new(),
            user_overrides: HashMap::new(),
            reply_to_message: true,
            ack_reaction: Some("\u{1f440}".into()), // 👀
            otp_self_approval: true,
            otp_cooldown_secs: 300,
        }
    }
}

impl std::fmt::Debug for MatrixAccountConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MatrixAccountConfig")
            .field("homeserver", &self.homeserver)
            .field("access_token", &"[REDACTED]")
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .field("user_id", &self.user_id)
            .field("device_id", &self.device_id)
            .field("device_display_name", &self.device_display_name)
            .field("ownership_mode", &self.ownership_mode)
            .field("dm_policy", &self.dm_policy)
            .field("room_policy", &self.room_policy)
            .field("mention_mode", &self.mention_mode)
            .field("room_allowlist", &self.room_allowlist)
            .field("user_allowlist", &self.user_allowlist)
            .field("auto_join", &self.auto_join)
            .field("model", &self.model)
            .field("model_provider", &self.model_provider)
            .field("stream_mode", &self.stream_mode)
            .field("edit_throttle_ms", &self.edit_throttle_ms)
            .field("stream_min_initial_chars", &self.stream_min_initial_chars)
            .field("channel_overrides", &self.channel_overrides)
            .field("user_overrides", &self.user_overrides)
            .field("reply_to_message", &self.reply_to_message)
            .field("ack_reaction", &self.ack_reaction)
            .field("otp_self_approval", &self.otp_self_approval)
            .field("otp_cooldown_secs", &self.otp_cooldown_secs)
            .finish()
    }
}

/// Wrapper that serializes secret fields as `"[REDACTED]"` for API responses.
pub struct RedactedConfig<'a>(pub &'a MatrixAccountConfig);

impl Serialize for RedactedConfig<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let c = self.0;
        let mut count = 16;
        count += c.password.is_some() as usize;
        count += c.user_id.is_some() as usize;
        count += c.device_id.is_some() as usize;
        count += c.device_display_name.is_some() as usize;
        count += c.model.is_some() as usize;
        count += c.model_provider.is_some() as usize;
        count += !c.channel_overrides.is_empty() as usize;
        count += !c.user_overrides.is_empty() as usize;
        count += c.ack_reaction.is_some() as usize;

        let mut s = serializer.serialize_struct("MatrixAccountConfig", count)?;
        s.serialize_field("homeserver", &c.homeserver)?;
        s.serialize_field("access_token", secret_serde::REDACTED)?;
        if c.password.is_some() {
            s.serialize_field("password", secret_serde::REDACTED)?;
        }
        if c.user_id.is_some() {
            s.serialize_field("user_id", &c.user_id)?;
        }
        if c.device_id.is_some() {
            s.serialize_field("device_id", &c.device_id)?;
        }
        if c.device_display_name.is_some() {
            s.serialize_field("device_display_name", &c.device_display_name)?;
        }
        s.serialize_field("ownership_mode", &c.ownership_mode)?;
        s.serialize_field("dm_policy", &c.dm_policy)?;
        s.serialize_field("room_policy", &c.room_policy)?;
        s.serialize_field("mention_mode", &c.mention_mode)?;
        s.serialize_field("room_allowlist", &c.room_allowlist)?;
        s.serialize_field("user_allowlist", &c.user_allowlist)?;
        s.serialize_field("auto_join", &c.auto_join)?;
        if c.model.is_some() {
            s.serialize_field("model", &c.model)?;
        }
        if c.model_provider.is_some() {
            s.serialize_field("model_provider", &c.model_provider)?;
        }
        s.serialize_field("stream_mode", &c.stream_mode)?;
        s.serialize_field("edit_throttle_ms", &c.edit_throttle_ms)?;
        s.serialize_field("stream_min_initial_chars", &c.stream_min_initial_chars)?;
        if !c.channel_overrides.is_empty() {
            s.serialize_field("channel_overrides", &c.channel_overrides)?;
        }
        if !c.user_overrides.is_empty() {
            s.serialize_field("user_overrides", &c.user_overrides)?;
        }
        s.serialize_field("reply_to_message", &c.reply_to_message)?;
        if c.ack_reaction.is_some() {
            s.serialize_field("ack_reaction", &c.ack_reaction)?;
        }
        s.serialize_field("otp_self_approval", &c.otp_self_approval)?;
        s.serialize_field("otp_cooldown_secs", &c.otp_cooldown_secs)?;
        s.end()
    }
}

impl ChannelConfigView for MatrixAccountConfig {
    fn allowlist(&self) -> &[String] {
        &self.user_allowlist
    }

    fn group_allowlist(&self) -> &[String] {
        &self.room_allowlist
    }

    fn dm_policy(&self) -> DmPolicy {
        self.dm_policy.clone()
    }

    fn group_policy(&self) -> GroupPolicy {
        self.room_policy.clone()
    }

    fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    fn model_provider(&self) -> Option<&str> {
        self.model_provider.as_deref()
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
}

#[cfg(test)]
mod tests {
    use {super::*, secrecy::ExposeSecret, std::collections::HashMap};

    #[test]
    fn config_round_trip() {
        let json = serde_json::json!({
            "homeserver": "https://matrix.example.com",
            "access_token": "syt_test_token",
            "password": "wordpass",
            "ownership_mode": "moltis_owned",
            "dm_policy": "allowlist",
            "room_policy": "allowlist",
            "mention_mode": "mention",
            "room_allowlist": ["!room:example.com"],
            "user_allowlist": ["@alice:example.com"],
            "auto_join": "allowlist",
            "stream_mode": "off",
            "edit_throttle_ms": 750,
            "stream_min_initial_chars": 45,
            "channel_overrides": {
                "!room:example.com": { "model": "gpt-4.1" }
            },
            "user_overrides": {
                "@alice:example.com": { "model": "claude-sonnet" }
            },
            "reply_to_message": true,
            "ack_reaction": "\u{1f440}",
            "otp_self_approval": true,
            "otp_cooldown_secs": 300,
        });
        let cfg: MatrixAccountConfig =
            serde_json::from_value(json).unwrap_or_else(|error| panic!("parse failed: {error}"));
        assert_eq!(cfg.homeserver, "https://matrix.example.com");
        assert_eq!(
            cfg.password
                .as_ref()
                .map(|secret| secret.expose_secret().as_str()),
            Some("wordpass")
        );
        assert_eq!(cfg.dm_policy, DmPolicy::Allowlist);
        assert_eq!(cfg.ownership_mode, MatrixOwnershipMode::MoltisOwned);
        assert_eq!(cfg.room_allowlist, vec!["!room:example.com"]);
        assert_eq!(cfg.mention_mode, MentionMode::Mention);
        assert_eq!(cfg.auto_join, AutoJoinPolicy::Allowlist);
        assert_eq!(cfg.stream_mode, StreamMode::Off);
        assert_eq!(cfg.edit_throttle_ms, 750);
        assert_eq!(cfg.stream_min_initial_chars, 45);
        assert_eq!(cfg.channel_model("!room:example.com"), Some("gpt-4.1"));
        assert_eq!(cfg.user_model("@alice:example.com"), Some("claude-sonnet"));

        let value =
            serde_json::to_value(&cfg).unwrap_or_else(|error| panic!("serialize failed: {error}"));
        assert_eq!(value["access_token"], "syt_test_token");
        let _: MatrixAccountConfig = serde_json::from_value(value)
            .unwrap_or_else(|error| panic!("re-parse failed: {error}"));
    }

    #[test]
    fn config_defaults() {
        let cfg = MatrixAccountConfig::default();
        assert_eq!(cfg.dm_policy, DmPolicy::Allowlist);
        assert_eq!(cfg.room_policy, GroupPolicy::Allowlist);
        assert_eq!(cfg.mention_mode, MentionMode::Mention);
        assert_eq!(cfg.ownership_mode, MatrixOwnershipMode::UserManaged);
        assert_eq!(cfg.auto_join, AutoJoinPolicy::Always);
        assert_eq!(cfg.stream_mode, StreamMode::EditInPlace);
        assert_eq!(cfg.edit_throttle_ms, 500);
        assert_eq!(cfg.stream_min_initial_chars, 30);
        assert!(cfg.reply_to_message);
        assert_eq!(cfg.ack_reaction.as_deref(), Some("\u{1f440}"));
        assert!(cfg.otp_self_approval);
        assert_eq!(cfg.otp_cooldown_secs, 300);
    }

    #[test]
    fn debug_redacts_token() {
        let cfg = MatrixAccountConfig {
            access_token: Secret::new("super-secret".into()),
            password: Some(Secret::new("hunter2".into())),
            ..Default::default()
        };
        let debug = format!("{cfg:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("super-secret"));
        assert!(!debug.contains("hunter2"));
    }

    #[test]
    fn redacted_config_hides_access_token() {
        let cfg = MatrixAccountConfig {
            access_token: Secret::new("super-secret".into()),
            password: Some(Secret::new("hunter2".into())),
            ..Default::default()
        };

        let value = serde_json::to_value(RedactedConfig(&cfg))
            .unwrap_or_else(|error| panic!("serialize failed: {error}"));

        assert_eq!(value["access_token"], "[REDACTED]");
        assert_eq!(value["password"], "[REDACTED]");
        assert!(!value.to_string().contains("super-secret"));
        assert!(!value.to_string().contains("hunter2"));
    }

    #[test]
    fn resolve_model_prefers_user_then_room_then_default() {
        let mut cfg = MatrixAccountConfig {
            model: Some("default-model".into()),
            ..Default::default()
        };
        cfg.channel_overrides
            .insert("!room:example.com".into(), ChannelOverride {
                model: Some("room-model".into()),
                model_provider: Some("openai".into()),
            });
        cfg.user_overrides
            .insert("@alice:example.com".into(), UserOverride {
                model: Some("user-model".into()),
                model_provider: Some("anthropic".into()),
            });

        assert_eq!(
            cfg.resolve_model("!room:example.com", "@alice:example.com"),
            Some("user-model")
        );
        assert_eq!(
            cfg.resolve_model("!room:example.com", "@bob:example.com"),
            Some("room-model")
        );
        assert_eq!(
            cfg.resolve_model("!other:example.com", "@bob:example.com"),
            Some("default-model")
        );
    }

    #[test]
    fn redacted_config_includes_override_maps_without_token() {
        let mut cfg = MatrixAccountConfig {
            access_token: Secret::new("super-secret".into()),
            ..Default::default()
        };
        cfg.channel_overrides = HashMap::from([("!room:example.com".into(), ChannelOverride {
            model: Some("room-model".into()),
            model_provider: None,
        })]);
        cfg.user_overrides = HashMap::from([("@alice:example.com".into(), UserOverride {
            model: Some("user-model".into()),
            model_provider: None,
        })]);

        let value = serde_json::to_value(RedactedConfig(&cfg))
            .unwrap_or_else(|error| panic!("serialize failed: {error}"));

        assert_eq!(
            value["channel_overrides"]["!room:example.com"]["model"],
            "room-model"
        );
        assert_eq!(
            value["user_overrides"]["@alice:example.com"]["model"],
            "user-model"
        );
        assert_eq!(value["stream_mode"], "edit_in_place");
        assert_eq!(value["edit_throttle_ms"], 500);
        assert_eq!(value["stream_min_initial_chars"], 30);
        assert_eq!(value["ownership_mode"], "user_managed");
        assert_eq!(value["access_token"], "[REDACTED]");
    }
}
