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

/// How this Slack account connects to Slack.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionMode {
    /// Use Socket Mode (requires `app_token`).
    #[default]
    SocketMode,
    /// Use Events API / HTTP webhook (requires `signing_secret`).
    EventsApi,
}

/// Stream mode for Slack responses.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamMode {
    /// Edit the placeholder message in-place as tokens arrive.
    #[default]
    EditInPlace,
    /// Use Slack's native chat streaming API (startStream/appendStream/stopStream).
    Native,
    /// Disable streaming — send the full response once complete.
    Off,
}

/// Configuration for a single Slack bot account.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SlackAccountConfig {
    /// Bot user OAuth token (`xoxb-...`).
    #[serde(serialize_with = "serialize_secret")]
    pub bot_token: Secret<String>,

    /// App-level token for Socket Mode (`xapp-...`).
    /// Required when `connection_mode` is `socket_mode`.
    #[serde(serialize_with = "serialize_secret")]
    pub app_token: Secret<String>,

    /// How this account connects to Slack.
    pub connection_mode: ConnectionMode,

    /// Signing secret for Events API request verification.
    /// Required when `connection_mode` is `events_api`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_optional_secret"
    )]
    pub signing_secret: Option<Secret<String>>,

    /// DM access policy.
    pub dm_policy: DmPolicy,

    /// Channel/group access policy.
    pub group_policy: GroupPolicy,

    /// Mention activation mode for channels.
    pub mention_mode: MentionMode,

    /// DM user allowlist (Slack user IDs).
    #[serde(default)]
    pub allowlist: Vec<String>,

    /// Channel allowlist (Slack channel IDs).
    #[serde(default)]
    pub channel_allowlist: Vec<String>,

    /// Default model for this account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Provider for the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,

    /// Stream mode for responses.
    pub stream_mode: StreamMode,

    /// Minimum milliseconds between edit-in-place updates.
    pub edit_throttle_ms: u64,

    /// Reply in threads (default: true).
    pub thread_replies: bool,

    /// Per-channel model/provider overrides (channel_id -> override).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub channel_overrides: HashMap<String, ChannelOverride>,

    /// Per-user model/provider overrides (user_id -> override).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub user_overrides: HashMap<String, UserOverride>,
}

impl std::fmt::Debug for SlackAccountConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlackAccountConfig")
            .field("bot_token", &"[REDACTED]")
            .field("app_token", &"[REDACTED]")
            .field("connection_mode", &self.connection_mode)
            .field(
                "signing_secret",
                &self.signing_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("dm_policy", &self.dm_policy)
            .field("group_policy", &self.group_policy)
            .field("mention_mode", &self.mention_mode)
            .field("allowlist", &self.allowlist)
            .field("channel_allowlist", &self.channel_allowlist)
            .field("model", &self.model)
            .field("model_provider", &self.model_provider)
            .field("stream_mode", &self.stream_mode)
            .field("edit_throttle_ms", &self.edit_throttle_ms)
            .field("thread_replies", &self.thread_replies)
            .field("channel_overrides", &self.channel_overrides)
            .field("user_overrides", &self.user_overrides)
            .finish()
    }
}

impl Default for SlackAccountConfig {
    fn default() -> Self {
        Self {
            bot_token: Secret::new(String::new()),
            app_token: Secret::new(String::new()),
            connection_mode: ConnectionMode::SocketMode,
            signing_secret: None,
            dm_policy: DmPolicy::Allowlist,
            group_policy: GroupPolicy::Open,
            mention_mode: MentionMode::Mention,
            allowlist: Vec::new(),
            channel_allowlist: Vec::new(),
            model: None,
            model_provider: None,
            stream_mode: StreamMode::EditInPlace,
            edit_throttle_ms: 500,
            thread_replies: true,
            channel_overrides: HashMap::new(),
            user_overrides: HashMap::new(),
        }
    }
}

impl ChannelConfigView for SlackAccountConfig {
    fn allowlist(&self) -> &[String] {
        &self.allowlist
    }

    fn group_allowlist(&self) -> &[String] {
        &self.channel_allowlist
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

fn serialize_secret<S: serde::Serializer>(
    secret: &Secret<String>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(secret.expose_secret())
}

fn serialize_optional_secret<S: serde::Serializer>(
    secret: &Option<Secret<String>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match secret {
        Some(s) => serializer.serialize_str(s.expose_secret()),
        None => serializer.serialize_none(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn default_config_round_trips() {
        let cfg = SlackAccountConfig::default();
        let json = serde_json::to_value(&cfg).unwrap();
        let _: SlackAccountConfig = serde_json::from_value(json).unwrap();
    }

    #[test]
    fn config_view_defaults() {
        let cfg = SlackAccountConfig::default();
        assert!(cfg.allowlist().is_empty());
        assert!(cfg.group_allowlist().is_empty());
        assert_eq!(cfg.dm_policy(), DmPolicy::Allowlist);
        assert_eq!(cfg.group_policy(), GroupPolicy::Open);
        assert!(cfg.model().is_none());
        assert!(cfg.model_provider().is_none());
    }

    #[test]
    fn config_with_tokens_round_trip() {
        let json = serde_json::json!({
            "bot_token": "xoxb-test-token",
            "app_token": "xapp-test-token",
            "dm_policy": "open",
            "group_policy": "allowlist",
            "mention_mode": "always",
            "allowlist": ["U123", "U456"],
            "channel_allowlist": ["C789"],
            "model": "claude-sonnet-4-20250514",
            "model_provider": "anthropic",
            "stream_mode": "edit_in_place",
            "edit_throttle_ms": 300,
            "thread_replies": false,
        });
        let cfg: SlackAccountConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.bot_token.expose_secret(), "xoxb-test-token");
        assert_eq!(cfg.app_token.expose_secret(), "xapp-test-token");
        assert_eq!(cfg.dm_policy, DmPolicy::Open);
        assert_eq!(cfg.group_policy, GroupPolicy::Allowlist);
        assert_eq!(cfg.mention_mode, MentionMode::Always);
        assert_eq!(cfg.allowlist, vec!["U123", "U456"]);
        assert_eq!(cfg.channel_allowlist, vec!["C789"]);
        assert_eq!(cfg.model.as_deref(), Some("claude-sonnet-4-20250514"));
        assert_eq!(cfg.stream_mode, StreamMode::EditInPlace);
        assert_eq!(cfg.edit_throttle_ms, 300);
        assert!(!cfg.thread_replies);

        // Round-trip
        let value = serde_json::to_value(&cfg).unwrap();
        let _: SlackAccountConfig = serde_json::from_value(value).unwrap();
    }

    #[test]
    fn stream_mode_off() {
        let json = serde_json::json!({
            "bot_token": "xoxb-test",
            "app_token": "xapp-test",
            "stream_mode": "off",
        });
        let cfg: SlackAccountConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.stream_mode, StreamMode::Off);
    }

    #[test]
    fn stream_mode_native() {
        let json = serde_json::json!({
            "bot_token": "xoxb-test",
            "app_token": "xapp-test",
            "stream_mode": "native",
        });
        let cfg: SlackAccountConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.stream_mode, StreamMode::Native);
    }

    #[test]
    fn debug_redacts_tokens() {
        let cfg = SlackAccountConfig {
            bot_token: Secret::new("super-secret-bot".into()),
            app_token: Secret::new("super-secret-app".into()),
            ..Default::default()
        };
        let debug = format!("{cfg:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("super-secret-bot"));
        assert!(!debug.contains("super-secret-app"));
    }

    #[test]
    fn defaults_are_sensible() {
        let cfg = SlackAccountConfig::default();
        assert_eq!(cfg.connection_mode, ConnectionMode::SocketMode);
        assert!(cfg.signing_secret.is_none());
        assert_eq!(cfg.stream_mode, StreamMode::EditInPlace);
        assert_eq!(cfg.edit_throttle_ms, 500);
        assert!(cfg.thread_replies);
        assert_eq!(cfg.mention_mode, MentionMode::Mention);
        assert!(cfg.channel_overrides.is_empty());
        assert!(cfg.user_overrides.is_empty());
    }

    #[test]
    fn connection_mode_events_api_round_trip() {
        let json = serde_json::json!({
            "bot_token": "xoxb-test",
            "app_token": "",
            "connection_mode": "events_api",
            "signing_secret": "abc123secret",
        });
        let cfg: SlackAccountConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.connection_mode, ConnectionMode::EventsApi);
        assert_eq!(
            cfg.signing_secret.as_ref().unwrap().expose_secret(),
            "abc123secret"
        );

        // Round-trip
        let value = serde_json::to_value(&cfg).unwrap();
        let cfg2: SlackAccountConfig = serde_json::from_value(value).unwrap();
        assert_eq!(cfg2.connection_mode, ConnectionMode::EventsApi);
        assert_eq!(
            cfg2.signing_secret.as_ref().unwrap().expose_secret(),
            "abc123secret"
        );
    }

    #[test]
    fn debug_redacts_signing_secret() {
        let cfg = SlackAccountConfig {
            signing_secret: Some(Secret::new("very-secret".into())),
            ..Default::default()
        };
        let debug = format!("{cfg:?}");
        assert!(!debug.contains("very-secret"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn resolve_model_user_overrides_channel() {
        let mut cfg = SlackAccountConfig {
            model: Some("default-model".into()),
            ..Default::default()
        };
        cfg.channel_overrides
            .insert("C123".into(), ChannelOverride {
                model: Some("channel-model".into()),
                ..Default::default()
            });
        cfg.user_overrides.insert("U456".into(), UserOverride {
            model: Some("user-model".into()),
            ..Default::default()
        });

        // User override wins
        assert_eq!(cfg.resolve_model("C123", "U456"), Some("user-model"));
        // Channel override wins when no user override
        assert_eq!(cfg.resolve_model("C123", "U999"), Some("channel-model"));
        // Account default when no overrides
        assert_eq!(cfg.resolve_model("C999", "U999"), Some("default-model"));
    }

    #[test]
    fn overrides_round_trip() {
        let json = serde_json::json!({
            "bot_token": "xoxb-test",
            "app_token": "xapp-test",
            "channel_overrides": {
                "C123": { "model": "gpt-4" }
            },
            "user_overrides": {
                "U456": { "model": "claude-sonnet", "model_provider": "anthropic" }
            }
        });
        let cfg: SlackAccountConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.channel_model("C123"), Some("gpt-4"));
        assert!(cfg.channel_model_provider("C123").is_none());
        assert_eq!(cfg.user_model("U456"), Some("claude-sonnet"));
        assert_eq!(cfg.user_model_provider("U456"), Some("anthropic"));

        // Round-trip preserves overrides
        let value = serde_json::to_value(&cfg).unwrap();
        let cfg2: SlackAccountConfig = serde_json::from_value(value).unwrap();
        assert_eq!(cfg2.channel_model("C123"), Some("gpt-4"));
        assert_eq!(cfg2.user_model("U456"), Some("claude-sonnet"));
    }
}
