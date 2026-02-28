use {
    moltis_channels::gating::{DmPolicy, GroupPolicy},
    serde::{Deserialize, Serialize},
    std::path::PathBuf,
};

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

    /// DM access policy.
    pub dm_policy: DmPolicy,

    /// Group access policy.
    pub group_policy: GroupPolicy,

    /// User/peer allowlist for DMs (JID user parts or phone numbers).
    pub allowlist: Vec<String>,

    /// Group JID allowlist.
    pub group_allowlist: Vec<String>,

    /// Enable OTP self-approval for non-allowlisted DM users (default: true).
    pub otp_self_approval: bool,

    /// Cooldown in seconds after 3 failed OTP attempts (default: 300).
    pub otp_cooldown_secs: u64,
}

impl std::fmt::Debug for WhatsAppAccountConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WhatsAppAccountConfig")
            .field("paired", &self.paired)
            .field("display_name", &self.display_name)
            .field("dm_policy", &self.dm_policy)
            .field("group_policy", &self.group_policy)
            .finish_non_exhaustive()
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
            dm_policy: DmPolicy::default(),
            group_policy: GroupPolicy::default(),
            allowlist: Vec::new(),
            group_allowlist: Vec::new(),
            otp_self_approval: true,
            otp_cooldown_secs: 300,
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
        assert_eq!(cfg.dm_policy, DmPolicy::Allowlist);
        assert_eq!(cfg.group_policy, GroupPolicy::Open);
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
            "phone_number": "+15551234567"
        }"#;
        let cfg: WhatsAppAccountConfig = serde_json::from_str(json).unwrap();
        assert!(cfg.paired);
        assert_eq!(cfg.display_name.as_deref(), Some("My Phone"));
        assert_eq!(cfg.phone_number.as_deref(), Some("+15551234567"));
        // Defaults for access control fields
        assert_eq!(cfg.dm_policy, DmPolicy::Allowlist);
        assert!(cfg.allowlist.is_empty());
    }

    #[test]
    fn deserialize_with_access_control() {
        let json = r#"{
            "dm_policy": "allowlist",
            "group_policy": "disabled",
            "allowlist": ["user1", "user2"],
            "group_allowlist": ["group1"],
            "otp_self_approval": false,
            "otp_cooldown_secs": 600
        }"#;
        let cfg: WhatsAppAccountConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.dm_policy, DmPolicy::Allowlist);
        assert_eq!(cfg.group_policy, GroupPolicy::Disabled);
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
}
