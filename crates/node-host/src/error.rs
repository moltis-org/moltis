//! Crate-level error type for `moltis-node-host`.

use thiserror::Error;

/// Errors produced by the node-host crate.
#[derive(Debug, Error)]
pub enum Error {
    /// I/O failure (file read/write, process spawn, etc.).
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// JSON serialization / deserialization failure.
    #[error(transparent)]
    Json(#[from] serde_json::Error),

    /// Invalid URL.
    #[error(transparent)]
    Url(#[from] url::ParseError),

    /// WebSocket transport error.
    #[error(transparent)]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    /// Configuration problem (missing file, bad value, etc.).
    #[error("{0}")]
    Config(String),

    /// Protocol-level error (unexpected frame, handshake failure, etc.).
    #[error("{0}")]
    Protocol(String),

    /// OS-service management error (launchd / systemd).
    #[error("{0}")]
    Service(String),

    /// Command execution error (missing args, timeout, etc.).
    #[error("{0}")]
    Command(String),

    /// The current platform does not support the requested service operation.
    #[error("service operation not supported on this platform")]
    UnsupportedPlatform,
}

/// Convenience alias used throughout this crate.
pub type Result<T> = std::result::Result<T, Error>;
