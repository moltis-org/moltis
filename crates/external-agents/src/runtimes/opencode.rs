use async_trait::async_trait;

use crate::{
    transport::{ExternalAgentSession, ExternalAgentTransport},
    types::{AgentTransportKind, ExternalAgentSpec},
};

const BINARY_NAME: &str = "opencode";

/// Transport for opencode CLI agent (interactive TUI via tmux).
pub struct OpencodeTransport;

impl OpencodeTransport {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for OpencodeTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExternalAgentTransport for OpencodeTransport {
    fn name(&self) -> &str {
        "opencode"
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self)))]
    async fn is_available(&self) -> bool {
        which::which(BINARY_NAME).is_ok()
    }

    fn supported_kinds(&self) -> &[AgentTransportKind] {
        &[AgentTransportKind::Opencode]
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self, _spec)))]
    async fn start_session(
        &self,
        _spec: &ExternalAgentSpec,
    ) -> anyhow::Result<Box<dyn ExternalAgentSession>> {
        anyhow::bail!("opencode runtime not yet implemented")
    }
}
