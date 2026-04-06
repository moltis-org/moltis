use async_trait::async_trait;

use crate::{
    transport::{ExternalAgentSession, ExternalAgentTransport},
    types::{AgentTransportKind, ExternalAgentSpec},
};

const BINARY_NAME: &str = "pi";

/// Transport for Pi agent (JSON-RPC with state management).
pub struct PiAgentTransport;

impl PiAgentTransport {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for PiAgentTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExternalAgentTransport for PiAgentTransport {
    fn name(&self) -> &str {
        "pi-agent"
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self)))]
    async fn is_available(&self) -> bool {
        which::which(BINARY_NAME).is_ok()
    }

    fn supported_kinds(&self) -> &[AgentTransportKind] {
        &[AgentTransportKind::PiAgent]
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self, _spec)))]
    async fn start_session(
        &self,
        _spec: &ExternalAgentSpec,
    ) -> anyhow::Result<Box<dyn ExternalAgentSession>> {
        anyhow::bail!("Pi agent runtime not yet implemented")
    }
}
