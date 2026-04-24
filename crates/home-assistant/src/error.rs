//! Error types for Moltis Home Assistant integration.

/// Errors produced by the Home Assistant client.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("config: {0}")]
    Config(String),

    #[error("connection: {0}")]
    Connection(String),

    #[error("authentication failed: {0}")]
    Auth(String),

    #[error("HTTP client: {0}")]
    Client(String),

    #[error("websocket: {0}")]
    WebSocket(String),

    #[error("entity not found: {0}")]
    NotFound(String),

    #[error("service call failed: {0}")]
    ServiceCall(String),

    #[error("event error: {0}")]
    Event(String),

    #[error("camera proxy: {0}")]
    Camera(String),

    #[error("serialization: {0}")]
    Serialization(String),

    #[error("request: {0}")]
    Request(#[from] reqwest::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

impl From<String> for Error {
    fn from(value: String) -> Self {
        Self::Config(value)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
