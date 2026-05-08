//! Crate-level error type for `moltis-memory`.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Database(#[from] sqlx::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Http(#[from] reqwest::Error),

    #[error(transparent)]
    Migration(#[from] sqlx::migrate::MigrateError),

    #[error("{0}")]
    Embedding(String),

    #[error("{0}")]
    Validation(String),

    #[error("{0}")]
    Reranking(String),
}

pub type Result<T> = std::result::Result<T, Error>;
