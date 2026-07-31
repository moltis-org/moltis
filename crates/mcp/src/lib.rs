//! MCP (Model Context Protocol) client support for moltis.
//!
//! This crate provides:
//! - JSON-RPC 2.0 over stdio transport (`transport`)
//! - MCP client for protocol handshake and tool interactions (`client`)
//! - Tool bridge adapting MCP tools to the agent tool interface (`tool_bridge`)
//! - Server lifecycle management (`manager`)
//! - Persisted server registry (`registry`)
//!
//! Remote HTTP/SSE servers keep secret-bearing values (URLs, header values)
//! in secret-aware types and only expose sanitized display projections.

pub mod auth;
pub mod client;
pub mod config_parsing;
pub mod error;
pub mod legacy_sse_transport;
pub mod managed_repositories;
pub mod manager;
mod manager_managed;
pub mod registry;
pub mod remote;
pub mod repository_discovery;
pub mod sse_transport;
pub mod tool_bridge;
pub mod traits;
pub mod transport;
pub mod types;

pub use {
    auth::{McpAuthProvider, McpAuthState, McpOAuthOverride, McpOAuthProvider, SharedAuthProvider},
    client::{McpClient, McpClientState},
    config_parsing::{merge_env_overrides, parse_server_config},
    error::{Context, Error, Result},
    managed_repositories::{
        ManagedApproval, ManagedApprovalRequest, ManagedDiscoveryMode, ManagedInstallSelection,
        ManagedOrigin, ManagedReconciliationResult, ManagedRepository, ManagedRepositoryAccess,
        ManagedRepositoryAlias, ManagedRepositoryId, ManagedRepositoryLock,
        ManagedRepositoryPreview, ManagedRepositorySource, ManagedRevision, ManagedServerCandidate,
        ManagedServerIdentity, ManagedWarning, ManagedWarningKind, managed_approval_block_reason,
        managed_runtime_env_overrides, preview_managed_repository,
    },
    manager::{ManagedServerStatus, McpManager, ServerStatus},
    registry::{McpOAuthConfig, McpRegistry, McpServerConfig, TransportType},
    repository_discovery::{
        DiscoveredMcpServer, DiscoveryWarning, DiscoveryWarningKind, PluginIdentity,
        RepositoryDiscovery, RepositoryManifestKind, discover_repository,
    },
    tool_bridge::{McpAgentTool, McpToolBridge},
    traits::{McpClientTrait, McpTransport},
    transport::StdioLaunchOptions,
    types::{McpManagerError, McpTransportError},
};
