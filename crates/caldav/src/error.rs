//! Crate-level error types for `moltis-caldav`.

use thiserror::Error;

/// Errors returned by CalDAV operations.
#[derive(Debug, Error)]
pub enum Error {
    /// I/O error (file, network socket, etc.).
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// The supplied URL could not be parsed or is otherwise invalid.
    #[error("{0}")]
    InvalidUrl(String),

    /// A CalDAV/WebDAV protocol-level error (discovery failure, unexpected
    /// server response, etc.).
    #[error("{0}")]
    Protocol(String),

    /// Failed to parse iCalendar (RFC 5545) data.
    #[error("{0}")]
    IcalParse(String),

    /// A required parameter was missing or had an invalid value.
    #[error("{0}")]
    Validation(String),

    /// The requested resource was not found on the server.
    #[error("{0}")]
    NotFound(String),
}

/// Convenience alias used throughout this crate.
pub type Result<T> = std::result::Result<T, Error>;
