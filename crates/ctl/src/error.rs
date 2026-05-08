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

/// Process exit codes.
pub mod exit {
    pub const SUCCESS: i32 = 0;
    pub const CONNECTION_ERROR: i32 = 1;
    pub const RPC_ERROR: i32 = 2;
    pub const BAD_ARGS: i32 = 3;
}
