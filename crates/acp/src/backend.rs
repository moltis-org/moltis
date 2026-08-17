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

pub const MAX_HISTORY_UPDATES: usize = 10_000;
pub const MAX_HISTORY_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_TURN_UPDATE_BYTES: usize = 8 * 1024 * 1024;

/// Marker used by backends when a syntactically valid ACP session does not
/// exist. Other load failures are operational and must not be blamed on input.
#[derive(Debug)]
pub struct SessionNotFound;

impl std::fmt::Display for SessionNotFound {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("session does not exist")
    }
}

impl std::error::Error for SessionNotFound {}

pub fn validate_history(updates: &[acp::SessionUpdate]) -> anyhow::Result<()> {
    if updates.len() > MAX_HISTORY_UPDATES {
        anyhow::bail!("session history exceeds {MAX_HISTORY_UPDATES} updates");
    }
    let bytes = updates.iter().try_fold(0usize, |total, update| {
        Ok::<_, serde_json::Error>(total.saturating_add(serde_json::to_vec(update)?.len()))
    })?;
    if bytes > MAX_HISTORY_BYTES {
        anyhow::bail!("session history exceeds {MAX_HISTORY_BYTES} bytes");
    }
    Ok(())
}

/// Sink for `session/update` notifications emitted while a turn is running.
///
/// Cloneable and `Send`, so a backend may hand it to spawned tasks. Sends are
/// non-blocking. A send returns `false` if the bounded buffer is full or the
/// protocol task has stopped listening, allowing the backend to abort rather
/// than growing memory without limit.
#[derive(Clone, Debug)]
pub struct TurnUpdates {
    tx: mpsc::UnboundedSender<acp::SessionUpdate>,
    bytes_sent: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    limit_exceeded: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl TurnUpdates {
    #[must_use]
    pub fn new(tx: mpsc::UnboundedSender<acp::SessionUpdate>) -> Self {
        Self {
            tx,
            bytes_sent: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            limit_exceeded: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Sends a raw update. Returns `false` when the byte budget is exhausted or
    /// the protocol task has stopped listening.
    pub fn send(&self, update: acp::SessionUpdate) -> bool {
        let Ok(bytes) = serde_json::to_vec(&update).map(|value| value.len()) else {
            self.limit_exceeded
                .store(true, std::sync::atomic::Ordering::Release);
            return false;
        };
        let reserved = self.bytes_sent.fetch_update(
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
            |used| {
                used.checked_add(bytes)
                    .filter(|total| *total <= MAX_TURN_UPDATE_BYTES)
            },
        );
        if reserved.is_err() {
            self.limit_exceeded
                .store(true, std::sync::atomic::Ordering::Release);
            return false;
        }
        if self.tx.send(update).is_ok() {
            return true;
        }
        self.bytes_sent
            .fetch_sub(bytes, std::sync::atomic::Ordering::AcqRel);
        false
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

    #[must_use]
    pub fn limit_exceeded(&self) -> bool {
        self.limit_exceeded
            .load(std::sync::atomic::Ordering::Acquire)
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

    /// Rolls back connection-scoped resources for a session setup that the
    /// protocol layer could not retain or replay.
    async fn discard_session(&self, _key: &SessionKey) -> anyhow::Result<()> {
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_update_budget_is_enforced_before_enqueue() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let updates = TurnUpdates::new(tx);
        let chunk = "x".repeat(5 * 1024 * 1024);
        assert!(updates.agent_message(chunk.clone()));
        assert!(!updates.agent_message(chunk));
        assert!(updates.limit_exceeded());
    }

    #[test]
    fn many_small_updates_are_limited_by_bytes_not_item_count() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let updates = TurnUpdates::new(tx);
        assert!((0..2_000).all(|_| updates.agent_message("x")));
        assert!(!updates.limit_exceeded());
    }
}
