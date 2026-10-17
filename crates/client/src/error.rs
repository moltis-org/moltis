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

/// Coarse cause of a failed connection, for choosing what to show the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// Nothing answered (daemon not running, wrong address, network down).
    Unreachable,
    /// The TLS certificate was not trusted or changed.
    Tls,
    Timeout,
    /// The daemon rejected the credentials.
    Auth,
    /// An established connection dropped.
    Closed,
    Other,
}

impl Error {
    #[must_use]
    pub fn failure_kind(&self) -> FailureKind {
        if self.is_auth() {
            return FailureKind::Auth;
        }
        match self {
            Self::Connect(_) | Self::Io(_) => FailureKind::Unreachable,
            Self::Tls(_) => FailureKind::Tls,
            Self::Timeout => FailureKind::Timeout,
            Self::Disconnected => FailureKind::Closed,
            _ => FailureKind::Other,
        }
    }

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

    #[test]
    fn failure_kinds() {
        assert_eq!(
            Error::Connect("refused".into()).failure_kind(),
            FailureKind::Unreachable
        );
        assert_eq!(Error::Tls("bad".into()).failure_kind(), FailureKind::Tls);
        assert_eq!(Error::Timeout.failure_kind(), FailureKind::Timeout);
        assert_eq!(
            Error::Unauthorized("no".into()).failure_kind(),
            FailureKind::Auth
        );
        assert_eq!(Error::Disconnected.failure_kind(), FailureKind::Closed);
        assert_eq!(
            Error::InvalidUrl("x".into()).failure_kind(),
            FailureKind::Other
        );
    }
}
