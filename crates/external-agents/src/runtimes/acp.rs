use async_trait::async_trait;

use crate::{
    transport::{ExternalAgentSession, ExternalAgentTransport},
    types::{AgentTransportKind, ExternalAgentSpec},
};

/// Transport for ACP (Agent Communication Protocol) agents over JSON-RPC stdio.
pub struct AcpTransport {
    binary: String,
}

impl AcpTransport {
    #[must_use]
    pub fn new(binary: String) -> Self {
        Self { binary }
    }
}

#[async_trait]
impl ExternalAgentTransport for AcpTransport {
    fn name(&self) -> &str {
        "acp"
    }

    async fn is_available(&self) -> bool {
        which::which(&self.binary).is_ok()
    }

    fn supported_kinds(&self) -> &[AgentTransportKind] {
        &[AgentTransportKind::Acp]
    }

    async fn start_session(
        &self,
        _spec: &ExternalAgentSpec,
    ) -> anyhow::Result<Box<dyn ExternalAgentSession>> {
        anyhow::bail!("ACP runtime not yet implemented")
    }
}
