use moltis_channels::Error as ChannelError;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("nostr SDK error: {0}")]
    Sdk(#[from] nostr_sdk::client::Error),

    #[error("nostr key error: {0}")]
    Key(#[from] nostr_sdk::prelude::key::Error),

    #[error("invalid config: {0}")]
    Config(String),

    #[error("relay error: {0}")]
    Relay(String),

    #[error("encryption error: {0}")]
    Encryption(String),

    #[error("event not found: {0}")]
    NotFound(String),
}

impl Error {
    /// Whether this error is transient and the operation may succeed on retry.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Sdk(_) | Self::Relay(_))
    }
}

impl From<Error> for ChannelError {
    fn from(e: Error) -> Self {
        match e {
            Error::Config(msg) => ChannelError::invalid_input(msg),
            Error::NotFound(id) => ChannelError::unavailable(format!("event not found: {id}")),
            other => ChannelError::external("nostr", other),
        }
    }
}
