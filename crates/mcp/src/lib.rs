//! MCP (Model Context Protocol) client support for moltis.
//!
//! This crate provides:
//! - JSON-RPC 2.0 over stdio transport (`transport`)
//! - MCP client for protocol handshake and tool interactions (`client`)
//! - Tool bridge adapting MCP tools to the agent tool interface (`tool_bridge`)
//! - Server lifecycle management (`manager`)
//! - Persisted server registry (`registry`)

pub mod auth;
pub mod client;
pub mod error;
pub mod manager;
pub mod registry;
pub mod tool_bridge;
pub mod traits;
pub mod transport;
pub mod types;

pub mod sse_transport;

pub use {
    auth::{McpAuthProvider, McpAuthState, McpOAuthOverride, McpOAuthProvider, SharedAuthProvider},
    client::{McpClient, McpClientState},
    error::{Context, Error, Result},
    manager::McpManager,
    registry::{McpOAuthConfig, McpRegistry, McpServerConfig, TransportType},
    tool_bridge::{McpAgentTool, McpToolBridge},
    traits::{McpClientTrait, McpTransport},
    types::{McpManagerError, McpTransportError},
};
