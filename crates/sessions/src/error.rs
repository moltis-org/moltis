use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),

    #[error(transparent)]
    Join(#[from] tokio::task::JoinError),

    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),

    #[error("file lock failed: {message}")]
    Lock { message: String },

    #[error("{message}")]
    Message { message: String },
}

impl Error {
    #[must_use]
    pub fn lock_failed(message: impl Into<String>) -> Self {
        Self::Lock {
            message: message.into(),
        }
    }

    #[must_use]
    pub fn message(message: impl Into<String>) -> Self {
        Self::Message {
            message: message.into(),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
