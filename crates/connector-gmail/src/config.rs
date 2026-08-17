use {
    secrecy::Secret,
    serde::{Deserialize, Deserializer, Serialize},
    std::fmt,
};

use crate::{GmailConnectorError, Result};

const SCHEMA_VERSION: u32 = 1;
const MAX_QUERY_BYTES: usize = 4 * 1024;
const GMAIL_READONLY_SCOPE: &str = "https://www.googleapis.com/auth/gmail.readonly";
const GMAIL_MODIFY_SCOPE: &str = "https://www.googleapis.com/auth/gmail.modify";
const GMAIL_FULL_SCOPE: &str = "https://mail.google.com/";

const fn schema_version() -> u32 {
    SCHEMA_VERSION
}

const fn default_max_messages() -> u32 {
    500
}

const fn default_include_body() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GmailAccountConfig {
    #[serde(default = "schema_version")]
    pub schema_version: u32,
}

impl Default for GmailAccountConfig {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
        }
    }
}

impl GmailAccountConfig {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(GmailConnectorError::AccountConfig(
                "schemaVersion must be 1",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GmailDatasetConfig {
    #[serde(default = "schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub query: String,
    #[serde(default = "default_max_messages")]
    pub max_messages: u32,
    #[serde(default = "default_include_body")]
    pub include_body: bool,
}

impl Default for GmailDatasetConfig {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            query: String::new(),
            max_messages: default_max_messages(),
            include_body: default_include_body(),
        }
    }
}

impl GmailDatasetConfig {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(GmailConnectorError::DatasetConfig(
                "schemaVersion must be 1",
            ));
        }
        if self.query.len() > MAX_QUERY_BYTES {
            return Err(GmailConnectorError::DatasetConfig(
                "query must not exceed 4096 bytes",
            ));
        }
        if !(1..=1_000).contains(&self.max_messages) {
            return Err(GmailConnectorError::DatasetConfig(
                "maxMessages must be between 1 and 1000",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Deserialize)]
pub struct AuthorizedUserToken {
    pub client_id: String,
    #[serde(deserialize_with = "deserialize_secret")]
    pub client_secret: Secret<String>,
    #[serde(deserialize_with = "deserialize_secret")]
    pub refresh_token: Secret<String>,
    #[serde(default, deserialize_with = "deserialize_scopes")]
    pub scopes: Vec<String>,
    #[serde(default, rename = "type")]
    credential_type: Option<String>,
}

impl fmt::Debug for AuthorizedUserToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizedUserToken")
            .field("client_id", &"[REDACTED]")
            .field("client_secret", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .field("scopes", &self.scopes)
            .field("credential_type", &self.credential_type)
            .finish()
    }
}

impl AuthorizedUserToken {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let token: Self =
            serde_json::from_slice(bytes).map_err(GmailConnectorError::CredentialJson)?;
        token.validate()?;
        Ok(token)
    }

    pub fn validate(&self) -> Result<()> {
        if self
            .credential_type
            .as_deref()
            .is_some_and(|value| value != "authorized_user")
        {
            return Err(GmailConnectorError::Credential(
                "type must be authorized_user",
            ));
        }
        if self.client_id.trim().is_empty() {
            return Err(GmailConnectorError::Credential(
                "client_id must not be empty",
            ));
        }
        use secrecy::ExposeSecret;
        if self.client_secret.expose_secret().is_empty() {
            return Err(GmailConnectorError::Credential(
                "client_secret must not be empty",
            ));
        }
        if self.refresh_token.expose_secret().is_empty() {
            return Err(GmailConnectorError::Credential(
                "refresh_token must not be empty",
            ));
        }
        if !self.scopes.iter().any(|scope| {
            matches!(
                scope.as_str(),
                GMAIL_READONLY_SCOPE | GMAIL_MODIFY_SCOPE | GMAIL_FULL_SCOPE
            )
        }) {
            return Err(GmailConnectorError::Credential(
                "a Gmail readonly-capable scope is required",
            ));
        }
        Ok(())
    }
}

fn deserialize_secret<'de, D>(deserializer: D) -> std::result::Result<Secret<String>, D::Error>
where
    D: Deserializer<'de>,
{
    String::deserialize(deserializer).map(Secret::new)
}

fn deserialize_scopes<'de, D>(deserializer: D) -> std::result::Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum ScopeValue {
        String(String),
        Array(Vec<String>),
    }

    let value = Option::<ScopeValue>::deserialize(deserializer)?;
    Ok(match value {
        Some(ScopeValue::String(value)) => {
            value.split_whitespace().map(ToOwned::to_owned).collect()
        },
        Some(ScopeValue::Array(values)) => values,
        None => Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn dataset_defaults_and_validation() {
        let config: GmailDatasetConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config, GmailDatasetConfig::default());
        assert!(config.validate().is_ok());
        assert!(
            GmailDatasetConfig {
                max_messages: 0,
                ..config.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            GmailDatasetConfig {
                max_messages: 1_001,
                ..config.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            GmailDatasetConfig {
                query: "x".repeat(MAX_QUERY_BYTES + 1),
                ..config
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn token_accepts_string_and_array_scopes_and_redacts_debug() {
        let string_token = AuthorizedUserToken::parse(
            br#"{"type":"authorized_user","client_id":"client-value","client_secret":"secret-value","refresh_token":"refresh-value","scopes":"openid https://www.googleapis.com/auth/gmail.readonly"}"#,
        )
        .unwrap();
        assert_eq!(string_token.scopes.len(), 2);
        let debug = format!("{string_token:?}");
        assert!(!debug.contains("client-value"));
        assert!(!debug.contains("secret-value"));
        assert!(!debug.contains("refresh-value"));

        let array_token = AuthorizedUserToken::parse(
            br#"{"client_id":"c","client_secret":"s","refresh_token":"r","scopes":["https://mail.google.com/"]}"#,
        )
        .unwrap();
        assert_eq!(array_token.scopes, vec![GMAIL_FULL_SCOPE]);
    }

    #[test]
    fn token_rejects_insufficient_scope_and_wrong_type() {
        assert!(AuthorizedUserToken::parse(
            br#"{"client_id":"c","client_secret":"s","refresh_token":"r","scopes":"openid email"}"#,
        )
        .is_err());
        assert!(AuthorizedUserToken::parse(
            br#"{"type":"service_account","client_id":"c","client_secret":"s","refresh_token":"r","scopes":"https://www.googleapis.com/auth/gmail.readonly"}"#,
        )
        .is_err());
    }
}
