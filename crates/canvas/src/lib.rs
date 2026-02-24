//! Canvas / A2UI host: HTTP + WebSocket server for agent-controlled HTML UI on mobile nodes.
//!
//! Flow: agent invokes canvas.push → canvas host serves HTML → node renders in
//! WKWebView/WebView → user action → event back to agent.

pub mod server;

pub mod error;

pub use error::{Error, Result};
