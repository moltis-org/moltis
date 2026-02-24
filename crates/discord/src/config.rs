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

/// Configuration for a single Discord bot account.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscordAccountConfig {
    /// Bot token from Discord Developer Portal.
    #[serde(serialize_with = "serialize_secret")]
    pub token: Secret<String>,

    /// DM access policy.
    pub dm_policy: DmPolicy,

    /// Guild (server) access policy.
    pub guild_policy: GroupPolicy,

    /// Mention activation mode for guild channels.
    pub mention_mode: MentionMode,

    /// User ID allowlist for DMs (supports glob patterns).
    pub user_allowlist: Vec<String>,

    /// Guild ID allowlist.
    pub guild_allowlist: Vec<String>,

    /// Channel ID allowlist (within allowed guilds).
    pub channel_allowlist: Vec<String>,

    /// Role name allowlist (users with these roles can interact).
    pub role_allowlist: Vec<String>,

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

    /// Use embeds for responses (richer formatting).
    pub use_embeds: bool,

    /// History limit for guild channel context (0 = disabled).
    pub history_limit: usize,
}

impl std::fmt::Debug for DiscordAccountConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscordAccountConfig")
            .field("token", &"[REDACTED]")
            .field("dm_policy", &self.dm_policy)
            .field("guild_policy", &self.guild_policy)
            .finish_non_exhaustive()
    }
}

fn serialize_secret<S: serde::Serializer>(
    secret: &Secret<String>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(secret.expose_secret())
}

impl Default for DiscordAccountConfig {
    fn default() -> Self {
        Self {
            token: Secret::new(String::new()),
            dm_policy: DmPolicy::default(),
            guild_policy: GroupPolicy::default(),
            mention_mode: MentionMode::default(),
            user_allowlist: Vec::new(),
            guild_allowlist: Vec::new(),
            channel_allowlist: Vec::new(),
            role_allowlist: Vec::new(),
            stream_mode: StreamMode::default(),
            edit_throttle_ms: 500,
            model: None,
            model_provider: None,
            use_embeds: true,
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
        let cfg = DiscordAccountConfig::default();
        assert_eq!(cfg.dm_policy, DmPolicy::Allowlist);
        assert_eq!(cfg.guild_policy, GroupPolicy::Open);
        assert_eq!(cfg.mention_mode, MentionMode::Mention);
        assert_eq!(cfg.stream_mode, StreamMode::EditInPlace);
        assert_eq!(cfg.edit_throttle_ms, 500);
        assert!(cfg.use_embeds);
    }

    #[test]
    fn deserialize_from_json() {
        let json = r#"{
            "token": "MTIz.abc.xyz",
            "dm_policy": "allowlist",
            "stream_mode": "off",
            "user_allowlist": ["123456789", "987654321"],
            "guild_allowlist": ["111222333"]
        }"#;
        let cfg: DiscordAccountConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.token.expose_secret(), "MTIz.abc.xyz");
        assert_eq!(cfg.dm_policy, DmPolicy::Allowlist);
        assert_eq!(cfg.stream_mode, StreamMode::Off);
        assert_eq!(cfg.user_allowlist, vec!["123456789", "987654321"]);
        assert_eq!(cfg.guild_allowlist, vec!["111222333"]);
        // defaults for unspecified fields
        assert_eq!(cfg.guild_policy, GroupPolicy::Open);
    }

    #[test]
    fn serialize_roundtrip() {
        let cfg = DiscordAccountConfig {
            token: Secret::new("tok".into()),
            dm_policy: DmPolicy::Disabled,
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let cfg2: DiscordAccountConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg2.dm_policy, DmPolicy::Disabled);
        assert_eq!(cfg2.token.expose_secret(), "tok");
    }
}
