//! The [`ObservationSink`] boundary plus the process-wide registry.
//!
//! `record` is deliberately synchronous and non-blocking: instrumentation sits
//! on the agent's hot path, so a slow or unreachable backend must never stall a
//! turn. Implementations enqueue and return; dropping data is always preferable
//! to applying backpressure to the agent loop.

use std::{
    sync::{Arc, RwLock},
    time::Duration,
};

use async_trait::async_trait;

use crate::{model::Event, runtime::SinkStatsSnapshot};

/// Destination for observability events.
#[async_trait]
pub trait ObservationSink: Send + Sync {
    /// Sink name, used in logs and status reporting.
    fn name(&self) -> &str;

    /// Enqueue an event. Must not block and must not fail loudly.
    fn record(&self, event: Event);

    /// Flush pending events, giving up after `timeout`.
    async fn flush(&self, timeout: Duration) -> anyhow::Result<()>;

    /// Point-in-time delivery health for this sink and any sinks below it.
    fn delivery_stats(&self) -> Vec<SinkStatsSnapshot> {
        Vec::new()
    }
}

/// Fans one event out to several sinks, so Langfuse and an OTLP collector can
/// run side by side from a single instrumentation pass.
pub struct SinkFanout {
    sinks: Vec<Arc<dyn ObservationSink>>,
    name: String,
}

impl SinkFanout {
    /// Build a fanout over `sinks`.
    #[must_use]
    pub fn new(sinks: Vec<Arc<dyn ObservationSink>>) -> Self {
        let name = sinks
            .iter()
            .map(|s| s.name().to_string())
            .collect::<Vec<_>>()
            .join("+");
        Self { sinks, name }
    }

    /// Whether any sink is attached.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sinks.is_empty()
    }

    /// Number of attached sinks.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sinks.len()
    }

    /// Names of the attached sinks.
    #[must_use]
    pub fn sink_names(&self) -> Vec<String> {
        self.sinks.iter().map(|s| s.name().to_string()).collect()
    }
}

#[async_trait]
impl ObservationSink for SinkFanout {
    fn name(&self) -> &str {
        &self.name
    }

    fn record(&self, event: Event) {
        // Clone per sink except the last, which takes ownership.
        let mut iter = self.sinks.iter().peekable();
        while let Some(sink) = iter.next() {
            if iter.peek().is_some() {
                sink.record(event.clone());
            } else {
                sink.record(event);
                return;
            }
        }
    }

    async fn flush(&self, timeout: Duration) -> anyhow::Result<()> {
        // Flush concurrently: a slow backend must not serialise the others.
        let futures = self.sinks.iter().map(|s| s.flush(timeout));
        let results = futures::future::join_all(futures).await;

        let errors: Vec<String> = results
            .into_iter()
            .filter_map(|r| r.err().map(|e| e.to_string()))
            .collect();

        if errors.is_empty() {
            return Ok(());
        }
        Err(anyhow::anyhow!("sink flush failed: {}", errors.join("; ")))
    }

    fn delivery_stats(&self) -> Vec<SinkStatsSnapshot> {
        self.sinks
            .iter()
            .flat_map(|sink| sink.delivery_stats())
            .collect()
    }
}

// ── Process-wide registry ───────────────────────────────────────────────────

/// The active sink. `RwLock` rather than `OnceLock` because the settings UI can
/// reconfigure instrumentation without a restart.
static GLOBAL_SINK: RwLock<Option<Arc<dyn ObservationSink>>> = RwLock::new(None);

#[cfg(test)]
pub(crate) static GLOBAL_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Install `sink` as the process-wide destination, replacing any previous one.
pub fn set_global_sink(sink: Arc<dyn ObservationSink>) {
    match GLOBAL_SINK.write() {
        Ok(mut guard) => *guard = Some(sink),
        Err(poisoned) => *poisoned.into_inner() = Some(sink),
    }
}

/// Remove the process-wide sink, disabling instrumentation.
pub fn clear_global_sink() {
    match GLOBAL_SINK.write() {
        Ok(mut guard) => *guard = None,
        Err(poisoned) => *poisoned.into_inner() = None,
    }
}

/// The active sink, if instrumentation is configured.
#[must_use]
pub fn global_sink() -> Option<Arc<dyn ObservationSink>> {
    match GLOBAL_SINK.read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

/// Run a short operation while holding the global sink registry read lock.
///
/// Used when cloning the sink must be atomic with related bookkeeping, such as
/// registering an active turn before shutdown can clear the registry.
pub(crate) fn with_global_sink<T>(
    operation: impl FnOnce(&Arc<dyn ObservationSink>) -> T,
) -> Option<T> {
    match GLOBAL_SINK.read() {
        Ok(guard) => guard.as_ref().map(operation),
        Err(poisoned) => poisoned.into_inner().as_ref().map(operation),
    }
}

/// Whether instrumentation is active.
///
/// Call this before doing any work to build an event: when no sink is
/// installed, instrumentation must cost nothing beyond this check.
#[must_use]
pub fn is_enabled() -> bool {
    match GLOBAL_SINK.read() {
        Ok(guard) => guard.is_some(),
        Err(poisoned) => poisoned.into_inner().is_some(),
    }
}

/// Send `event` to the active sink, if any.
pub fn record(event: Event) {
    if let Some(sink) = global_sink() {
        sink.record(event);
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::sync::Mutex;

    use {
        super::*,
        crate::model::{ObservationKind, ObservationRecord, TraceId, TraceRecord},
    };

    struct CollectingSink {
        name: String,
        events: Mutex<Vec<Event>>,
    }

    impl CollectingSink {
        fn new(name: &str) -> Arc<Self> {
            Arc::new(Self {
                name: name.to_string(),
                events: Mutex::new(Vec::new()),
            })
        }

        fn count(&self) -> usize {
            self.events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len()
        }
    }

    #[async_trait]
    impl ObservationSink for CollectingSink {
        fn name(&self) -> &str {
            &self.name
        }

        fn record(&self, event: Event) {
            self.events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(event);
        }

        async fn flush(&self, _timeout: Duration) -> anyhow::Result<()> {
            Ok(())
        }
    }

    struct FailingSink;

    #[async_trait]
    impl ObservationSink for FailingSink {
        fn name(&self) -> &str {
            "failing"
        }

        fn record(&self, _event: Event) {}

        async fn flush(&self, _timeout: Duration) -> anyhow::Result<()> {
            Err(anyhow::anyhow!("backend unreachable"))
        }
    }

    fn sample_event() -> Event {
        Event::ObservationEnd(Box::new(ObservationRecord::start(
            TraceId::generate(),
            ObservationKind::Generation,
            "llm-call",
        )))
    }

    #[test]
    fn fanout_delivers_to_every_sink() {
        let a = CollectingSink::new("a");
        let b = CollectingSink::new("b");
        let c = CollectingSink::new("c");
        let fanout = SinkFanout::new(vec![a.clone(), b.clone(), c.clone()]);

        fanout.record(sample_event());

        assert_eq!(a.count(), 1);
        assert_eq!(b.count(), 1);
        assert_eq!(c.count(), 1);
    }

    #[test]
    fn fanout_name_lists_every_backend() {
        let fanout = SinkFanout::new(vec![
            CollectingSink::new("langfuse"),
            CollectingSink::new("otlp"),
        ]);
        assert_eq!(fanout.name(), "langfuse+otlp");
        assert_eq!(fanout.len(), 2);
        assert!(!fanout.is_empty());
    }

    #[test]
    fn empty_fanout_accepts_events_without_panicking() {
        let fanout = SinkFanout::new(Vec::new());
        fanout.record(sample_event());
        assert!(fanout.is_empty());
    }

    #[tokio::test]
    async fn fanout_flush_reports_every_failure() {
        let fanout = SinkFanout::new(vec![
            CollectingSink::new("ok"),
            Arc::new(FailingSink),
            Arc::new(FailingSink),
        ]);

        let error = fanout
            .flush(Duration::from_millis(10))
            .await
            .expect_err("failing sinks must surface an error");
        let rendered = error.to_string();

        // Both failures must be reported, not just the first.
        assert_eq!(rendered.matches("backend unreachable").count(), 2);
    }

    #[tokio::test]
    async fn fanout_flush_succeeds_when_all_sinks_succeed() {
        let fanout = SinkFanout::new(vec![CollectingSink::new("a"), CollectingSink::new("b")]);
        assert!(fanout.flush(Duration::from_millis(10)).await.is_ok());
    }

    // The global registry is process-wide state, so these assertions live in a
    // single test to stay independent of test execution order.
    #[test]
    fn global_registry_install_record_and_clear() {
        let _guard = GLOBAL_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let sink = CollectingSink::new("global");

        assert!(!is_enabled());
        record(sample_event());
        assert_eq!(sink.count(), 0, "events must drop while no sink installed");

        set_global_sink(sink.clone());
        assert!(is_enabled());
        assert!(global_sink().is_some());

        record(sample_event());
        assert_eq!(sink.count(), 1);

        clear_global_sink();
        assert!(!is_enabled());
        record(sample_event());
        assert_eq!(sink.count(), 1, "events must drop after clearing the sink");
    }

    #[test]
    fn trace_event_reports_owning_trace() {
        let trace = TraceRecord::new("turn");
        let id = trace.id.clone();
        let event = Event::Trace(Box::new(trace));
        assert_eq!(event.trace_id(), &id);
    }
}
