//! Browser error types.

use thiserror::Error;

/// Errors that can occur during browser operations.
#[derive(Debug, Error)]
pub enum BrowserError {
    #[error("browser not available: Chrome/Chromium not found")]
    BrowserNotAvailable,

    #[error("browser launch failed: {0}")]
    LaunchFailed(String),

    #[error("navigation failed: {0}")]
    NavigationFailed(String),

    #[error("element not found: ref {0}")]
    ElementNotFound(u32),

    #[error("invalid selector: {0}")]
    InvalidSelector(String),

    #[error("JavaScript evaluation failed: {0}")]
    JsEvalFailed(String),

    #[error("screenshot failed: {0}")]
    ScreenshotFailed(String),

    #[error("timeout: {0}")]
    Timeout(String),

    #[error("pool exhausted: no browser instances available")]
    PoolExhausted,

    #[error("browser closed unexpectedly")]
    BrowserClosed,

    #[error("connection closed: {0}")]
    ConnectionClosed(String),

    #[error("CDP error: {0}")]
    Cdp(String),

    #[error("invalid action: {0}")]
    InvalidAction(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<chromiumoxide::error::CdpError> for BrowserError {
    fn from(err: chromiumoxide::error::CdpError) -> Self {
        BrowserError::Cdp(err.to_string())
    }
}
