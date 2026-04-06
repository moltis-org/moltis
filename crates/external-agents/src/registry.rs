use std::collections::HashMap;

use crate::{
    error::ExternalAgentError,
    transport::{ExternalAgentSession, ExternalAgentTransport},
    types::{AgentTransportKind, ExternalAgentInfo, ExternalAgentSpec},
};

/// Registry of external agent transports.
///
/// Dispatches session creation to the correct transport based on the
/// requested [`AgentTransportKind`].
pub struct ExternalAgentRegistry {
    transports: Vec<Box<dyn ExternalAgentTransport>>,
    kind_index: HashMap<AgentTransportKind, usize>,
}

impl ExternalAgentRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            transports: Vec::new(),
            kind_index: HashMap::new(),
        }
    }

    /// Register a transport. All kinds it supports are indexed.
    pub fn register(&mut self, transport: Box<dyn ExternalAgentTransport>) {
        let idx = self.transports.len();
        for kind in transport.supported_kinds() {
            self.kind_index.insert(*kind, idx);
        }
        self.transports.push(transport);
    }

    /// Start a session with the appropriate transport for the given spec.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self, spec), fields(kind = %spec.kind)))]
    pub async fn start_session(
        &self,
        spec: &ExternalAgentSpec,
    ) -> Result<Box<dyn ExternalAgentSession>, ExternalAgentError> {
        let idx = self.kind_index.get(&spec.kind).ok_or_else(|| {
            ExternalAgentError::NoRuntimeForKind {
                kind: spec.kind.to_string(),
            }
        })?;
        let transport = &self.transports[*idx];
        transport
            .start_session(spec)
            .await
            .map_err(|e| ExternalAgentError::SessionStartFailed {
                reason: e.to_string(),
            })
    }

    /// List all known agent kinds with their availability status.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self)))]
    pub async fn list_agents(&self) -> Vec<ExternalAgentInfo> {
        let mut infos = Vec::new();
        for transport in &self.transports {
            let installed = transport.is_available().await;
            for kind in transport.supported_kinds() {
                infos.push(ExternalAgentInfo {
                    kind: *kind,
                    name: kind.as_str().to_string(),
                    installed,
                    version: None,
                });
            }
        }
        infos
    }

    /// Check whether any transport is registered for the given kind.
    #[must_use]
    pub fn has_kind(&self, kind: AgentTransportKind) -> bool {
        self.kind_index.contains_key(&kind)
    }
}

impl Default for ExternalAgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{
            transport::{ExternalAgentSession, ExternalAgentTransport},
            types::{
                AgentTransportKind, ContextSnapshot, ExternalAgentEvent, ExternalAgentSpec,
                ExternalAgentStatus,
            },
        },
        async_trait::async_trait,
        std::pin::Pin,
    };

    struct FakeTransport;

    #[async_trait]
    impl ExternalAgentTransport for FakeTransport {
        fn name(&self) -> &str {
            "fake"
        }

        async fn is_available(&self) -> bool {
            true
        }

        fn supported_kinds(&self) -> &[AgentTransportKind] {
            &[AgentTransportKind::ClaudeCode]
        }

        async fn start_session(
            &self,
            _spec: &ExternalAgentSpec,
        ) -> anyhow::Result<Box<dyn ExternalAgentSession>> {
            Ok(Box::new(FakeSession))
        }
    }

    struct FakeSession;

    #[async_trait]
    impl ExternalAgentSession for FakeSession {
        fn external_session_id(&self) -> Option<&str> {
            Some("fake-123")
        }

        async fn send_prompt(
            &mut self,
            _prompt: &str,
            _context: Option<&ContextSnapshot>,
        ) -> anyhow::Result<Pin<Box<dyn futures::Stream<Item = ExternalAgentEvent> + Send>>>
        {
            Ok(Box::pin(futures::stream::empty()))
        }

        async fn is_alive(&self) -> bool {
            true
        }

        async fn shutdown(&mut self) -> anyhow::Result<()> {
            Ok(())
        }

        fn status(&self) -> ExternalAgentStatus {
            ExternalAgentStatus::Idle
        }
    }

    #[tokio::test]
    async fn registry_dispatch() {
        let mut registry = ExternalAgentRegistry::new();
        registry.register(Box::new(FakeTransport));

        assert!(registry.has_kind(AgentTransportKind::ClaudeCode));
        assert!(!registry.has_kind(AgentTransportKind::Codex));

        let spec = ExternalAgentSpec::new(AgentTransportKind::ClaudeCode);
        let session = registry.start_session(&spec).await;
        assert!(session.is_ok());

        let agents = registry.list_agents().await;
        assert_eq!(agents.len(), 1);
        assert!(agents[0].installed);
    }

    #[tokio::test]
    async fn registry_unknown_kind() {
        let registry = ExternalAgentRegistry::new();
        let spec = ExternalAgentSpec::new(AgentTransportKind::Codex);
        let result = registry.start_session(&spec).await;
        assert!(result.is_err());
    }
}
