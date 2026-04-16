//! Error types for the code-index crate.

use std::path::PathBuf;

/// Errors that can occur during codebase indexing operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("git repository not found at {path}: {message}")]
    GitRepoNotFound { path: PathBuf, message: String },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("indexing failed for project {project_id}: {message}")]
    IndexFailed { project_id: String, message: String },

    #[error("configuration error: {0}")]
    Config(String),

    #[error("backend unavailable: {0}")]
    BackendUnavailable(String),

    #[error("snapshot store error: {0}")]
    Store(String),

    #[error("index store error: {0}")]
    IndexStore(String),

    #[error("search failed for project {project_id}: {message}")]
    SearchFailed { project_id: String, message: String },
}

pub type Result<T> = std::result::Result<T, Error>;
