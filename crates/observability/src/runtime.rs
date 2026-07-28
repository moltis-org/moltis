//! Batching export runtime shared by every backend.
//!
//! A backend supplies a [`Transport`]; this module supplies everything around
//! it — a bounded queue, size- and time-based batching, retry with exponential
//! backoff and jitter, and drop accounting.
//!
//! The central invariant: **the agent loop is never blocked by telemetry**.
//! [`BatchSink::record`] uses `try_send` and drops on a full queue. Losing
//! traces is an acceptable failure; stalling a user's turn is not.

use std::{
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use {
    async_trait::async_trait,
    tokio::sync::{mpsc, oneshot},
    tracing::{debug, trace, warn},
};

use crate::{model::Event, sink::ObservationSink};

/// Why a transport send failed.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// Transient: worth retrying (5xx, 429, connection reset).
    #[error("retryable transport failure: {0}")]
    Retryable(String),
    /// Permanent: retrying cannot help (401, 400, malformed payload).
    #[error("fatal transport failure: {0}")]
    Fatal(String),
}

/// Backend-specific delivery mechanism.
#[async_trait]
pub trait Transport: Send + Sync + 'static {
    /// Transport name, used in logs and status reporting.
    fn name(&self) -> &str;

    /// Deliver a batch. Called from the background task only.
    async fn send(&self, batch: &[Event]) -> Result<(), TransportError>;

    /// Whether this transport can represent `event`. This runs on the recording
    /// hot path and must be fast and non-blocking.
    fn accepts(&self, _event: &Event) -> bool {
        true
    }

    /// Approximate serialized size of `event`, used for batch sizing.
    fn estimate_bytes(&self, event: &Event) -> usize {
        serde_json::to_vec(event).map(|v| v.len()).unwrap_or(1024)
    }
}

/// Batching and retry parameters.
#[derive(Debug, Clone)]
pub struct BatchConfig {
    /// Maximum events held before a forced flush.
    pub max_batch_events: usize,
    /// Maximum estimated batch size in bytes before a forced flush.
    pub max_batch_bytes: usize,
    /// Maximum time an event waits before being flushed.
    pub flush_interval: Duration,
    /// Bounded queue depth. Events are dropped once this fills.
    pub queue_capacity: usize,
    /// Retry attempts for retryable failures.
    pub max_retries: u32,
    /// Base delay for exponential backoff.
    pub initial_backoff: Duration,
    /// Ceiling for exponential backoff.
    pub max_backoff: Duration,
}

impl Default for BatchConfig {
    fn default() -> Self {
        Self {
            max_batch_events: 128,
            // Comfortably under Langfuse's per-request ceiling, leaving room
            // for the estimate to be wrong without the request being rejected.
            max_batch_bytes: 3_000_000,
            flush_interval: Duration::from_secs(5),
            queue_capacity: 10_000,
            max_retries: 3,
            initial_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(30),
        }
    }
}

/// Counters describing sink health, surfaced in the settings UI.
#[derive(Debug, Default)]
pub struct SinkStats {
    name: String,
    /// Events accepted into the queue.
    pub accepted: AtomicU64,
    /// Events dropped because the queue was full.
    pub dropped_queue_full: AtomicU64,
    /// Events dropped after exhausting retries or hitting a fatal error.
    pub dropped_failed: AtomicU64,
    /// Batches delivered successfully.
    pub batches_sent: AtomicU64,
    /// Batches that failed permanently.
    pub batches_failed: AtomicU64,
    /// Events delivered successfully.
    pub delivered: AtomicU64,
    /// Retry attempts made after retryable failures.
    pub retries: AtomicU64,
    last_success_at: RwLock<Option<String>>,
    last_error: RwLock<Option<String>>,
    last_error_at: RwLock<Option<String>>,
}

/// Point-in-time snapshot of [`SinkStats`].
#[derive(Debug, Clone, serde::Serialize)]
pub struct SinkStatsSnapshot {
    /// Sink whose counters are represented by this snapshot.
    pub name: String,
    /// Events accepted into the queue.
    pub accepted: u64,
    /// Events dropped because the queue was full.
    pub dropped_queue_full: u64,
    /// Events dropped after exhausting retries or hitting a fatal error.
    pub dropped_failed: u64,
    /// Batches delivered successfully.
    pub batches_sent: u64,
    /// Batches that failed permanently.
    pub batches_failed: u64,
    /// Events delivered successfully.
    pub delivered: u64,
    /// Retry attempts made after retryable failures.
    pub retries: u64,
    /// RFC 3339 timestamp of the latest successful delivery.
    pub last_success_at: Option<String>,
    /// Most recent delivery error.
    pub last_error: Option<String>,
    /// RFC 3339 timestamp of the most recent delivery error.
    pub last_error_at: Option<String>,
}

impl SinkStats {
    fn new(name: String) -> Self {
        Self {
            name,
            ..Default::default()
        }
    }

    /// Read every counter.
    #[must_use]
    pub fn snapshot(&self) -> SinkStatsSnapshot {
        SinkStatsSnapshot {
            name: self.name.clone(),
            accepted: self.accepted.load(Ordering::Relaxed),
            dropped_queue_full: self.dropped_queue_full.load(Ordering::Relaxed),
            dropped_failed: self.dropped_failed.load(Ordering::Relaxed),
            batches_sent: self.batches_sent.load(Ordering::Relaxed),
            batches_failed: self.batches_failed.load(Ordering::Relaxed),
            delivered: self.delivered.load(Ordering::Relaxed),
            retries: self.retries.load(Ordering::Relaxed),
            last_success_at: read_state(&self.last_success_at),
            last_error: read_state(&self.last_error),
            last_error_at: read_state(&self.last_error_at),
        }
    }

    fn record_success(&self, count: u64) {
        self.delivered.fetch_add(count, Ordering::Relaxed);
        write_state(&self.last_success_at, now_rfc3339());
    }

    fn record_error(&self, reason: String) {
        write_state(&self.last_error, Some(reason));
        write_state(&self.last_error_at, now_rfc3339());
    }
}

fn read_state(state: &RwLock<Option<String>>) -> Option<String> {
    state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

fn write_state(state: &RwLock<Option<String>>, value: Option<String>) {
    match state.write() {
        Ok(mut guard) => *guard = value,
        Err(poisoned) => *poisoned.into_inner() = value,
    }
}

fn now_rfc3339() -> Option<String> {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .ok()
}

/// Queue message: either an event or a flush barrier.
enum Message {
    Event(Box<Event>),
    Flush(oneshot::Sender<()>),
}

/// An [`ObservationSink`] that batches into a [`Transport`].
pub struct BatchSink {
    name: String,
    transport: Arc<dyn Transport>,
    tx: mpsc::Sender<Message>,
    stats: Arc<SinkStats>,
}

impl BatchSink {
    /// Spawn the background export task and return a sink feeding it.
    #[must_use]
    pub fn spawn(transport: Arc<dyn Transport>, config: BatchConfig) -> Self {
        let (tx, rx) = mpsc::channel(config.queue_capacity);
        let name = transport.name().to_string();
        let stats = Arc::new(SinkStats::new(name.clone()));

        tokio::spawn(export_loop(
            rx,
            Arc::clone(&transport),
            config,
            Arc::clone(&stats),
        ));

        Self {
            name,
            transport,
            tx,
            stats,
        }
    }

    /// Health counters for this sink.
    #[must_use]
    pub fn stats(&self) -> SinkStatsSnapshot {
        self.stats.snapshot()
    }
}

#[async_trait]
impl ObservationSink for BatchSink {
    fn name(&self) -> &str {
        &self.name
    }

    fn record(&self, event: Event) {
        if !self.transport.accepts(&event) {
            return;
        }

        // `try_send` never awaits: a saturated queue drops the event rather
        // than applying backpressure to the caller's turn.
        match self.tx.try_send(Message::Event(Box::new(event))) {
            Ok(()) => {
                self.stats.accepted.fetch_add(1, Ordering::Relaxed);
            },
            Err(mpsc::error::TrySendError::Full(_)) => {
                let dropped = self
                    .stats
                    .dropped_queue_full
                    .fetch_add(1, Ordering::Relaxed)
                    + 1;
                // Log sparsely: a saturated queue would otherwise generate more
                // log volume than the telemetry it is failing to send.
                if dropped.is_power_of_two() {
                    warn!(
                        sink = %self.name,
                        dropped,
                        "observability queue full; dropping events"
                    );
                }
            },
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.stats.dropped_failed.fetch_add(1, Ordering::Relaxed);
            },
        }

        #[cfg(feature = "metrics")]
        moltis_metrics::counter!(
            "moltis_observability_events_total",
            "sink" => self.name.clone()
        )
        .increment(1);
    }

    async fn flush(&self, timeout: Duration) -> anyhow::Result<()> {
        let flush = async {
            let (ack_tx, ack_rx) = oneshot::channel();
            if self.tx.send(Message::Flush(ack_tx)).await.is_err() {
                return Err(anyhow::anyhow!("export task for {} has stopped", self.name));
            }
            ack_rx.await.map_err(|_| {
                anyhow::anyhow!("export task for {} dropped the flush barrier", self.name)
            })
        };
        match tokio::time::timeout(timeout, flush).await {
            Ok(result) => result,
            Err(_) => Err(anyhow::anyhow!("flush of {} timed out", self.name)),
        }
    }

    fn delivery_stats(&self) -> Vec<SinkStatsSnapshot> {
        vec![self.stats()]
    }
}

/// Drain the queue, batching by count, size and time.
async fn export_loop(
    mut rx: mpsc::Receiver<Message>,
    transport: Arc<dyn Transport>,
    config: BatchConfig,
    stats: Arc<SinkStats>,
) {
    let mut batch: Vec<Event> = Vec::with_capacity(config.max_batch_events.min(1024));
    let mut batch_bytes = 0usize;
    let mut ticker = tokio::time::interval(config.flush_interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            message = rx.recv() => {
                match message {
                    Some(Message::Event(event)) => {
                        batch_bytes += transport.estimate_bytes(&event);
                        batch.push(*event);

                        if batch.len() >= config.max_batch_events
                            || batch_bytes >= config.max_batch_bytes
                        {
                            deliver(&transport, &mut batch, &config, &stats).await;
                            batch_bytes = 0;
                        }
                    },
                    Some(Message::Flush(ack)) => {
                        deliver(&transport, &mut batch, &config, &stats).await;
                        batch_bytes = 0;
                        // The receiver may have timed out; that is not an error.
                        let _ = ack.send(());
                    },
                    None => {
                        // Sink dropped: make a final delivery attempt so a
                        // clean shutdown does not discard buffered spans.
                        deliver(&transport, &mut batch, &config, &stats).await;
                        return;
                    },
                }
            },
            _ = ticker.tick() => {
                if !batch.is_empty() {
                    deliver(&transport, &mut batch, &config, &stats).await;
                    batch_bytes = 0;
                }
            },
        }
    }
}

/// Send `batch` with retries, then clear it regardless of outcome.
async fn deliver(
    transport: &Arc<dyn Transport>,
    batch: &mut Vec<Event>,
    config: &BatchConfig,
    stats: &Arc<SinkStats>,
) {
    if batch.is_empty() {
        return;
    }

    let count = batch.len() as u64;
    let mut attempt = 0u32;

    loop {
        match transport.send(batch).await {
            Ok(()) => {
                stats.batches_sent.fetch_add(1, Ordering::Relaxed);
                stats.record_success(count);
                trace!(sink = transport.name(), events = count, "batch delivered");
                break;
            },
            Err(TransportError::Fatal(reason)) => {
                // Retrying a 401 or a malformed payload only burns quota.
                warn!(
                    sink = transport.name(),
                    events = count,
                    %reason,
                    "dropping batch after fatal export failure"
                );
                stats.batches_failed.fetch_add(1, Ordering::Relaxed);
                stats.dropped_failed.fetch_add(count, Ordering::Relaxed);
                stats.record_error(reason);
                break;
            },
            Err(TransportError::Retryable(reason)) if attempt < config.max_retries => {
                let delay = backoff_delay(config, attempt);
                debug!(
                    sink = transport.name(),
                    attempt,
                    delay_ms = delay.as_millis(),
                    %reason,
                    "retrying observability export"
                );
                stats.retries.fetch_add(1, Ordering::Relaxed);
                stats.record_error(reason);
                tokio::time::sleep(delay).await;
                attempt += 1;
            },
            Err(TransportError::Retryable(reason)) => {
                warn!(
                    sink = transport.name(),
                    events = count,
                    attempts = attempt + 1,
                    %reason,
                    "dropping batch after exhausting export retries"
                );
                stats.batches_failed.fetch_add(1, Ordering::Relaxed);
                stats.dropped_failed.fetch_add(count, Ordering::Relaxed);
                stats.record_error(reason);
                break;
            },
        }
    }

    batch.clear();
}

/// Exponential backoff with full jitter, clamped to `max_backoff`.
///
/// Jitter matters here: without it, every Moltis instance that hits the same
/// rate limit retries in lockstep and re-creates the overload.
fn backoff_delay(config: &BatchConfig, attempt: u32) -> Duration {
    let exponent = attempt.min(16);
    let base = config
        .initial_backoff
        .saturating_mul(2u32.saturating_pow(exponent));
    let capped = base.min(config.max_backoff);
    let millis = capped.as_millis().min(u128::from(u64::MAX)) as u64;
    if millis == 0 {
        return Duration::ZERO;
    }
    let jittered = rand::random_range(0..=millis);
    Duration::from_millis(jittered)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::sync::Mutex;

    use {
        super::*,
        crate::model::{ObservationKind, ObservationRecord, TraceId},
    };

    #[derive(Default)]
    struct RecordingTransport {
        batches: Mutex<Vec<usize>>,
        /// Number of leading calls that fail retryably.
        fail_retryable: AtomicU64,
        /// Whether every call fails fatally.
        fail_fatal: bool,
    }

    #[async_trait]
    impl Transport for RecordingTransport {
        fn name(&self) -> &str {
            "recording"
        }

        async fn send(&self, batch: &[Event]) -> Result<(), TransportError> {
            if self.fail_fatal {
                return Err(TransportError::Fatal("unauthorized".into()));
            }
            if self.fail_retryable.load(Ordering::Relaxed) > 0 {
                self.fail_retryable.fetch_sub(1, Ordering::Relaxed);
                return Err(TransportError::Retryable("503".into()));
            }
            self.batches
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(batch.len());
            Ok(())
        }
    }

    impl RecordingTransport {
        fn batches(&self) -> Vec<usize> {
            self.batches
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }
    }

    fn event() -> Event {
        Event::ObservationEnd(Box::new(ObservationRecord::start(
            TraceId::generate(),
            ObservationKind::Generation,
            "llm-call",
        )))
    }

    fn fast_config() -> BatchConfig {
        BatchConfig {
            flush_interval: Duration::from_millis(50),
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(5),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn flush_delivers_pending_events() {
        let transport = Arc::new(RecordingTransport::default());
        let sink = BatchSink::spawn(transport.clone(), fast_config());

        sink.record(event());
        sink.record(event());
        sink.flush(Duration::from_secs(5))
            .await
            .expect("flush should succeed");

        assert_eq!(transport.batches(), vec![2]);
        assert_eq!(sink.stats().accepted, 2);
        assert_eq!(sink.stats().delivered, 2);
        assert!(sink.stats().last_success_at.is_some());
    }

    #[tokio::test]
    async fn batch_is_forced_once_event_count_reached() {
        let transport = Arc::new(RecordingTransport::default());
        let config = BatchConfig {
            max_batch_events: 3,
            ..fast_config()
        };
        let sink = BatchSink::spawn(transport.clone(), config);

        for _ in 0..3 {
            sink.record(event());
        }
        sink.flush(Duration::from_secs(5))
            .await
            .expect("flush should succeed");

        // The size trigger must fire on its own, before the flush barrier.
        assert_eq!(transport.batches().first(), Some(&3));
    }

    #[tokio::test]
    async fn batch_is_forced_once_byte_budget_reached() {
        let transport = Arc::new(RecordingTransport::default());
        let config = BatchConfig {
            max_batch_events: 10_000,
            max_batch_bytes: 1,
            ..fast_config()
        };
        let sink = BatchSink::spawn(transport.clone(), config);

        sink.record(event());
        sink.flush(Duration::from_secs(5))
            .await
            .expect("flush should succeed");

        assert_eq!(transport.batches().first(), Some(&1));
    }

    #[tokio::test]
    async fn retryable_failures_are_retried_then_succeed() {
        let transport = Arc::new(RecordingTransport::default());
        transport.fail_retryable.store(2, Ordering::Relaxed);
        let sink = BatchSink::spawn(transport.clone(), fast_config());

        sink.record(event());
        sink.flush(Duration::from_secs(5))
            .await
            .expect("flush should succeed");

        assert_eq!(transport.batches(), vec![1]);
        assert_eq!(sink.stats().batches_sent, 1);
        assert_eq!(sink.stats().dropped_failed, 0);
        assert_eq!(sink.stats().delivered, 1);
        assert_eq!(sink.stats().retries, 2);
        assert!(sink.stats().last_error.is_some());
        assert!(sink.stats().last_success_at.is_some());
    }

    #[tokio::test]
    async fn retries_are_bounded_and_batch_is_dropped() {
        let transport = Arc::new(RecordingTransport::default());
        transport.fail_retryable.store(100, Ordering::Relaxed);
        let config = BatchConfig {
            max_retries: 2,
            ..fast_config()
        };
        let sink = BatchSink::spawn(transport.clone(), config);

        sink.record(event());
        sink.flush(Duration::from_secs(5))
            .await
            .expect("flush should still complete");

        assert!(transport.batches().is_empty());
        assert_eq!(sink.stats().dropped_failed, 1);
        assert_eq!(sink.stats().batches_failed, 1);
        assert_eq!(sink.stats().retries, 2);
        assert_eq!(sink.stats().delivered, 0);
        assert!(sink.stats().last_error_at.is_some());
    }

    #[tokio::test]
    async fn fatal_failures_are_not_retried() {
        let transport = Arc::new(RecordingTransport {
            fail_fatal: true,
            ..Default::default()
        });
        let sink = BatchSink::spawn(transport.clone(), fast_config());

        sink.record(event());
        sink.flush(Duration::from_secs(5))
            .await
            .expect("flush should still complete");

        assert_eq!(sink.stats().batches_failed, 1);
        assert_eq!(sink.stats().dropped_failed, 1);
        assert_eq!(sink.stats().retries, 0);
        assert!(
            sink.stats()
                .last_error
                .is_some_and(|error| error.contains("unauthorized"))
        );
    }

    #[tokio::test]
    async fn full_queue_drops_events_without_blocking_caller() {
        let transport = Arc::new(RecordingTransport::default());
        let config = BatchConfig {
            queue_capacity: 4,
            max_batch_events: 100_000,
            // Long interval so the drain never runs during the test.
            flush_interval: Duration::from_secs(3600),
            ..fast_config()
        };
        let sink = BatchSink::spawn(transport, config);

        // Far more events than the queue can hold. `record` is synchronous, so
        // if it ever blocked this test would hang rather than fail.
        for _ in 0..500 {
            sink.record(event());
        }

        let stats = sink.stats();
        assert!(
            stats.dropped_queue_full > 0,
            "expected drops on a full queue"
        );
        assert!(
            stats.accepted < 500,
            "queue must not have absorbed every event"
        );
        assert_eq!(stats.accepted + stats.dropped_queue_full, 500);
    }

    #[tokio::test]
    async fn time_based_flush_delivers_without_an_explicit_barrier() {
        let transport = Arc::new(RecordingTransport::default());
        let config = BatchConfig {
            max_batch_events: 10_000,
            flush_interval: Duration::from_millis(20),
            ..fast_config()
        };
        let sink = BatchSink::spawn(transport.clone(), config);

        sink.record(event());
        // Deliberately no flush() call: the ticker must do the work.
        for _ in 0..50 {
            if !transport.batches().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        assert_eq!(transport.batches(), vec![1]);
    }

    #[test]
    fn backoff_is_capped_and_jittered_within_bounds() {
        let config = BatchConfig {
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_millis(1000),
            ..Default::default()
        };

        for attempt in 0..8 {
            let delay = backoff_delay(&config, attempt);
            assert!(
                delay <= config.max_backoff,
                "attempt {attempt} exceeded the cap"
            );
        }
    }

    #[test]
    fn backoff_does_not_overflow_on_large_attempt_counts() {
        let config = BatchConfig::default();
        // A pathological attempt count must clamp, not panic on overflow.
        assert!(backoff_delay(&config, u32::MAX) <= config.max_backoff);
    }
}
