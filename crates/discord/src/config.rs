use {
    moltis_channels::gating::{DmPolicy, GroupPolicy, MentionMode},
    secrecy::{ExposeSecret, Secret},
    serde::{Deserialize, Serialize},
};

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
}
