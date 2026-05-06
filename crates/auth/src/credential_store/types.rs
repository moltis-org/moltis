use serde::{Deserialize, Serialize};

/// Authentication method used to verify an identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMethod {
    Password,
    Passkey,
    ApiKey,
    Loopback,
}

/// A verified identity after successful authentication.
#[derive(Debug, Clone)]
pub struct AuthIdentity {
    pub method: AuthMethod,
    /// Scopes granted to this identity. Empty for full access (password,
    /// passkey, loopback). Populated for API keys with scope restrictions.
    pub scopes: Vec<String>,
}

impl AuthIdentity {
    /// Returns `true` if this identity has the given scope, or has
    /// unrestricted access (password/passkey/loopback or unscooped API key).
    pub fn has_scope(&self, scope: &str) -> bool {
        if self.method != AuthMethod::ApiKey {
            return true;
        }
        self.scopes.is_empty() || self.scopes.iter().any(|s| s == scope)
    }
}

/// A registered passkey entry (for listing in the UI).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasskeyEntry {
    pub id: i64,
    pub name: String,
    pub created_at: String,
}

/// An API key entry (for listing in the UI, never exposes the full key).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyEntry {
    pub id: i64,
    pub label: String,
    pub key_prefix: String,
    pub created_at: String,
    /// Scopes granted to this API key. Empty/None means no access (must specify scopes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scopes: Option<Vec<String>>,
}

/// Result of verifying an API key, including granted scopes.
#[derive(Debug, Clone)]
pub struct ApiKeyVerification {
    pub key_id: i64,
    /// Scopes granted to this key. Empty means no access (key must specify scopes).
    pub scopes: Vec<String>,
}

/// All valid API key scopes.
pub const VALID_SCOPES: &[&str] = &[
    "operator.admin",
    "operator.read",
    "operator.write",
    "operator.approvals",
    "operator.pairing",
];

/// An environment variable entry (for listing in the UI, never exposes the value).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvVarEntry {
    pub id: i64,
    pub key: String,
    pub created_at: String,
    pub updated_at: String,
    pub encrypted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SshAuthMode {
    System,
    Managed,
}

impl SshAuthMode {
    pub(super) fn as_db_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Managed => "managed",
        }
    }

    pub(super) fn parse_db(value: &str) -> crate::Result<Self> {
        match value {
            "system" => Ok(Self::System),
            "managed" => Ok(Self::Managed),
            _ => Err(crate::Error::Ssh(format!(
                "unknown ssh auth mode '{value}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshKeyEntry {
    pub id: i64,
    pub name: String,
    pub public_key: String,
    pub fingerprint: String,
    pub created_at: String,
    pub updated_at: String,
    pub encrypted: bool,
    pub target_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshTargetEntry {
    pub id: i64,
    pub label: String,
    pub target: String,
    pub port: Option<u16>,
    pub known_host: Option<String>,
    pub auth_mode: SshAuthMode,
    pub key_id: Option<i64>,
    pub key_name: Option<String>,
    pub is_default: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct SshResolvedTarget {
    pub id: i64,
    pub node_id: String,
    pub label: String,
    pub target: String,
    pub port: Option<u16>,
    pub known_host: Option<String>,
    pub auth_mode: SshAuthMode,
    pub key_id: Option<i64>,
    pub key_name: Option<String>,
}
