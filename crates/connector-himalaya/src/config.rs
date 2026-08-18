use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{HimalayaConnectorError, Result};

const SCHEMA_VERSION: u32 = 1;
const MAX_ACCOUNT_NAME_BYTES: usize = 256;
const MAX_MAILBOX_ID_BYTES: usize = 1024;
const MAX_QUERY_BYTES: usize = 4 * 1024;

const fn schema_version() -> u32 {
    SCHEMA_VERSION
}

const fn default_max_messages() -> u32 {
    500
}

const fn default_include_bodies() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HimalayaBackend {
    Imap,
    Jmap,
    Gmail,
    Msgraph,
    Maildir,
    M2dir,
}

impl HimalayaBackend {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Imap => "imap",
            Self::Jmap => "jmap",
            Self::Gmail => "gmail",
            Self::Msgraph => "msgraph",
            Self::Maildir => "maildir",
            Self::M2dir => "m2dir",
        }
    }

    #[must_use]
    pub const fn supports_shared_search(self) -> bool {
        !matches!(self, Self::Gmail | Self::Msgraph)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HimalayaAccountConfig {
    #[serde(default = "schema_version")]
    pub schema_version: u32,
    pub account_name: String,
    pub backend: HimalayaBackend,
}

impl HimalayaAccountConfig {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(HimalayaConnectorError::AccountConfig(
                "schemaVersion must be 1",
            ));
        }
        if self.account_name.trim().is_empty() {
            return Err(HimalayaConnectorError::AccountConfig(
                "accountName must not be empty",
            ));
        }
        if self.account_name.len() > MAX_ACCOUNT_NAME_BYTES {
            return Err(HimalayaConnectorError::AccountConfig(
                "accountName exceeds 256 bytes",
            ));
        }
        if self.account_name.chars().any(char::is_control) {
            return Err(HimalayaConnectorError::AccountConfig(
                "accountName must not contain control characters",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HimalayaDatasetConfig {
    #[serde(default = "schema_version")]
    pub schema_version: u32,
    pub mailbox_ids: Vec<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default = "default_max_messages")]
    pub max_messages: u32,
    #[serde(default = "default_include_bodies")]
    pub include_bodies: bool,
}

impl HimalayaDatasetConfig {
    pub fn validate(&self, backend: HimalayaBackend) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(HimalayaConnectorError::DatasetConfig(
                "schemaVersion must be 1",
            ));
        }
        if !(1..=32).contains(&self.mailbox_ids.len()) {
            return Err(HimalayaConnectorError::DatasetConfig(
                "mailboxIds must contain between 1 and 32 entries",
            ));
        }
        if self.mailbox_ids.iter().any(|id| {
            id.trim().is_empty()
                || id.len() > MAX_MAILBOX_ID_BYTES
                || id.chars().any(char::is_control)
        }) {
            return Err(HimalayaConnectorError::DatasetConfig(
                "mailbox IDs must be nonempty, bounded, and contain no control characters",
            ));
        }
        if self.mailbox_ids.iter().collect::<HashSet<_>>().len() != self.mailbox_ids.len() {
            return Err(HimalayaConnectorError::DatasetConfig(
                "mailboxIds must be exactly deduplicated",
            ));
        }
        if self.query.as_ref().is_some_and(|query| {
            query.len() > MAX_QUERY_BYTES || query.chars().any(|character| character == '\0')
        }) {
            return Err(HimalayaConnectorError::DatasetConfig(
                "query must not exceed 4096 bytes or contain NUL",
            ));
        }
        if self.query.as_ref().is_some_and(|query| !query.is_empty())
            && !backend.supports_shared_search()
        {
            return Err(HimalayaConnectorError::DatasetConfig(
                "search is not supported by the Gmail or Msgraph Himalaya v2 backends",
            ));
        }
        if !(1..=1000).contains(&self.max_messages) {
            return Err(HimalayaConnectorError::DatasetConfig(
                "maxMessages must be between 1 and 1000",
            ));
        }
        Ok(())
    }
}

impl Default for HimalayaDatasetConfig {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            mailbox_ids: Vec::new(),
            query: None,
            max_messages: default_max_messages(),
            include_bodies: default_include_bodies(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_configs_and_rejects_persisted_extra_fields() -> Result<()> {
        let account = HimalayaAccountConfig {
            schema_version: 1,
            account_name: "work".to_owned(),
            backend: HimalayaBackend::Imap,
        };
        account.validate()?;
        assert!(
            serde_json::from_str::<HimalayaAccountConfig>(
                r#"{"accountName":"work","backend":"imap","executable":"evil"}"#
            )
            .is_err()
        );

        let mut dataset = HimalayaDatasetConfig {
            mailbox_ids: vec!["INBOX".to_owned()],
            ..HimalayaDatasetConfig::default()
        };
        dataset.validate(HimalayaBackend::Imap)?;
        dataset.query = Some("x".to_owned());
        assert!(dataset.validate(HimalayaBackend::Gmail).is_err());
        Ok(())
    }
}
