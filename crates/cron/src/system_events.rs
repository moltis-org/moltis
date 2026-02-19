//! In-memory event buffer that the heartbeat drains.
//!
//! Background processes (cron jobs, exec completions) enqueue events here.
//! On the next heartbeat tick the queue is drained, and event summaries are
//! prepended to the heartbeat prompt so the LLM can relay noteworthy items.

use std::{
    collections::VecDeque,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use tokio::sync::Mutex;

/// Maximum events retained before oldest entries are dropped.
const MAX_EVENTS: usize = 20;

/// A single system event waiting for the heartbeat to process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemEvent {
    /// Human-readable event description.
    pub text: String,
    /// Origin tag, e.g. `"exec-event"`, `"cron:<id>"`.
    pub reason: String,
    /// When the event was enqueued (epoch milliseconds).
    pub enqueued_at_ms: u64,
}

/// Thread-safe, bounded, dedup-aware event buffer.
pub struct SystemEventsQueue {
    events: Mutex<VecDeque<SystemEvent>>,
}

impl SystemEventsQueue {
    /// Create a new empty queue.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            events: Mutex::new(VecDeque::new()),
        })
    }

    /// Enqueue an event. Consecutive duplicate `text` values are deduplicated.
    /// When the buffer exceeds [`MAX_EVENTS`], the oldest entry is dropped.
    pub async fn enqueue(&self, text: String, reason: String) {
        let mut events = self.events.lock().await;

        // Dedup consecutive identical text.
        if events.back().is_some_and(|last| last.text == text) {
            return;
        }

        let enqueued_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        if events.len() >= MAX_EVENTS {
            events.pop_front();
        }

        events.push_back(SystemEvent {
            text,
            reason,
            enqueued_at_ms,
        });
    }

    /// Drain all pending events, returning them in FIFO order.
    pub async fn drain(&self) -> Vec<SystemEvent> {
        let mut events = self.events.lock().await;
        events.drain(..).collect()
    }

    /// Peek at all pending events without removing them.
    pub async fn peek(&self) -> Vec<SystemEvent> {
        let events = self.events.lock().await;
        events.iter().cloned().collect()
    }

    /// Check whether the queue is empty.
    pub async fn is_empty(&self) -> bool {
        self.events.lock().await.is_empty()
    }
}

impl Default for SystemEventsQueue {
    fn default() -> Self {
        Self {
            events: Mutex::new(VecDeque::new()),
        }
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn enqueue_and_drain() {
        let q = SystemEventsQueue::new();
        q.enqueue("hello".into(), "test".into()).await;
        q.enqueue("world".into(), "test".into()).await;
        let events = q.drain().await;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].text, "hello");
        assert_eq!(events[1].text, "world");
        assert!(q.is_empty().await);
    }

    #[tokio::test]
    async fn dedup_consecutive_identical() {
        let q = SystemEventsQueue::new();
        q.enqueue("same".into(), "a".into()).await;
        q.enqueue("same".into(), "b".into()).await;
        q.enqueue("different".into(), "c".into()).await;
        q.enqueue("same".into(), "d".into()).await;
        let events = q.drain().await;
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].text, "same");
        assert_eq!(events[1].text, "different");
        assert_eq!(events[2].text, "same");
    }

    #[tokio::test]
    async fn capacity_drops_oldest() {
        let q = SystemEventsQueue::new();
        for i in 0..25 {
            q.enqueue(format!("event-{i}"), "test".into()).await;
        }
        let events = q.drain().await;
        assert_eq!(events.len(), MAX_EVENTS);
        assert_eq!(events[0].text, "event-5");
        assert_eq!(events[MAX_EVENTS - 1].text, "event-24");
    }

    #[tokio::test]
    async fn peek_does_not_remove() {
        let q = SystemEventsQueue::new();
        q.enqueue("peek-me".into(), "test".into()).await;
        let peeked = q.peek().await;
        assert_eq!(peeked.len(), 1);
        assert!(!q.is_empty().await);
        let drained = q.drain().await;
        assert_eq!(drained.len(), 1);
    }

    #[tokio::test]
    async fn drain_empty() {
        let q = SystemEventsQueue::new();
        let events = q.drain().await;
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn enqueued_at_ms_is_set() {
        let q = SystemEventsQueue::new();
        q.enqueue("timed".into(), "test".into()).await;
        let events = q.peek().await;
        assert!(events[0].enqueued_at_ms > 0);
    }
}
