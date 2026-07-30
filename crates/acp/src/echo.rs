//! A backend with no Moltis behind it.
//!
//! Useful for smoke-testing a client's handshake (`moltis acp --echo`) and as
//! the fixture the protocol tests drive, so protocol coverage does not require
//! standing up a gateway.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use {agent_client_protocol as acp, async_trait::async_trait};

use crate::{
    backend::{AcpBackend, BackendCapabilities, TurnUpdates},
    session::SessionKey,
    setup::SessionSetup,
};

/// Echoes the prompt back as a streamed agent message.
#[derive(Debug, Default)]
pub struct EchoBackend {
    next_id: AtomicU64,
    cancels: Mutex<HashMap<SessionKey, Arc<AtomicBool>>>,
}

impl EchoBackend {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn cancel_flag(&self, key: &SessionKey) -> Arc<AtomicBool> {
        let mut cancels = self
            .cancels
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Arc::clone(cancels.entry(key.clone()).or_default())
    }
}

#[async_trait]
impl AcpBackend for EchoBackend {
    async fn create_session(&self, _setup: &SessionSetup) -> anyhow::Result<SessionKey> {
        let id = self
            .next_id
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        Ok(SessionKey::namespaced(format!("echo-{id}")))
    }

    async fn prompt(
        &self,
        key: &SessionKey,
        prompt: String,
        updates: TurnUpdates,
    ) -> anyhow::Result<acp::StopReason> {
        let cancelled = self.cancel_flag(key);
        cancelled.store(false, Ordering::SeqCst);

        updates.agent_thought("echoing the prompt back");
        // Chunked so clients exercise their streaming path rather than
        // receiving one atomic message.
        for word in prompt.split_inclusive(char::is_whitespace) {
            if cancelled.load(Ordering::SeqCst) {
                return Ok(acp::StopReason::Cancelled);
            }
            if !updates.agent_message(word.to_string()) {
                return Ok(acp::StopReason::Cancelled);
            }
        }
        Ok(acp::StopReason::EndTurn)
    }

    async fn cancel(&self, key: &SessionKey) -> anyhow::Result<()> {
        self.cancel_flag(key).store(true, Ordering::SeqCst);
        Ok(())
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            load_session: false,
        }
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, tokio::sync::mpsc};

    async fn setup() -> SessionSetup {
        SessionSetup::new(std::env::temp_dir(), Vec::new())
            .await
            .expect("setup")
    }

    #[tokio::test]
    async fn sessions_are_namespaced_and_unique() {
        let backend = EchoBackend::new();
        let first = backend
            .create_session(&setup().await)
            .await
            .expect("session");
        let second = backend
            .create_session(&setup().await)
            .await
            .expect("session");
        assert!(first.is_namespaced());
        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn prompt_streams_chunks_then_ends_turn() {
        let backend = EchoBackend::new();
        let key = backend
            .create_session(&setup().await)
            .await
            .expect("session");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let reason = backend
            .prompt(&key, "hello there".to_string(), TurnUpdates::new(tx))
            .await
            .expect("turn");
        assert_eq!(reason, acp::StopReason::EndTurn);

        let mut text = String::new();
        let mut thoughts = 0;
        while let Ok(update) = rx.try_recv() {
            match update {
                acp::SessionUpdate::AgentMessageChunk(chunk) => {
                    if let acp::ContentBlock::Text(block) = chunk.content {
                        text.push_str(&block.text);
                    }
                },
                acp::SessionUpdate::AgentThoughtChunk(_) => thoughts += 1,
                _ => {},
            }
        }
        assert_eq!(text, "hello there");
        assert_eq!(thoughts, 1);
    }

    #[tokio::test]
    async fn cancel_stops_the_turn() {
        let backend = EchoBackend::new();
        let key = backend
            .create_session(&setup().await)
            .await
            .expect("session");
        backend.cancel(&key).await.expect("cancel");
        // The flag is cleared when a turn starts, so a cancel issued before the
        // turn must not leak into it.
        let (tx, _rx) = mpsc::unbounded_channel();
        let reason = backend
            .prompt(&key, "hello".to_string(), TurnUpdates::new(tx))
            .await
            .expect("turn");
        assert_eq!(reason, acp::StopReason::EndTurn);
    }

    #[tokio::test]
    async fn turn_ends_when_the_client_stops_listening() {
        let backend = EchoBackend::new();
        let key = backend
            .create_session(&setup().await)
            .await
            .expect("session");
        let (tx, rx) = mpsc::unbounded_channel();
        drop(rx);
        let reason = backend
            .prompt(&key, "hello".to_string(), TurnUpdates::new(tx))
            .await
            .expect("turn");
        assert_eq!(reason, acp::StopReason::Cancelled);
    }
}
