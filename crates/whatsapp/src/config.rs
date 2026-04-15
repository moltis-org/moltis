use {
    moltis_channels::{
        config_view::ChannelConfigView,
        gating::{DmPolicy, GroupPolicy, MentionMode},
    },
    serde::{Deserialize, Serialize},
    std::{collections::HashMap, path::PathBuf},
};

/// Per-chat model/provider/agent override.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

/// Configuration for a single WhatsApp account.
///
/// Unlike Telegram, WhatsApp uses Linked Devices (QR code pairing) so no
/// bot token is needed. The Signal Protocol session state is persisted in a
/// per-account store.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WhatsAppAccountConfig {
    /// Path to the store for this account's Signal Protocol sessions.
    /// Defaults to `<data_dir>/whatsapp/<account_id>/`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_path: Option<PathBuf>,

    /// Display name populated after successful pairing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,

    /// Phone number populated after successful pairing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone_number: Option<String>,

    /// Whether this account has been paired (QR code scanned).
    pub paired: bool,

    /// Default model ID for this account's sessions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Provider name associated with `model`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,

    /// Default agent ID for this account's sessions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,

    /// DM access policy.
    pub dm_policy: DmPolicy,

    /// Group access policy.
    pub group_policy: GroupPolicy,

    /// Mention activation mode for groups.
    pub mention_mode: MentionMode,

    /// User/peer allowlist for DMs (JID user parts or phone numbers).
    pub allowlist: Vec<String>,

    /// Group JID allowlist.
    pub group_allowlist: Vec<String>,

    /// Enable OTP self-approval for non-allowlisted DM users (default: true).
    pub otp_self_approval: bool,

    /// Cooldown in seconds after 3 failed OTP attempts (default: 300).
    pub otp_cooldown_secs: u64,

    /// Per-group/chat overrides keyed by WhatsApp chat JID.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub channel_overrides: HashMap<String, ChatOverride>,

    /// Per-user overrides keyed by sender JID.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub user_overrides: HashMap<String, ChatOverride>,
}

impl std::fmt::Debug for WhatsAppAccountConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WhatsAppAccountConfig")
            .field("paired", &self.paired)
            .field("display_name", &self.display_name)
            .field("dm_policy", &self.dm_policy)
            .field("group_policy", &self.group_policy)
            .field("mention_mode", &self.mention_mode)
            .field("channel_overrides", &self.channel_overrides)
            .field("user_overrides", &self.user_overrides)
            .finish_non_exhaustive()
    }
}

impl ChannelConfigView for WhatsAppAccountConfig {
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

impl Default for WhatsAppAccountConfig {
    fn default() -> Self {
        Self {
            store_path: None,
            display_name: None,
            phone_number: None,
            paired: false,
            model: None,
            model_provider: None,
            agent_id: None,
            dm_policy: DmPolicy::default(),
            group_policy: GroupPolicy::default(),
            mention_mode: MentionMode::Always,
            allowlist: Vec::new(),
            group_allowlist: Vec::new(),
            otp_self_approval: true,
            otp_cooldown_secs: 300,
            channel_overrides: HashMap::new(),
            user_overrides: HashMap::new(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let cfg = WhatsAppAccountConfig::default();
        assert!(!cfg.paired);
        assert!(cfg.store_path.is_none());
        assert!(cfg.display_name.is_none());
        assert!(cfg.model.is_none());
        assert!(cfg.agent_id.is_none());
        assert_eq!(cfg.dm_policy, DmPolicy::Allowlist);
        assert_eq!(cfg.group_policy, GroupPolicy::Open);
        assert_eq!(cfg.mention_mode, MentionMode::Always);
        assert!(cfg.allowlist.is_empty());
        assert!(cfg.group_allowlist.is_empty());
        assert!(cfg.otp_self_approval);
        assert_eq!(cfg.otp_cooldown_secs, 300);
    }

    #[test]
    fn deserialize_from_json() {
        let json = r#"{
            "paired": true,
            "display_name": "My Phone",
            "phone_number": "+15551234567",
            "mention_mode": "mention"
        }"#;
        let cfg: WhatsAppAccountConfig = serde_json::from_str(json).unwrap();
        assert!(cfg.paired);
        assert_eq!(cfg.display_name.as_deref(), Some("My Phone"));
        assert_eq!(cfg.phone_number.as_deref(), Some("+15551234567"));
        assert_eq!(cfg.mention_mode, MentionMode::Mention);
        // Defaults for access control fields
        assert_eq!(cfg.dm_policy, DmPolicy::Allowlist);
        assert!(cfg.allowlist.is_empty());
    }

    #[test]
    fn deserialize_with_access_control() {
        let json = r#"{
            "dm_policy": "allowlist",
            "group_policy": "disabled",
            "mention_mode": "none",
            "allowlist": ["user1", "user2"],
            "group_allowlist": ["group1"],
            "otp_self_approval": false,
            "otp_cooldown_secs": 600
        }"#;
        let cfg: WhatsAppAccountConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.dm_policy, DmPolicy::Allowlist);
        assert_eq!(cfg.group_policy, GroupPolicy::Disabled);
        assert_eq!(cfg.mention_mode, MentionMode::None);
        assert_eq!(cfg.allowlist, vec!["user1", "user2"]);
        assert_eq!(cfg.group_allowlist, vec!["group1"]);
        assert!(!cfg.otp_self_approval);
        assert_eq!(cfg.otp_cooldown_secs, 600);
    }

    #[test]
    fn serialize_roundtrip() {
        let cfg = WhatsAppAccountConfig {
            paired: true,
            display_name: Some("Test".into()),
            dm_policy: DmPolicy::Allowlist,
            allowlist: vec!["alice".into()],
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let cfg2: WhatsAppAccountConfig = serde_json::from_str(&json).unwrap();
        assert!(cfg2.paired);
        assert_eq!(cfg2.display_name.as_deref(), Some("Test"));
        assert_eq!(cfg2.dm_policy, DmPolicy::Allowlist);
        assert_eq!(cfg2.allowlist, vec!["alice"]);
    }

    #[test]
    fn resolve_overrides_prefers_user_then_group_then_default() {
        let mut cfg = WhatsAppAccountConfig {
            model: Some("default-model".into()),
            agent_id: Some("default-agent".into()),
            ..Default::default()
        };
        cfg.channel_overrides
            .insert("120363456789@g.us".into(), ChatOverride {
                model: Some("group-model".into()),
                agent_id: Some("group-agent".into()),
                ..Default::default()
            });
        cfg.user_overrides
            .insert("15551234567@s.whatsapp.net".into(), ChatOverride {
                model: Some("user-model".into()),
                agent_id: Some("user-agent".into()),
                ..Default::default()
            });

        assert_eq!(
            cfg.resolve_model("120363456789@g.us", "15551234567@s.whatsapp.net"),
            Some("user-model")
        );
        assert_eq!(
            cfg.resolve_agent_id("120363456789@g.us", "15551234567@s.whatsapp.net"),
            Some("user-agent")
        );
        assert_eq!(
            cfg.resolve_model("120363456789@g.us", "missing@s.whatsapp.net"),
            Some("group-model")
        );
        assert_eq!(
            cfg.resolve_agent_id("120363456789@g.us", "missing@s.whatsapp.net"),
            Some("group-agent")
        );
        assert_eq!(
            cfg.resolve_agent_id("other@g.us", "missing@s.whatsapp.net"),
            Some("default-agent")
        );
    }
}
