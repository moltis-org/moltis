use thiserror::Error;

pub type Result<T> = std::result::Result<T, GmailConnectorError>;

#[derive(Debug, Error)]
pub enum GmailConnectorError {
    #[error("invalid Gmail account configuration: {0}")]
    AccountConfig(&'static str),
    #[error("invalid Gmail dataset configuration: {0}")]
    DatasetConfig(&'static str),
    #[error("invalid Google authorized-user credential: {0}")]
    Credential(&'static str),
    #[error("failed to read Google authorized-user credential")]
    CredentialIo(#[source] std::io::Error),
    #[error("failed to parse Google authorized-user credential")]
    CredentialJson(#[source] serde_json::Error),
    #[error("Google OAuth token refresh failed")]
    OAuth(#[source] moltis_oauth::Error),
    #[error("Gmail HTTP request failed")]
    Http(#[source] reqwest::Error),
    #[error("Gmail request timed out")]
    Timeout,
    #[error("Gmail API returned HTTP status {0}")]
    ApiStatus(u16),
    #[error("invalid Gmail API response: {0}")]
    ServerResponse(String),
    #[error("invalid Gmail message body: {0}")]
    MessageBody(String),
    #[error("failed to encode Gmail connector item")]
    Serialization(#[source] serde_json::Error),
}

impl From<serde_json::Error> for GmailConnectorError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}
