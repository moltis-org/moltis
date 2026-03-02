//! Session event bus for cross-UI synchronization.
//!
//! A `tokio::sync::broadcast` channel carries lightweight [`SessionEvent`]
//! values so both the macOS bridge FFI and gateway WebSocket layer can
//! observe session lifecycle updates.

use tokio::sync::broadcast;

/// A change to a session that other UIs should know about.
#[derive(Clone, Debug)]
pub enum SessionEvent {
    Created { session_key: String },
    Deleted { session_key: String },
    Patched { session_key: String },
}

/// Thin wrapper around a `broadcast::Sender<SessionEvent>`.
///
/// Cloning the bus is cheap (Arc internally) and gives each holder
/// the ability to both publish and subscribe.
#[derive(Clone, Debug)]
pub struct SessionEventBus {
    tx: broadcast::Sender<SessionEvent>,
}

impl Default for SessionEventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionEventBus {
    /// Create a new bus with a bounded capacity of 64 events.
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(64);
        Self { tx }
    }

    /// Publish an event. Returns the number of active receivers.
    /// If there are no subscribers the event is silently dropped.
    pub fn publish(&self, event: SessionEvent) -> usize {
        // `send` returns Err only when there are zero receivers, not a problem.
        self.tx.send(event).unwrap_or(0)
    }

    /// Subscribe to the event stream.
    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[tokio::test]
    async fn publish_and_receive() {
        let bus = SessionEventBus::new();
        let mut rx = bus.subscribe();

        bus.publish(SessionEvent::Created {
            session_key: "s1".into(),
        });
        bus.publish(SessionEvent::Patched {
            session_key: "s1".into(),
        });
        bus.publish(SessionEvent::Deleted {
            session_key: "s1".into(),
        });

        let e1 = rx.recv().await.unwrap();
        assert!(matches!(e1, SessionEvent::Created { session_key } if session_key == "s1"));

        let e2 = rx.recv().await.unwrap();
        assert!(matches!(e2, SessionEvent::Patched { session_key } if session_key == "s1"));

        let e3 = rx.recv().await.unwrap();
        assert!(matches!(e3, SessionEvent::Deleted { session_key } if session_key == "s1"));
    }

    #[tokio::test]
    async fn publish_without_subscribers_does_not_panic() {
        let bus = SessionEventBus::new();
        let count = bus.publish(SessionEvent::Created {
            session_key: "orphan".into(),
        });
        assert_eq!(count, 0);
    }
}
