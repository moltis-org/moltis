//! Error types for `moltis-ctl`.

#[derive(Debug, thiserror::Error)]
pub enum CtlError {
    #[error("connection failed: {0}")]
    Connect(String),

    #[error("handshake failed: {0}")]
    Handshake(String),

    #[error("RPC error [{code}]: {message}")]
    Rpc { code: String, message: String },

    #[error("WebSocket error: {0}")]
    Ws(String),

    #[error("timeout waiting for response")]
    Timeout,

    #[error("{0}")]
    Other(String),
}

impl From<moltis_client::Error> for CtlError {
    fn from(error: moltis_client::Error) -> Self {
        use moltis_client::Error;
        match error {
            Error::InvalidUrl(_) | Error::Connect(_) | Error::Tls(_) | Error::Http { .. } => {
                Self::Connect(error.to_string())
            },
            Error::Handshake(_) | Error::Unauthorized(_) => Self::Handshake(error.to_string()),
            Error::Rpc { code, message } => Self::Rpc { code, message },
            Error::Disconnected => Self::Ws(error.to_string()),
            Error::Timeout => Self::Timeout,
            other => Self::Other(other.to_string()),
        }
    }
}

/// Process exit codes.
pub mod exit {
    pub const SUCCESS: i32 = 0;
    pub const CONNECTION_ERROR: i32 = 1;
    pub const RPC_ERROR: i32 = 2;
    pub const BAD_ARGS: i32 = 3;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_client_errors() {
        assert!(matches!(
            CtlError::from(moltis_client::Error::Timeout),
            CtlError::Timeout
        ));
        assert!(matches!(
            CtlError::from(moltis_client::Error::Rpc {
                code: "NOT_FOUND".into(),
                message: "x".into()
            }),
            CtlError::Rpc { ref code, .. } if code == "NOT_FOUND"
        ));
        assert!(matches!(
            CtlError::from(moltis_client::Error::Unauthorized("no".into())),
            CtlError::Handshake(_)
        ));
        assert!(matches!(
            CtlError::from(moltis_client::Error::InvalidUrl("bad".into())),
            CtlError::Connect(_)
        ));
    }
}
