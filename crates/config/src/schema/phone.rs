use {
    secrecy::Secret,
    serde::{Deserialize, Serialize},
};

/// Phone call configuration (telephony providers).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PhoneConfig {
    /// Enable phone calls globally.
    pub enabled: bool,
    /// Active provider: "twilio" (more coming).
    pub provider: String,
    /// Provider IDs to list in the UI. Empty means list all.
    pub providers: Vec<String>,
    /// Default inbound call policy: "disabled", "allowlist", "open".
    pub inbound_policy: String,
    /// Allowlisted phone numbers (E.164).
    #[serde(default)]
    pub allowlist: Vec<String>,
    /// Maximum call duration in seconds.
    #[serde(default = "default_max_duration")]
    pub max_duration_secs: u64,
    /// Twilio-specific settings.
    pub twilio: PhoneTwilioConfig,
    /// Telnyx-specific settings.
    pub telnyx: PhoneTelnyxConfig,
    /// Plivo-specific settings.
    pub plivo: PhonePlivoConfig,
}

fn default_max_duration() -> u64 {
    3600
}

impl Default for PhoneConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: String::new(),
            providers: Vec::new(),
            inbound_policy: "disabled".to_string(),
            allowlist: Vec::new(),
            max_duration_secs: default_max_duration(),
            twilio: PhoneTwilioConfig::default(),
            telnyx: PhoneTelnyxConfig::default(),
            plivo: PhonePlivoConfig::default(),
        }
    }
}

/// Twilio provider configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PhoneTwilioConfig {
    /// Twilio Account SID.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::schema::serialize_option_secret",
        deserialize_with = "crate::schema::deserialize_option_secret"
    )]
    pub account_sid: Option<Secret<String>>,
    /// Twilio Auth Token.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::schema::serialize_option_secret",
        deserialize_with = "crate::schema::deserialize_option_secret"
    )]
    pub auth_token: Option<Secret<String>>,
    /// Phone number (E.164 format).
    pub from_number: Option<String>,
    /// Public webhook URL for Twilio callbacks.
    pub webhook_url: Option<String>,
}

/// Telnyx provider configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PhoneTelnyxConfig {
    /// Telnyx API key (v2).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::schema::serialize_option_secret",
        deserialize_with = "crate::schema::deserialize_option_secret"
    )]
    pub api_key: Option<Secret<String>>,
    /// Telnyx Call Control connection ID.
    pub connection_id: Option<String>,
    /// Ed25519 public key for webhook signature verification.
    pub public_key: Option<String>,
    /// Phone number (E.164 format).
    pub from_number: Option<String>,
    /// Public webhook URL for Telnyx callbacks.
    pub webhook_url: Option<String>,
}

/// Plivo provider configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PhonePlivoConfig {
    /// Plivo Auth ID.
    pub auth_id: Option<String>,
    /// Plivo Auth Token.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::schema::serialize_option_secret",
        deserialize_with = "crate::schema::deserialize_option_secret"
    )]
    pub auth_token: Option<Secret<String>>,
    /// Phone number (E.164 format).
    pub from_number: Option<String>,
    /// Public webhook URL for Plivo callbacks.
    pub webhook_url: Option<String>,
}
