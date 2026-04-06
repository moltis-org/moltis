use async_trait::async_trait;

use crate::{
    transport::{ExternalAgentSession, ExternalAgentTransport},
    types::{AgentTransportKind, ExternalAgentSpec},
};

const BINARY_NAME: &str = "codex";

/// Transport for Codex CLI agent (JSON-RPC over stdio).
pub struct CodexTransport;

impl CodexTransport {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for CodexTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExternalAgentTransport for CodexTransport {
    fn name(&self) -> &str {
        "codex"
    }

    async fn is_available(&self) -> bool {
        which::which(BINARY_NAME).is_ok()
    }

    fn supported_kinds(&self) -> &[AgentTransportKind] {
        &[AgentTransportKind::Codex]
    }

    async fn start_session(
        &self,
        _spec: &ExternalAgentSpec,
    ) -> anyhow::Result<Box<dyn ExternalAgentSession>> {
        anyhow::bail!("Codex runtime not yet implemented")
    }
}
