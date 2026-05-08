//! External CLI Agent Bridge for Moltis.
//!
//! Connects Moltis chat sessions to external CLI coding agents (Claude Code,
//! opencode, Codex CLI, Pi agent, ACP agents). The CLI agent owns its own
//! session state; Moltis acts as orchestrator and source of truth for
//! conversation history. Delta sync on reattach, compaction reconciliation
//! handled gracefully.

pub mod bridge;
pub mod error;
pub mod registry;
pub mod runtimes;
pub mod transport;
pub mod types;

pub use {
    error::ExternalAgentError,
    registry::ExternalAgentRegistry,
    transport::{ExternalAgentSession, ExternalAgentTransport},
    types::{
        AcpPermissionHandler, AcpPermissionOption, AcpPermissionOptionKind, AcpPermissionRequest,
        AgentTransportKind, BridgeState, ContextSnapshot, ExternalAgentEvent, ExternalAgentInfo,
        ExternalAgentSpec, ExternalAgentStatus, TokenUsage,
    },
};
