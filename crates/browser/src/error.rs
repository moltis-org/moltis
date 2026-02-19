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

/// Substrings that indicate the CDP WebSocket connection is dead.
const STALE_CONNECTION_PATTERNS: &[&str] = &[
    "receiver is gone",
    "oneshot canceled",
    "Request timed out",
    "Connection closed",
    "AlreadyClosed",
    "closed connection",
];

impl BrowserError {
    /// Returns `true` when this error indicates the CDP connection to the
    /// browser is dead and the session should be recycled.
    pub fn is_connection_error(&self) -> bool {
        match self {
            // Explicit dead-connection variants
            Self::BrowserClosed | Self::ConnectionClosed(_) => true,

            // Message-bearing variants — check for known stale-connection patterns
            Self::Cdp(msg)
            | Self::ScreenshotFailed(msg)
            | Self::JsEvalFailed(msg)
            | Self::NavigationFailed(msg)
            | Self::Timeout(msg) => STALE_CONNECTION_PATTERNS.iter().any(|p| msg.contains(p)),

            _ => false,
        }
    }
}

impl From<chromiumoxide::error::CdpError> for BrowserError {
    fn from(err: chromiumoxide::error::CdpError) -> Self {
        BrowserError::Cdp(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_variants_are_connection_errors() {
        assert!(BrowserError::BrowserClosed.is_connection_error());
        assert!(BrowserError::ConnectionClosed("whatever".into()).is_connection_error());
    }

    #[test]
    fn stale_connection_messages_detected() {
        let patterns = [
            "send failed because receiver is gone",
            "oneshot canceled",
            "Request timed out.",
            "Connection closed by remote",
            "AlreadyClosed",
            "WebSocket closed connection",
        ];

        // Each pattern should be detected in every message-bearing variant
        for msg in patterns {
            let m = msg.to_string();
            assert!(
                BrowserError::Cdp(m.clone()).is_connection_error(),
                "Cdp({msg})"
            );
            assert!(
                BrowserError::ScreenshotFailed(m.clone()).is_connection_error(),
                "ScreenshotFailed({msg})"
            );
            assert!(
                BrowserError::JsEvalFailed(m.clone()).is_connection_error(),
                "JsEvalFailed({msg})"
            );
            assert!(
                BrowserError::NavigationFailed(m.clone()).is_connection_error(),
                "NavigationFailed({msg})"
            );
            assert!(
                BrowserError::Timeout(m.clone()).is_connection_error(),
                "Timeout({msg})"
            );
        }
    }

    #[test]
    fn normal_errors_are_not_connection_errors() {
        assert!(!BrowserError::BrowserNotAvailable.is_connection_error());
        assert!(!BrowserError::LaunchFailed("out of memory".into()).is_connection_error());
        assert!(!BrowserError::ElementNotFound(42).is_connection_error());
        assert!(!BrowserError::InvalidSelector("div>".into()).is_connection_error());
        assert!(!BrowserError::PoolExhausted.is_connection_error());
        assert!(!BrowserError::InvalidAction("bad action".into()).is_connection_error());
        // Message-bearing variant with an unrelated message
        assert!(!BrowserError::Cdp("some other CDP error".into()).is_connection_error());
        assert!(
            !BrowserError::Timeout("element not found after 5000ms".into()).is_connection_error()
        );
    }
}
