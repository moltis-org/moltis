//! Error type of `moltis-client`.

use moltis_protocol::{ErrorShape, error_codes};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid server URL: {0}")]
    InvalidUrl(String),

    #[error("connection failed: {0}")]
    Connect(String),

    #[error("TLS error: {0}")]
    Tls(String),

    #[error("handshake failed: {0}")]
    Handshake(String),

    /// The daemon rejected the credentials (or none were given).
    #[error("not authorized: {0}")]
    Unauthorized(String),

    #[error("RPC error [{code}]: {message}")]
    Rpc { code: String, message: String },

    #[error("not connected to the daemon")]
    Disconnected,

    #[error("timed out waiting for the daemon")]
    Timeout,

    #[error("HTTP {status}: {body}")]
    Http { status: u16, body: String },

    #[error("invalid payload: {0}")]
    Json(#[from] serde_json::Error),

    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl Error {
    /// Whether retrying the same connection with the same credentials is
    /// pointless until the user changes something.
    #[must_use]
    pub fn is_auth(&self) -> bool {
        match self {
            Self::Unauthorized(_) => true,
            Self::Rpc { code, .. } => {
                code == error_codes::UNAUTHORIZED || code == error_codes::FORBIDDEN
            },
            Self::Http { status, .. } => *status == 401 || *status == 403,
            _ => false,
        }
    }
}

impl From<ErrorShape> for Error {
    fn from(shape: ErrorShape) -> Self {
        Self::Rpc {
            code: shape.code,
            message: shape.message,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_errors_are_detected() {
        assert!(Error::Unauthorized("x".into()).is_auth());
        assert!(Error::from(ErrorShape::new(error_codes::FORBIDDEN, "no")).is_auth());
        assert!(
            Error::Http {
                status: 401,
                body: String::new()
            }
            .is_auth()
        );
        assert!(!Error::Timeout.is_auth());
        assert!(!Error::from(ErrorShape::new(error_codes::INTERNAL, "boom")).is_auth());
    }
}
