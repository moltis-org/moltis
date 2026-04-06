use async_trait::async_trait;

use crate::{
    transport::{ExternalAgentSession, ExternalAgentTransport},
    types::{AgentTransportKind, ExternalAgentSpec},
};

const BINARY_NAME: &str = "claude";

/// Transport for Claude Code CLI agent.
///
/// Supports two modes:
/// - `-p` (print/one-shot): prompt via stdin, structured JSON on stdout
/// - Interactive: tmux session with terminal output parsing
pub struct ClaudeCodeTransport;

impl ClaudeCodeTransport {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for ClaudeCodeTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExternalAgentTransport for ClaudeCodeTransport {
    fn name(&self) -> &str {
        "claude-code"
    }

    async fn is_available(&self) -> bool {
        which::which(BINARY_NAME).is_ok()
    }

    fn supported_kinds(&self) -> &[AgentTransportKind] {
        &[AgentTransportKind::ClaudeCode]
    }

    async fn start_session(
        &self,
        _spec: &ExternalAgentSpec,
    ) -> anyhow::Result<Box<dyn ExternalAgentSession>> {
        // Phase 4 implementation — stub for now
        anyhow::bail!("Claude Code runtime not yet implemented")
    }
}
