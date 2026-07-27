//! The seam between the (`!Send`) ACP protocol handler and Moltis's (`Send`)
//! services.
//!
//! `agent_client_protocol` declares its traits with `#[async_trait(?Send)]`, so
//! every future the protocol handler produces is pinned to the thread running
//! the [`tokio::task::LocalSet`]. Moltis's `ChatService` is `Send + Sync` and
//! expects to run on the multi-threaded runtime.
//!
//! [`AcpBackend`] is where the two meet. It is deliberately `Send + Sync`, so
//! implementations live entirely in the threaded world and never learn that a
//! `LocalSet` exists. Streaming flows the other way through [`TurnUpdates`],
//! which is a plain channel sender: the backend pushes updates from whatever
//! task it likes, and the protocol side forwards them as `session/update`
//! notifications from the local thread.

use {agent_client_protocol as acp, async_trait::async_trait, tokio::sync::mpsc};

use crate::{session::SessionKey, setup::SessionSetup};

/// Sink for `session/update` notifications emitted while a turn is running.
///
/// Cloneable and `Send`, so a backend may hand it to spawned tasks. Sends are
/// non-blocking and infallible from the caller's point of view: once the client
/// has gone away the updates are simply dropped, and the turn will notice when
/// it tries to finish.
#[derive(Clone, Debug)]
pub struct TurnUpdates {
    tx: mpsc::UnboundedSender<acp::SessionUpdate>,
}

impl TurnUpdates {
    #[must_use]
    pub fn new(tx: mpsc::UnboundedSender<acp::SessionUpdate>) -> Self {
        Self { tx }
    }

    /// Sends a raw update. Returns `false` once the receiver is gone.
    pub fn send(&self, update: acp::SessionUpdate) -> bool {
        self.tx.send(update).is_ok()
    }

    /// Streams a chunk of the agent's visible reply.
    pub fn agent_message(&self, text: impl Into<String>) -> bool {
        self.send(acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new(acp::ContentBlock::from(text.into())),
        ))
    }

    /// Streams a chunk of the agent's reasoning.
    pub fn agent_thought(&self, text: impl Into<String>) -> bool {
        self.send(acp::SessionUpdate::AgentThoughtChunk(
            acp::ContentChunk::new(acp::ContentBlock::from(text.into())),
        ))
    }

    /// Returns whether the client is still listening.
    #[must_use]
    pub fn is_open(&self) -> bool {
        !self.tx.is_closed()
    }
}

/// What an [`AcpBackend`] supports, surfaced to the client during `initialize`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BackendCapabilities {
    /// Whether `session/load` can resume a previously created session.
    pub load_session: bool,
}

/// Moltis-side implementation of a single ACP conversation surface.
///
/// One process serves one client, matching how ACP harnesses spawn agents, but
/// a backend may hold several sessions at once because a client is free to open
/// more than one.
#[async_trait]
pub trait AcpBackend: Send + Sync + 'static {
    /// Creates a new session and returns its Moltis key.
    async fn create_session(&self, setup: &SessionSetup) -> anyhow::Result<SessionKey>;

    /// Resumes an existing session, returning its history so the protocol layer
    /// can replay it to the client as `session/update` notifications.
    ///
    /// Only called when [`BackendCapabilities::load_session`] is set.
    async fn load_session(
        &self,
        _key: &SessionKey,
        _setup: &SessionSetup,
    ) -> anyhow::Result<Vec<acp::SessionUpdate>> {
        Err(anyhow::anyhow!("session/load is not supported"))
    }

    /// Runs one turn to completion.
    ///
    /// Must not return until the turn is over: deltas go out through `updates`
    /// while this future is pending, and the returned stop reason is what
    /// resolves the client's `session/prompt` call.
    async fn prompt(
        &self,
        key: &SessionKey,
        prompt: String,
        updates: TurnUpdates,
    ) -> anyhow::Result<acp::StopReason>;

    /// Aborts the in-flight turn for `key`, if any.
    ///
    /// Arrives out-of-band while `prompt` is still pending, so it must not wait
    /// on that turn. The pending `prompt` is expected to wind up promptly
    /// afterwards.
    async fn cancel(&self, key: &SessionKey) -> anyhow::Result<()>;

    /// Releases connection-scoped turns, processes, and registrations.
    async fn shutdown(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::default()
    }
}
