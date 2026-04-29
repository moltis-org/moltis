//! Telephony account configuration.

use {
    moltis_channels::{
        config_view::ChannelConfigView,
        gating::{DmPolicy, GroupPolicy},
    },
    secrecy::Secret,
    serde::{Deserialize, Serialize},
};

/// Which telephony provider backend to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TelephonyProviderId {
    #[default]
    Twilio,
    Telnyx,
    Plivo,
}

/// Inbound call access policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum InboundPolicy {
    /// No inbound calls accepted.
    #[default]
    Disabled,
    /// Only allowlisted phone numbers.
    Allowlist,
    /// Anyone can call (open).
    Open,
}

/// Configuration for a single telephony account.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TelephonyAccountConfig {
    /// Provider backend.
    pub provider: TelephonyProviderId,

    /// Outbound caller ID (E.164 format, e.g. "+15551234567").
    pub from_number: String,

    /// Default destination number for outbound calls (optional).
    pub to_number: Option<String>,

    // ── Provider credentials ──
    /// Twilio Account SID.
    #[serde(default, skip_serializing_if = "Option::is_none", skip_serializing)]
    pub account_sid: Option<Secret<String>>,

    /// Twilio Auth Token.
    #[serde(default, skip_serializing_if = "Option::is_none", skip_serializing)]
    pub auth_token: Option<Secret<String>>,

    // ── Webhook settings ──
    /// Public URL where the provider can reach our webhook.
    /// Required for inbound calls and call status callbacks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_url: Option<String>,

    /// Port for the webhook listener (default: 3334).
    #[serde(default = "default_webhook_port")]
    pub webhook_port: u16,

    // ── Call settings ──
    /// Maximum call duration in seconds (default: 3600 = 1 hour).
    #[serde(default = "default_max_duration")]
    pub max_duration_secs: u64,

    /// Delay in seconds before hanging up in notify mode (default: 3).
    #[serde(default = "default_notify_hangup_delay")]
    pub notify_hangup_delay_secs: u64,

    // ── Access control ──
    /// Inbound call policy.
    pub inbound_policy: InboundPolicy,

    /// Allowlisted phone numbers (E.164 format).
    #[serde(default)]
    pub allowlist: Vec<String>,

    // ── Voice settings ──
    /// TTS voice ID to use for bot speech.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_id: Option<String>,

    /// TTS provider override (uses gateway default if unset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tts_provider: Option<String>,

    // ── Agent routing ──
    /// Model override for calls on this account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Model provider override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,

    /// Agent ID to use for call conversations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

fn default_webhook_port() -> u16 {
    3334
}
fn default_max_duration() -> u64 {
    3600
}
fn default_notify_hangup_delay() -> u64 {
    3
}

impl Default for TelephonyAccountConfig {
    fn default() -> Self {
        Self {
            provider: TelephonyProviderId::default(),
            from_number: String::new(),
            to_number: None,
            account_sid: None,
            auth_token: None,
            webhook_url: None,
            webhook_port: default_webhook_port(),
            max_duration_secs: default_max_duration(),
            notify_hangup_delay_secs: default_notify_hangup_delay(),
            inbound_policy: InboundPolicy::default(),
            allowlist: Vec::new(),
            voice_id: None,
            tts_provider: None,
            model: None,
            model_provider: None,
            agent_id: None,
        }
    }
}

impl ChannelConfigView for TelephonyAccountConfig {
    fn allowlist(&self) -> &[String] {
        &self.allowlist
    }

    fn group_allowlist(&self) -> &[String] {
        &[]
    }

    fn dm_policy(&self) -> DmPolicy {
        match self.inbound_policy {
            InboundPolicy::Disabled => DmPolicy::Disabled,
            InboundPolicy::Allowlist => DmPolicy::Allowlist,
            InboundPolicy::Open => DmPolicy::Open,
        }
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

    fn agent_id(&self) -> Option<&str> {
        self.agent_id.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_sensible_values() {
        let cfg = TelephonyAccountConfig::default();
        assert_eq!(cfg.provider, TelephonyProviderId::Twilio);
        assert_eq!(cfg.webhook_port, 3334);
        assert_eq!(cfg.max_duration_secs, 3600);
        assert_eq!(cfg.notify_hangup_delay_secs, 3);
        assert_eq!(cfg.inbound_policy, InboundPolicy::Disabled);
    }

    #[test]
    fn config_roundtrip_json() {
        let cfg = TelephonyAccountConfig {
            from_number: "+15551234567".into(),
            ..Default::default()
        };
        let json = serde_json::to_value(&cfg).unwrap_or_default();
        let parsed: TelephonyAccountConfig = serde_json::from_value(json).unwrap_or_default();
        assert_eq!(parsed.from_number, "+15551234567");
    }

    #[test]
    fn dm_policy_maps_from_inbound_policy() {
        let cfg = TelephonyAccountConfig {
            inbound_policy: InboundPolicy::Open,
            ..Default::default()
        };
        assert_eq!(cfg.dm_policy(), DmPolicy::Open);
    }
}
