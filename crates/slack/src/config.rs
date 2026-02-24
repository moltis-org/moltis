use {
    moltis_channels::gating::{DmPolicy, GroupPolicy, MentionMode},
    secrecy::{ExposeSecret, Secret},
    serde::{Deserialize, Serialize},
};

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

/// Connection mode for Slack.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionMode {
    /// Socket Mode (WebSocket-based, no public endpoint needed).
    #[default]
    Socket,
    // Future: EventsApi for HTTP webhook-based connection
}

/// Activation mode in channels.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivationMode {
    /// Respond only when @mentioned.
    #[default]
    Mention,
    /// Respond to all messages in allowed channels.
    Always,
    /// Only respond in threads where bot is a participant.
    ThreadOnly,
}

/// Configuration for a single Slack workspace/bot account.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SlackAccountConfig {
    /// Bot User OAuth Token (xoxb-...).
    #[serde(serialize_with = "serialize_secret")]
    pub bot_token: Secret<String>,

    /// App-Level Token for Socket Mode (xapp-...).
    #[serde(serialize_with = "serialize_secret")]
    pub app_token: Secret<String>,

    /// Connection mode.
    pub mode: ConnectionMode,

    /// DM access policy.
    pub dm_policy: DmPolicy,

    /// Channel/group access policy.
    pub channel_policy: GroupPolicy,

    /// Mention activation mode for channels.
    pub mention_mode: MentionMode,

    /// User ID allowlist for DMs (supports glob patterns).
    pub user_allowlist: Vec<String>,

    /// Channel ID allowlist (supports glob patterns).
    pub channel_allowlist: Vec<String>,

    /// How streaming responses are delivered.
    pub stream_mode: StreamMode,

    /// Minimum interval between edit-in-place updates (ms).
    pub edit_throttle_ms: u64,

    /// Default model ID for this account's sessions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Provider name associated with `model`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,

    /// Reply in thread by default.
    pub thread_replies: bool,

    /// History limit for channel context (0 = disabled).
    pub history_limit: usize,
}

impl std::fmt::Debug for SlackAccountConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlackAccountConfig")
            .field("bot_token", &"[REDACTED]")
            .field("app_token", &"[REDACTED]")
            .field("mode", &self.mode)
            .field("dm_policy", &self.dm_policy)
            .field("channel_policy", &self.channel_policy)
            .finish_non_exhaustive()
    }
}

fn serialize_secret<S: serde::Serializer>(
    secret: &Secret<String>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(secret.expose_secret())
}

impl Default for SlackAccountConfig {
    fn default() -> Self {
        Self {
            bot_token: Secret::new(String::new()),
            app_token: Secret::new(String::new()),
            mode: ConnectionMode::default(),
            dm_policy: DmPolicy::default(),
            channel_policy: GroupPolicy::default(),
            mention_mode: MentionMode::default(),
            user_allowlist: Vec::new(),
            channel_allowlist: Vec::new(),
            stream_mode: StreamMode::default(),
            edit_throttle_ms: 500,
            model: None,
            model_provider: None,
            thread_replies: true,
            history_limit: 0,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let cfg = SlackAccountConfig::default();
        assert_eq!(cfg.dm_policy, DmPolicy::Allowlist);
        assert_eq!(cfg.channel_policy, GroupPolicy::Open);
        assert_eq!(cfg.mention_mode, MentionMode::Mention);
        assert_eq!(cfg.stream_mode, StreamMode::EditInPlace);
        assert_eq!(cfg.edit_throttle_ms, 500);
        assert!(cfg.thread_replies);
    }

    #[test]
    fn deserialize_from_json() {
        let json = r#"{
            "bot_token": "xoxb-test",
            "app_token": "xapp-test",
            "dm_policy": "allowlist",
            "stream_mode": "off",
            "user_allowlist": ["U123", "U456"]
        }"#;
        let cfg: SlackAccountConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.bot_token.expose_secret(), "xoxb-test");
        assert_eq!(cfg.app_token.expose_secret(), "xapp-test");
        assert_eq!(cfg.dm_policy, DmPolicy::Allowlist);
        assert_eq!(cfg.stream_mode, StreamMode::Off);
        assert_eq!(cfg.user_allowlist, vec!["U123", "U456"]);
        // defaults for unspecified fields
        assert_eq!(cfg.channel_policy, GroupPolicy::Open);
    }

    #[test]
    fn serialize_roundtrip() {
        let cfg = SlackAccountConfig {
            bot_token: Secret::new("xoxb-tok".into()),
            app_token: Secret::new("xapp-tok".into()),
            dm_policy: DmPolicy::Disabled,
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let cfg2: SlackAccountConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg2.dm_policy, DmPolicy::Disabled);
        assert_eq!(cfg2.bot_token.expose_secret(), "xoxb-tok");
    }
}
