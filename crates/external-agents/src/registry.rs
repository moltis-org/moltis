use std::{collections::HashMap, pin::Pin};

use futures::Stream;

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, external_agents as external_agent_metrics, histogram, labels};
#[cfg(feature = "metrics")]
use std::time::Instant;

use crate::{
    error::ExternalAgentError,
    transport::{ExternalAgentSession, ExternalAgentTransport},
    types::{
        AgentTransportKind, ContextSnapshot, ExternalAgentEvent, ExternalAgentInfo,
        ExternalAgentSpec, ExternalAgentStatus,
    },
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
        let transport_name = transport.name().to_string();

        #[cfg(feature = "metrics")]
        let start = Instant::now();

        #[cfg(feature = "metrics")]
        counter!(
            external_agent_metrics::SESSION_STARTS_TOTAL,
            labels::KIND => spec.kind.as_str(),
            labels::TRANSPORT => transport_name.clone()
        )
        .increment(1);

        let session = transport.start_session(spec).await;

        #[cfg(feature = "metrics")]
        histogram!(
            external_agent_metrics::SESSION_START_DURATION_SECONDS,
            labels::KIND => spec.kind.as_str(),
            labels::TRANSPORT => transport_name.clone()
        )
        .record(start.elapsed().as_secs_f64());

        let session = session.map_err(|e| {
            #[cfg(feature = "metrics")]
            counter!(
                external_agent_metrics::SESSION_START_ERRORS_TOTAL,
                labels::KIND => spec.kind.as_str(),
                labels::TRANSPORT => transport_name.clone(),
                labels::ERROR_TYPE => "transport_start"
            )
            .increment(1);

            ExternalAgentError::SessionStartFailed {
                reason: e.to_string(),
            }
        })?;

        Ok(Box::new(InstrumentedSession::new(
            session,
            spec.kind,
            transport_name,
        )))
    }

    /// List all known agent kinds with their availability status.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self)))]
    pub async fn list_agents(&self) -> Vec<ExternalAgentInfo> {
        let mut infos = Vec::new();
        for transport in &self.transports {
            #[cfg(feature = "metrics")]
            let transport_name = transport.name().to_string();

            #[cfg(feature = "metrics")]
            let start = Instant::now();

            let installed = transport.is_available().await;

            #[cfg(feature = "metrics")]
            counter!(
                external_agent_metrics::TRANSPORT_AVAILABILITY_CHECKS_TOTAL,
                labels::TRANSPORT => transport_name.clone(),
                labels::SUCCESS => installed.to_string()
            )
            .increment(1);

            #[cfg(feature = "metrics")]
            histogram!(
                external_agent_metrics::TRANSPORT_AVAILABILITY_DURATION_SECONDS,
                labels::TRANSPORT => transport_name
            )
            .record(start.elapsed().as_secs_f64());

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

struct InstrumentedSession {
    inner: Box<dyn ExternalAgentSession>,
    #[cfg(any(feature = "metrics", feature = "tracing"))]
    kind: AgentTransportKind,
    #[cfg(any(feature = "metrics", feature = "tracing"))]
    transport_name: String,
}

impl InstrumentedSession {
    fn new(
        inner: Box<dyn ExternalAgentSession>,
        kind: AgentTransportKind,
        transport_name: String,
    ) -> Self {
        #[cfg(not(any(feature = "metrics", feature = "tracing")))]
        let _ = (kind, transport_name);

        Self {
            inner,
            #[cfg(any(feature = "metrics", feature = "tracing"))]
            kind,
            #[cfg(any(feature = "metrics", feature = "tracing"))]
            transport_name,
        }
    }
}

#[async_trait::async_trait]
impl ExternalAgentSession for InstrumentedSession {
    fn external_session_id(&self) -> Option<&str> {
        self.inner.external_session_id()
    }

    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(
            skip(self, prompt, context),
            fields(
                kind = %self.kind,
                transport = %self.transport_name,
                prompt_len = prompt.len(),
                has_context = context.is_some()
            )
        )
    )]
    async fn send_prompt(
        &mut self,
        prompt: &str,
        context: Option<&ContextSnapshot>,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = ExternalAgentEvent> + Send>>> {
        #[cfg(feature = "metrics")]
        let start = Instant::now();

        #[cfg(feature = "metrics")]
        counter!(
            external_agent_metrics::PROMPTS_TOTAL,
            labels::KIND => self.kind.as_str(),
            labels::TRANSPORT => self.transport_name.clone()
        )
        .increment(1);

        let result = self.inner.send_prompt(prompt, context).await;

        #[cfg(feature = "metrics")]
        histogram!(
            external_agent_metrics::PROMPT_DURATION_SECONDS,
            labels::KIND => self.kind.as_str(),
            labels::TRANSPORT => self.transport_name.clone()
        )
        .record(start.elapsed().as_secs_f64());

        if let Err(_error) = &result {
            #[cfg(feature = "metrics")]
            counter!(
                external_agent_metrics::PROMPT_ERRORS_TOTAL,
                labels::KIND => self.kind.as_str(),
                labels::TRANSPORT => self.transport_name.clone(),
                labels::ERROR_TYPE => "prompt_delivery"
            )
            .increment(1);
        }

        result
    }

    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(skip(self), fields(kind = %self.kind, transport = %self.transport_name))
    )]
    async fn is_alive(&self) -> bool {
        self.inner.is_alive().await
    }

    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(skip(self), fields(kind = %self.kind, transport = %self.transport_name))
    )]
    async fn shutdown(&mut self) -> anyhow::Result<()> {
        #[cfg(feature = "metrics")]
        let start = Instant::now();

        #[cfg(feature = "metrics")]
        counter!(
            external_agent_metrics::SESSION_SHUTDOWNS_TOTAL,
            labels::KIND => self.kind.as_str(),
            labels::TRANSPORT => self.transport_name.clone()
        )
        .increment(1);

        let result = self.inner.shutdown().await;

        #[cfg(feature = "metrics")]
        histogram!(
            external_agent_metrics::SESSION_SHUTDOWN_DURATION_SECONDS,
            labels::KIND => self.kind.as_str(),
            labels::TRANSPORT => self.transport_name.clone()
        )
        .record(start.elapsed().as_secs_f64());

        if let Err(_error) = &result {
            #[cfg(feature = "metrics")]
            counter!(
                external_agent_metrics::SESSION_SHUTDOWN_ERRORS_TOTAL,
                labels::KIND => self.kind.as_str(),
                labels::TRANSPORT => self.transport_name.clone(),
                labels::ERROR_TYPE => "shutdown"
            )
            .increment(1);
        }

        result
    }

    fn status(&self) -> ExternalAgentStatus {
        self.inner.status()
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
        futures::StreamExt,
        std::{
            pin::Pin,
            sync::{
                Arc,
                atomic::{AtomicUsize, Ordering},
            },
        },
    };

    #[derive(Default)]
    struct SessionTracker {
        send_prompt_calls: AtomicUsize,
        is_alive_calls: AtomicUsize,
        shutdown_calls: AtomicUsize,
    }

    struct FakeTransport {
        tracker: Arc<SessionTracker>,
    }

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
            Ok(Box::new(FakeSession {
                tracker: Arc::clone(&self.tracker),
            }))
        }
    }

    struct FakeSession {
        tracker: Arc<SessionTracker>,
    }

    #[async_trait]
    impl ExternalAgentSession for FakeSession {
        fn external_session_id(&self) -> Option<&str> {
            Some("fake-123")
        }

        async fn send_prompt(
            &mut self,
            _prompt: &str,
            _context: Option<&ContextSnapshot>,
        ) -> anyhow::Result<Pin<Box<dyn Stream<Item = ExternalAgentEvent> + Send>>> {
            self.tracker
                .send_prompt_calls
                .fetch_add(1, Ordering::SeqCst);
            Ok(Box::pin(futures::stream::empty()))
        }

        async fn is_alive(&self) -> bool {
            self.tracker.is_alive_calls.fetch_add(1, Ordering::SeqCst);
            true
        }

        async fn shutdown(&mut self) -> anyhow::Result<()> {
            self.tracker.shutdown_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn status(&self) -> ExternalAgentStatus {
            ExternalAgentStatus::Idle
        }
    }

    #[tokio::test]
    async fn registry_dispatch() {
        let mut registry = ExternalAgentRegistry::new();
        registry.register(Box::new(FakeTransport {
            tracker: Arc::new(SessionTracker::default()),
        }));

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

    #[tokio::test]
    async fn registry_wraps_sessions_and_preserves_behavior() {
        let tracker = Arc::new(SessionTracker::default());
        let mut registry = ExternalAgentRegistry::new();
        registry.register(Box::new(FakeTransport {
            tracker: Arc::clone(&tracker),
        }));

        let spec = ExternalAgentSpec::new(AgentTransportKind::ClaudeCode);
        let mut session = match registry.start_session(&spec).await {
            Ok(session) => session,
            Err(error) => panic!("session should start: {error}"),
        };

        assert_eq!(session.external_session_id(), Some("fake-123"));
        assert_eq!(session.status(), ExternalAgentStatus::Idle);

        let mut events = match session.send_prompt("hello", None).await {
            Ok(events) => events,
            Err(error) => panic!("prompt should send: {error}"),
        };
        assert!(events.next().await.is_none());
        assert!(session.is_alive().await);
        if let Err(error) = session.shutdown().await {
            panic!("shutdown should succeed: {error}");
        }

        assert_eq!(tracker.send_prompt_calls.load(Ordering::SeqCst), 1);
        assert_eq!(tracker.is_alive_calls.load(Ordering::SeqCst), 1);
        assert_eq!(tracker.shutdown_calls.load(Ordering::SeqCst), 1);
    }
}
