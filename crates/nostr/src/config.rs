//! Nostr channel account configuration.

use {
    moltis_channels::{
        config_view::ChannelConfigView,
        gating::{DmPolicy, GroupPolicy, MentionMode},
    },
    moltis_common::secret_serde,
    nostr_sdk::prelude::PublicKey,
    secrecy::Secret,
    serde::{Deserialize, Serialize, ser::SerializeStruct},
};

use crate::groups::GroupMessageKind;

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

    /// Exact sender IDs allowed to run privileged channel commands.
    /// Empty grants nobody privileged access.
    #[serde(default)]
    pub operators: Vec<String>,

    /// NIP-29 group ids (the `h` tag values) to join — e.g. Buzz channels.
    ///
    /// This list is authoritative for group access: it is both the set we
    /// subscribe to and the set we accept messages from. Empty (the default)
    /// keeps the account in DM-only mode. Changing it takes effect on the next
    /// account restart (relay subscriptions are fixed at connect).
    pub groups: Vec<String>,

    /// When the bot should respond in group chats: `mention` (only when its
    /// pubkey is `p`-tagged — the default), `always`, or `none`.
    pub group_mention_mode: MentionMode,

    /// Acknowledge inbound group messages with reactions: 👀 on receipt, a phase
    /// glyph per tool the agent runs (🌐 💻 ✏️ …), then ✅ or ❌ when the turn
    /// finishes (NIP-25 `kind:7`, each retracted via NIP-09).
    pub group_ack_reactions: bool,

    /// Dialect for group messages that are not replies: `nip29` (`kind:9`) or
    /// `buzz_v2` (`kind:40002`).
    ///
    /// Both kinds are always *read*, and replies mirror whatever kind they
    /// answer, so this only affects cold-start sends into a group we have not
    /// yet received a message from. Set it to `buzz_v2` for a Buzz workspace.
    pub group_message_kind: GroupMessageKind,

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

    /// Agent ID override for this account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,

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
            operators: Vec::new(),
            groups: Vec::new(),
            group_mention_mode: MentionMode::Mention,
            group_ack_reactions: true,
            group_message_kind: GroupMessageKind::default(),
            enabled: true,
            profile: None,
            model: None,
            model_provider: None,
            agent_id: None,
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
            .field("operators", &self.operators)
            .field("groups", &self.groups)
            .field("group_mention_mode", &self.group_mention_mode)
            .field("group_ack_reactions", &self.group_ack_reactions)
            .field("group_message_kind", &self.group_message_kind)
            .field("enabled", &self.enabled)
            .field("profile", &self.profile)
            .field("model", &self.model)
            .field("model_provider", &self.model_provider)
            .field("agent_id", &self.agent_id)
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
        // Must equal the number of unconditional `serialize_field` calls below;
        // self-describing formats emit the declared length verbatim.
        let mut count = 15;
        count += c.agent_id.is_some() as usize;
        let mut s = serializer.serialize_struct("NostrAccountConfig", count)?;
        s.serialize_field("secret_key", "[REDACTED]")?;
        s.serialize_field("relays", &c.relays)?;
        s.serialize_field("dm_policy", &c.dm_policy)?;
        s.serialize_field("allowed_pubkeys", &c.allowed_pubkeys)?;
        s.serialize_field("operators", &c.operators)?;
        s.serialize_field("groups", &c.groups)?;
        s.serialize_field("group_mention_mode", &c.group_mention_mode)?;
        s.serialize_field("group_ack_reactions", &c.group_ack_reactions)?;
        s.serialize_field("group_message_kind", &c.group_message_kind)?;
        s.serialize_field("enabled", &c.enabled)?;
        s.serialize_field("profile", &c.profile)?;
        s.serialize_field("model", &c.model)?;
        s.serialize_field("model_provider", &c.model_provider)?;
        if c.agent_id.is_some() {
            s.serialize_field("agent_id", &c.agent_id)?;
        }
        s.serialize_field("otp_self_approval", &c.otp_self_approval)?;
        s.serialize_field("otp_cooldown_secs", &c.otp_cooldown_secs)?;
        s.end()
    }
}

impl ChannelConfigView for NostrAccountConfig {
    fn allowlist(&self) -> &[String] {
        &self.allowed_pubkeys
    }

    fn operators(&self) -> &[String] {
        &self.operators
    }

    fn group_allowlist(&self) -> &[String] {
        // NIP-29 group ids the bot joins (e.g. Buzz channels).
        &self.groups
    }

    fn dm_policy(&self) -> DmPolicy {
        self.dm_policy.clone()
    }

    fn group_policy(&self) -> GroupPolicy {
        // `groups` is the join list *and* the allowlist — there is no "open"
        // mode, because we only ever subscribe to the groups named here.
        if self.groups.is_empty() {
            GroupPolicy::Disabled
        } else {
            GroupPolicy::Allowlist
        }
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

    /// A Nostr identity has two textual spellings — `npub1…` bech32 and 64-char
    /// hex — and both are valid in `allowed_pubkeys`. The channel always reports
    /// the sender as hex, so plain string comparison locks out every operator
    /// who configured the npub form the docs and web UI show. Compare parsed
    /// keys so the two are interchangeable, then fall back to the textual
    /// default, which is what keeps glob entries working.
    fn sender_on_allowlist(&self, sender_id: &str) -> bool {
        if let Ok(sender) = PublicKey::parse(sender_id)
            && self
                .allowed_pubkeys
                .iter()
                .filter_map(|entry| PublicKey::parse(entry).ok())
                .any(|allowed| allowed == sender)
        {
            return true;
        }
        moltis_channels::gating::sender_matches_allowlist(sender_id, &self.allowed_pubkeys)
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

    /// `/approve` and friends authorize against this, and the channel always
    /// reports the sender as hex — so an operator who configured the npub form
    /// (what the docs and the web UI show) must still be recognised.
    #[test]
    fn allowlist_accepts_either_spelling_of_the_same_key() {
        use nostr_sdk::prelude::{Keys, ToBech32};

        let operator = Keys::generate().public_key();
        let hex = operator.to_hex();
        let npub = operator.to_bech32().unwrap_or_default();
        assert!(npub.starts_with("npub1"), "precondition: bech32 form");

        for configured in [&hex, &npub] {
            let cfg = NostrAccountConfig {
                allowed_pubkeys: vec![configured.clone()],
                ..Default::default()
            };
            assert!(
                cfg.sender_on_allowlist(&hex),
                "sender hex must match an allowlist written as {configured}"
            );
        }
    }

    #[test]
    fn allowlist_rejects_a_different_key() {
        let cfg = NostrAccountConfig {
            allowed_pubkeys: vec![nostr_sdk::prelude::Keys::generate().public_key().to_hex()],
            ..Default::default()
        };
        let stranger = nostr_sdk::prelude::Keys::generate().public_key().to_hex();
        assert!(!cfg.sender_on_allowlist(&stranger));
    }

    /// Parsed comparison must not cost the textual features: a glob entry is
    /// not a key, so it has to keep falling through to string matching.
    #[test]
    fn allowlist_still_honours_glob_entries() {
        let cfg = NostrAccountConfig {
            allowed_pubkeys: vec!["npub1abc*".into()],
            ..Default::default()
        };
        assert!(cfg.sender_on_allowlist("npub1abcdef"));
        assert!(!cfg.sender_on_allowlist("npub1zzz"));
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

    #[test]
    fn default_group_config_is_dm_only() {
        let cfg = NostrAccountConfig::default();
        assert!(cfg.groups.is_empty());
        assert_eq!(cfg.group_mention_mode, MentionMode::Mention);
        // Group policy reports Disabled until a group is configured.
        assert_eq!(cfg.group_policy(), GroupPolicy::Disabled);
    }

    /// The join list doubles as the allowlist — there is no "open" group mode.
    #[test]
    fn configured_groups_expose_policy_and_allowlist() {
        let cfg = NostrAccountConfig {
            groups: vec!["buzz-general".into(), "buzz-dev".into()],
            group_mention_mode: MentionMode::Always,
            ..Default::default()
        };
        assert_eq!(cfg.group_policy(), GroupPolicy::Allowlist);
        assert_eq!(cfg.group_allowlist(), &["buzz-general", "buzz-dev"]);
    }

    #[test]
    fn groups_round_trip_json() {
        let cfg = NostrAccountConfig {
            secret_key: Secret::new("deadbeef".repeat(8)),
            groups: vec!["buzz-general".into()],
            group_mention_mode: MentionMode::Always,
            ..Default::default()
        };
        let json = serde_json::to_value(&cfg).unwrap_or_default();
        let parsed: NostrAccountConfig = serde_json::from_value(json).unwrap_or_default();
        assert_eq!(parsed.groups, vec!["buzz-general"]);
        assert_eq!(parsed.group_mention_mode, MentionMode::Always);
    }

    /// The declared struct length is not observable in JSON, so pin the field
    /// count instead: adding a field without bumping `count` desynchronises the
    /// two, which self-describing formats do surface.
    #[test]
    fn redacted_config_field_count_matches_declaration() {
        let base = serde_json::to_value(RedactedConfig(&NostrAccountConfig::default()))
            .unwrap_or_default();
        assert_eq!(
            base.as_object().map(serde_json::Map::len),
            Some(15),
            "keep `count` in RedactedConfig::serialize in sync"
        );

        let with_agent = NostrAccountConfig {
            agent_id: Some("agent".into()),
            ..Default::default()
        };
        let json = serde_json::to_value(RedactedConfig(&with_agent)).unwrap_or_default();
        assert_eq!(json.as_object().map(serde_json::Map::len), Some(16));
    }

    #[test]
    fn redacted_config_includes_group_fields() {
        let cfg = NostrAccountConfig {
            groups: vec!["buzz-general".into()],
            ..Default::default()
        };
        let json = serde_json::to_value(RedactedConfig(&cfg)).unwrap_or_default();
        assert_eq!(json["secret_key"], "[REDACTED]");
        assert!(json["groups"].is_array());
        assert!(json.get("group_mention_mode").is_some());
    }
}
