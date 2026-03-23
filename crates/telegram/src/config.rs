use std::collections::HashMap;

use {
    moltis_channels::{
        config_view::ChannelConfigView,
        gating::{DmPolicy, GroupPolicy, MentionMode},
    },
    secrecy::{ExposeSecret, Secret},
    serde::{Deserialize, Serialize},
};

/// Per-channel model/provider override.
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
    /// No streaming — send the final response as a single message.
    Off,
}

/// Configuration for a single Telegram bot account.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TelegramAccountConfig {
    /// Bot token from @BotFather.
    #[serde(serialize_with = "serialize_secret")]
    pub token: Secret<String>,

    /// DM access policy.
    pub dm_policy: DmPolicy,

    /// Group access policy.
    pub group_policy: GroupPolicy,

    /// Mention activation mode for groups.
    pub mention_mode: MentionMode,

    /// User/peer allowlist for DMs.
    pub allowlist: Vec<String>,

    /// Group/chat ID allowlist.
    pub group_allowlist: Vec<String>,

    /// How streaming responses are delivered.
    pub stream_mode: StreamMode,

    /// Minimum interval between edit-in-place updates (ms).
    pub edit_throttle_ms: u64,

    /// Send a short non-silent message when edit-in-place streaming finishes.
    /// This can trigger a reliable "completion" push notification in Telegram.
    pub stream_notify_on_complete: bool,

    /// Minimum number of characters to accumulate before sending the first
    /// streamed message. Helps avoid early push notifications with tiny drafts.
    pub stream_min_initial_chars: usize,

    /// Default model ID for this bot's sessions (e.g. "claude-sonnet-4-5-20250929").
    /// When set, channel messages use this model instead of the first registered provider.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Provider name associated with `model` (e.g. "anthropic").
    /// Stored alongside the model ID for display and debugging; the registry
    /// resolves the provider from the model ID at runtime.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,

    /// Enable OTP self-approval for non-allowlisted DM users (default: true).
    pub otp_self_approval: bool,

    /// Cooldown in seconds after 3 failed OTP attempts (default: 300).
    pub otp_cooldown_secs: u64,

    /// Send bot responses as Telegram replies to the user's message.
    /// When false (default), responses are sent as standalone messages.
    pub reply_to_message: bool,

    /// Per-channel model/provider overrides (chat_id -> override).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub channel_overrides: HashMap<String, ChannelOverride>,

    /// Per-user model/provider overrides (user_id -> override).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub user_overrides: HashMap<String, UserOverride>,
}

impl std::fmt::Debug for TelegramAccountConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelegramAccountConfig")
            .field("token", &"[REDACTED]")
            .field("dm_policy", &self.dm_policy)
            .field("group_policy", &self.group_policy)
            .field("channel_overrides", &self.channel_overrides)
            .field("user_overrides", &self.user_overrides)
            .finish_non_exhaustive()
    }
}

fn serialize_secret<S: serde::Serializer>(
    secret: &Secret<String>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(secret.expose_secret())
}

impl ChannelConfigView for TelegramAccountConfig {
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

impl Default for TelegramAccountConfig {
    fn default() -> Self {
        Self {
            token: Secret::new(String::new()),
            dm_policy: DmPolicy::default(),
            group_policy: GroupPolicy::default(),
            mention_mode: MentionMode::default(),
            allowlist: Vec::new(),
            group_allowlist: Vec::new(),
            stream_mode: StreamMode::default(),
            edit_throttle_ms: 300,
            stream_notify_on_complete: false,
            stream_min_initial_chars: 30,
            model: None,
            model_provider: None,
            otp_self_approval: true,
            otp_cooldown_secs: 300,
            reply_to_message: false,
            channel_overrides: HashMap::new(),
            user_overrides: HashMap::new(),
        }
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let cfg = TelegramAccountConfig::default();
        assert_eq!(cfg.dm_policy, DmPolicy::Allowlist);
        assert_eq!(cfg.group_policy, GroupPolicy::Open);
        assert_eq!(cfg.mention_mode, MentionMode::Mention);
        assert_eq!(cfg.stream_mode, StreamMode::EditInPlace);
        assert_eq!(cfg.edit_throttle_ms, 300);
        assert!(!cfg.stream_notify_on_complete);
        assert_eq!(cfg.stream_min_initial_chars, 30);
    }

    #[test]
    fn deserialize_from_json() {
        let json = r#"{
            "token": "123:ABC",
            "dm_policy": "allowlist",
            "stream_mode": "off",
            "stream_notify_on_complete": true,
            "stream_min_initial_chars": 42,
            "allowlist": ["user1", "user2"]
        }"#;
        let cfg: TelegramAccountConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.token.expose_secret(), "123:ABC");
        assert_eq!(cfg.dm_policy, DmPolicy::Allowlist);
        assert_eq!(cfg.stream_mode, StreamMode::Off);
        assert!(cfg.stream_notify_on_complete);
        assert_eq!(cfg.stream_min_initial_chars, 42);
        assert_eq!(cfg.allowlist, vec!["user1", "user2"]);
        // defaults for unspecified fields
        assert_eq!(cfg.group_policy, GroupPolicy::Open);
    }

    #[test]
    fn default_config_has_empty_overrides() {
        let cfg = TelegramAccountConfig::default();
        assert!(cfg.channel_overrides.is_empty());
        assert!(cfg.user_overrides.is_empty());
    }

    #[test]
    fn resolve_model_user_overrides_channel() {
        let mut cfg = TelegramAccountConfig {
            model: Some("default-model".into()),
            ..Default::default()
        };
        cfg.channel_overrides
            .insert("-100123".into(), ChannelOverride {
                model: Some("channel-model".into()),
                ..Default::default()
            });
        cfg.user_overrides.insert("456".into(), UserOverride {
            model: Some("user-model".into()),
            ..Default::default()
        });

        // User override wins
        assert_eq!(cfg.resolve_model("-100123", "456"), Some("user-model"));
        // Channel override wins when no user override
        assert_eq!(cfg.resolve_model("-100123", "999"), Some("channel-model"));
        // Account default when no overrides
        assert_eq!(cfg.resolve_model("-100999", "999"), Some("default-model"));
    }

    #[test]
    fn overrides_round_trip() {
        let json = serde_json::json!({
            "token": "123:ABC",
            "channel_overrides": {
                "-100123": { "model": "gpt-4" }
            },
            "user_overrides": {
                "456": { "model": "claude-sonnet", "model_provider": "anthropic" }
            }
        });
        let cfg: TelegramAccountConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.channel_model("-100123"), Some("gpt-4"));
        assert!(cfg.channel_model_provider("-100123").is_none());
        assert_eq!(cfg.user_model("456"), Some("claude-sonnet"));
        assert_eq!(cfg.user_model_provider("456"), Some("anthropic"));

        // Round-trip preserves overrides
        let value = serde_json::to_value(&cfg).unwrap();
        let cfg2: TelegramAccountConfig = serde_json::from_value(value).unwrap();
        assert_eq!(cfg2.channel_model("-100123"), Some("gpt-4"));
        assert_eq!(cfg2.user_model("456"), Some("claude-sonnet"));
    }

    #[test]
    fn serialize_roundtrip() {
        let cfg = TelegramAccountConfig {
            token: Secret::new("tok".into()),
            dm_policy: DmPolicy::Disabled,
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let cfg2: TelegramAccountConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg2.dm_policy, DmPolicy::Disabled);
        assert_eq!(cfg2.token.expose_secret(), "tok");
    }
}
