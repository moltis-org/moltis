use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("QMD is not available")]
    NotAvailable,
    #[error("QMD command timed out after {0}ms")]
    Timeout(u64),
    #[error("QMD {command} failed: {stderr}")]
    CommandFailed { command: String, stderr: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Utf8(#[from] std::string::FromUtf8Error),
}

pub type Result<T> = std::result::Result<T, Error>;
