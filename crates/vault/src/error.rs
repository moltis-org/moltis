//! Vault error types.

/// Errors produced by vault operations.
#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    /// The vault is sealed (locked) — must be unlocked first.
    #[error("vault is sealed")]
    Sealed,

    /// The vault is already initialized (DEK already exists).
    #[error("vault is already initialized")]
    AlreadyInitialized,

    /// The vault has not been initialized yet (no DEK).
    #[error("vault is not initialized")]
    NotInitialized,

    /// Password or recovery key is incorrect.
    #[error("incorrect password or recovery key")]
    BadCredential,

    /// Encryption or decryption failed (tampered data, wrong key).
    #[error("cipher error: {0}")]
    CipherError(String),

    /// Base64 decoding failed.
    #[error("base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),

    /// Database error.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    /// JSON serialization / deserialization error.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// Generic error wrapper.
    #[error("{0}")]
    Other(#[from] anyhow::Error),
}
