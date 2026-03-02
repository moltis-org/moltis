use std::error::Error as StdError;

/// Crate-wide result type for channel operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Typed channel errors shared across channel traits.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Input payload or parameter is invalid.
    #[error("invalid channel input: {message}")]
    InvalidInput { message: String },

    /// A requested account ID is not registered.
    #[error("unknown channel account: {account_id}")]
    UnknownAccount { account_id: String },

    /// Operation is currently unavailable (not configured/ready).
    #[error("channel operation unavailable: {message}")]
    Unavailable { message: String },

    /// Wrapped source error from an external dependency.
    #[error("channel operation failed: {context}: {source}")]
    External {
        context: String,
        #[source]
        source: Box<dyn StdError + Send + Sync>,
    },

    /// JSON (de)serialization failed.
    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),

    /// Integer parsing failed.
    #[error(transparent)]
    ParseInt(#[from] std::num::ParseIntError),
}

impl Error {
    /// Whether this error is transient and the operation may succeed if retried.
    ///
    /// - `External` errors are assumed retryable (network/transient failures).
    /// - `Unavailable` errors are retryable (service may become ready).
    /// - `UnknownAccount`, `InvalidInput`, parse, and serde errors are fatal.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::External { .. } | Self::Unavailable { .. })
    }

    #[must_use]
    pub fn invalid_input(message: impl std::fmt::Display) -> Self {
        Self::InvalidInput {
            message: message.to_string(),
        }
    }

    #[must_use]
    pub fn unavailable(message: impl std::fmt::Display) -> Self {
        Self::Unavailable {
            message: message.to_string(),
        }
    }

    #[must_use]
    pub fn unknown_account(account_id: impl std::fmt::Display) -> Self {
        Self::UnknownAccount {
            account_id: account_id.to_string(),
        }
    }

    #[must_use]
    pub fn external(
        context: impl Into<String>,
        source: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self::External {
            context: context.into(),
            source: Box::new(source),
        }
    }
}
