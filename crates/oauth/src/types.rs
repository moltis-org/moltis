use {
    secrecy::{ExposeSecret, Secret},
    serde::{Deserialize, Serialize},
};

/// OAuth 2.0 provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthConfig {
    pub client_id: String,
    pub auth_url: String,
    pub token_url: String,
    pub redirect_uri: String,
    /// Optional RFC 8707 resource indicator (sent to authorize + token endpoints).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Extra query parameters to include in the authorization URL.
    #[serde(default)]
    pub extra_auth_params: Vec<(String, String)>,
    /// If true, use the GitHub device-flow instead of PKCE authorization code flow.
    #[serde(default)]
    pub device_flow: bool,
}

/// Stored OAuth tokens.
#[derive(Clone, Serialize, Deserialize)]
pub struct OAuthTokens {
    #[serde(serialize_with = "serialize_secret")]
    pub access_token: Secret<String>,
    #[serde(
        default,
        serialize_with = "serialize_option_secret",
        skip_serializing_if = "Option::is_none"
    )]
    pub refresh_token: Option<Secret<String>>,
    #[serde(
        default,
        serialize_with = "serialize_option_secret",
        skip_serializing_if = "Option::is_none"
    )]
    pub id_token: Option<Secret<String>>,
    /// Provider-specific account identifier (for example ChatGPT account/org id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    /// Unix timestamp when the access token expires.
    pub expires_at: Option<u64>,
}

impl std::fmt::Debug for OAuthTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthTokens")
            .field("access_token", &"[REDACTED]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("id_token", &self.id_token.as_ref().map(|_| "[REDACTED]"))
            .field(
                "account_id",
                &self.account_id.as_ref().map(|_| "[REDACTED]"),
            )
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// PKCE challenge pair.
#[derive(Debug, Clone)]
pub struct PkceChallenge {
    pub verifier: String,
    pub challenge: String,
}

// ── Serde helpers for Secret<String> ────────────────────────────────────────

/// Serialize a `Secret<String>` by exposing its inner value.
/// Use only for fields that must round-trip through storage (config files, token JSON).
pub fn serialize_secret<S: serde::Serializer>(
    secret: &Secret<String>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(secret.expose_secret())
}

/// Serialize an `Option<Secret<String>>` by exposing its inner value.
pub fn serialize_option_secret<S: serde::Serializer>(
    secret: &Option<Secret<String>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match secret {
        Some(s) => serializer.serialize_some(s.expose_secret()),
        None => serializer.serialize_none(),
    }
}
