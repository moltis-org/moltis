//! Telephony error types.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("provider error: {0}")]
    Provider(String),

    #[error("call not found: {0}")]
    CallNotFound(String),

    #[error("invalid state transition: {from:?} -> {to:?}")]
    InvalidTransition {
        from: crate::types::CallState,
        to: crate::types::CallState,
    },

    #[error("webhook verification failed: {0}")]
    WebhookVerification(String),

    #[error("configuration error: {0}")]
    Config(String),

    #[error("audio conversion error: {0}")]
    Audio(String),

    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
