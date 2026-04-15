//! Nostr channel account configuration.

use {
    moltis_channels::{
        config_view::ChannelConfigView,
        gating::{DmPolicy, GroupPolicy},
    },
    moltis_common::secret_serde,
    secrecy::Secret,
    serde::{Deserialize, Serialize, ser::SerializeStruct},
};

/// NIP-01 profile metadata to publish on connect.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NostrProfile {
    /// Bot display name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Longer display name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Short bio / about text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    /// Avatar URL (HTTPS).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    /// NIP-05 identifier (e.g. `bot@example.com`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nip05: Option<String>,
}

/// Configuration for a single Nostr account.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NostrAccountConfig {
    /// Secret key in `nsec1...` (bech32) or 64-char hex format.
    #[serde(serialize_with = "secret_serde::serialize_secret")]
    pub secret_key: Secret<String>,

    /// Relay WebSocket URLs (e.g. `wss://relay.damus.io`).
    pub relays: Vec<String>,

    /// DM access policy.
    pub dm_policy: DmPolicy,

    /// Public keys allowed to send DMs (npub1/hex).
    pub allowed_pubkeys: Vec<String>,

    /// Whether this account is enabled.
    pub enabled: bool,

    /// NIP-01 profile metadata to publish on connect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<NostrProfile>,

    /// Default model ID for sessions created from this account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Provider name associated with the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,

    /// Enable OTP self-approval for non-allowlisted DM users.
    pub otp_self_approval: bool,

    /// Cooldown in seconds after 3 failed OTP attempts.
    pub otp_cooldown_secs: u64,
}

impl Default for NostrAccountConfig {
    fn default() -> Self {
        Self {
            secret_key: Secret::new(String::new()),
            relays: default_relays(),
            dm_policy: DmPolicy::Allowlist,
            allowed_pubkeys: Vec::new(),
            enabled: true,
            profile: None,
            model: None,
            model_provider: None,
            otp_self_approval: true,
            otp_cooldown_secs: 300,
        }
    }
}

fn default_relays() -> Vec<String> {
    vec![
        "wss://relay.damus.io".into(),
        "wss://relay.nostr.band".into(),
        "wss://nos.lol".into(),
    ]
}

impl std::fmt::Debug for NostrAccountConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NostrAccountConfig")
            .field("secret_key", &"[REDACTED]")
            .field("relays", &self.relays)
            .field("dm_policy", &self.dm_policy)
            .field("allowed_pubkeys", &self.allowed_pubkeys)
            .field("enabled", &self.enabled)
            .field("profile", &self.profile)
            .field("model", &self.model)
            .field("model_provider", &self.model_provider)
            .field("otp_self_approval", &self.otp_self_approval)
            .field("otp_cooldown_secs", &self.otp_cooldown_secs)
            .finish()
    }
}

/// Wrapper that serializes secret fields as `[REDACTED]` for API responses.
pub struct RedactedConfig<'a>(pub &'a NostrAccountConfig);

impl Serialize for RedactedConfig<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let c = self.0;
        let mut s = serializer.serialize_struct("NostrAccountConfig", 10)?;
        s.serialize_field("secret_key", "[REDACTED]")?;
        s.serialize_field("relays", &c.relays)?;
        s.serialize_field("dm_policy", &c.dm_policy)?;
        s.serialize_field("allowed_pubkeys", &c.allowed_pubkeys)?;
        s.serialize_field("enabled", &c.enabled)?;
        s.serialize_field("profile", &c.profile)?;
        s.serialize_field("model", &c.model)?;
        s.serialize_field("model_provider", &c.model_provider)?;
        s.serialize_field("otp_self_approval", &c.otp_self_approval)?;
        s.serialize_field("otp_cooldown_secs", &c.otp_cooldown_secs)?;
        s.end()
    }
}

impl ChannelConfigView for NostrAccountConfig {
    fn allowlist(&self) -> &[String] {
        &self.allowed_pubkeys
    }

    fn group_allowlist(&self) -> &[String] {
        // Nostr DMs are always 1:1, no group concept
        &[]
    }

    fn dm_policy(&self) -> DmPolicy {
        self.dm_policy.clone()
    }

    fn group_policy(&self) -> GroupPolicy {
        GroupPolicy::Disabled
    }

    fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    fn model_provider(&self) -> Option<&str> {
        self.model_provider.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use secrecy::Secret;

    use super::*;

    #[test]
    fn default_config_has_sensible_values() {
        let cfg = NostrAccountConfig::default();
        assert_eq!(cfg.dm_policy, DmPolicy::Allowlist);
        assert_eq!(cfg.relays.len(), 3);
        assert!(cfg.enabled);
        assert!(cfg.otp_self_approval);
        assert_eq!(cfg.otp_cooldown_secs, 300);
    }

    #[test]
    fn redacted_config_hides_secret() {
        let cfg = NostrAccountConfig {
            secret_key: Secret::new("nsec1test".into()),
            ..Default::default()
        };
        let json = serde_json::to_value(RedactedConfig(&cfg));
        assert!(json.is_ok());
        let json = json.unwrap_or_default();
        assert_eq!(json["secret_key"], "[REDACTED]");
        assert!(json["relays"].is_array());
    }

    #[test]
    fn round_trip_json() {
        let cfg = NostrAccountConfig {
            secret_key: Secret::new("deadbeef".repeat(8)),
            relays: vec!["wss://test.relay".into()],
            dm_policy: DmPolicy::Open,
            allowed_pubkeys: vec!["npub1test".into()],
            ..Default::default()
        };
        let json = serde_json::to_value(&cfg);
        assert!(json.is_ok());
        let json = json.unwrap_or_default();
        let parsed: Result<NostrAccountConfig, _> = serde_json::from_value(json);
        assert!(parsed.is_ok());
        let parsed = parsed.unwrap_or_default();
        assert_eq!(parsed.dm_policy, DmPolicy::Open);
        assert_eq!(parsed.relays, vec!["wss://test.relay"]);
    }

    #[test]
    fn config_view_dm_only() {
        let cfg = NostrAccountConfig {
            model: Some("test-model".into()),
            model_provider: Some("test-provider".into()),
            ..Default::default()
        };
        assert_eq!(cfg.group_policy(), GroupPolicy::Disabled);
        assert!(cfg.group_allowlist().is_empty());
        assert_eq!(cfg.model(), Some("test-model"));
        assert_eq!(cfg.model_provider(), Some("test-provider"));
    }
}
