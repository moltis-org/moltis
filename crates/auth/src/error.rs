use thiserror::Error;

/// Errors produced by the `moltis-auth` crate.
#[derive(Debug, Error)]
pub enum Error {
    /// SQLite / sqlx database error.
    #[error(transparent)]
    Database(#[from] sqlx::Error),

    /// I/O error.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// Configuration file read/write error.
    #[error(transparent)]
    Config(#[from] moltis_config::Error),

    /// JSON serialization / deserialization error.
    #[error(transparent)]
    Json(#[from] serde_json::Error),

    /// Password hashing or vault encryption/decryption failure.
    #[error("{0}")]
    Crypto(String),

    /// Business-logic validation failure (e.g. "password already set").
    #[error("{0}")]
    Validation(String),

    /// WebAuthn protocol error.
    #[error("{0}")]
    WebAuthn(String),

    /// SSH key/target validation error.
    #[error("{0}")]
    Ssh(String),
}

/// Crate-level result alias.
pub type Result<T> = std::result::Result<T, Error>;
