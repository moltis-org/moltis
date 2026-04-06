use std::pin::Pin;

use {async_trait::async_trait, futures::Stream};

use crate::types::{
    AgentTransportKind, ContextSnapshot, ExternalAgentEvent, ExternalAgentSpec, ExternalAgentStatus,
};

/// A transport backend that can spawn and manage external agent sessions.
///
/// Each transport handles a specific set of [`AgentTransportKind`] values
/// (e.g. PTY-based for Claude Code / opencode, JSON-RPC for Codex / ACP).
#[async_trait]
pub trait ExternalAgentTransport: Send + Sync {
    /// Human-readable transport name (e.g. "pty", "json-rpc").
    fn name(&self) -> &str;

    /// Check whether the required CLI binary is available on `$PATH`.
    async fn is_available(&self) -> bool;

    /// Which agent kinds this transport can handle.
    fn supported_kinds(&self) -> &[AgentTransportKind];

    /// Start a new session with the given spec.
    async fn start_session(
        &self,
        spec: &ExternalAgentSpec,
    ) -> anyhow::Result<Box<dyn ExternalAgentSession>>;
}

/// A live session with an external CLI agent.
///
/// The session owns the child process (PTY, tmux, or JSON-RPC subprocess)
/// and provides a streaming interface for sending prompts and receiving events.
#[async_trait]
pub trait ExternalAgentSession: Send + Sync {
    /// The external agent's own session/thread identifier, if available.
    fn external_session_id(&self) -> Option<&str>;

    /// Send a prompt to the agent, optionally with a context snapshot.
    ///
    /// Returns a stream of [`ExternalAgentEvent`]s.
    async fn send_prompt(
        &mut self,
        prompt: &str,
        context: Option<&ContextSnapshot>,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = ExternalAgentEvent> + Send>>>;

    /// Whether the underlying process is still alive.
    async fn is_alive(&self) -> bool;

    /// Gracefully shut down the external agent.
    async fn shutdown(&mut self) -> anyhow::Result<()>;

    /// Current status of the session.
    fn status(&self) -> ExternalAgentStatus;
}
