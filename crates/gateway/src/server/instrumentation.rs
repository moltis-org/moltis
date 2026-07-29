//! Startup wiring for agent instrumentation.
//!
//! Builds every configured backend and installs the resulting fanout as the
//! process-wide sink, which the agent runner then discovers without any
//! plumbing through its call signatures.

use std::{
    sync::{Arc, RwLock},
    time::Duration,
};

use {
    moltis_config::InstrumentationConfig,
    moltis_observability::{ObservationSink, SinkStatsSnapshot, SkippedBackend},
    tracing::{info, warn},
};

#[cfg(feature = "langfuse")]
use moltis_observability::exporters::langfuse::LangfuseClient;

/// Live instrumentation state, surfaced by the `instrumentation.*` RPC methods.
#[derive(Default)]
pub struct InstrumentationState {
    inner: RwLock<InstrumentationSnapshot>,
}

/// The latest applied outcome, including failures when no sink could start.
#[derive(Default)]
struct InstrumentationSnapshot {
    sink: Option<Arc<dyn ObservationSink>>,
    backends: Vec<String>,
    skipped: Vec<SkippedBackend>,
    scores: bool,
    #[cfg(feature = "langfuse")]
    langfuse: Option<Arc<LangfuseClient>>,
}

/// Status reported to the settings UI.
#[derive(Debug, Clone, serde::Serialize)]
pub struct InstrumentationStatus {
    /// Whether any backend is actively exporting.
    pub active: bool,
    /// Backends that are running.
    pub backends: Vec<String>,
    /// Backends that were enabled in config but could not start, with reasons.
    /// Surfaced rather than only logged: a silently disabled exporter is
    /// indistinguishable from a broken one.
    pub skipped: Vec<SkippedBackend>,
    /// Live delivery health for each exporter path.
    pub delivery: Vec<SinkStatsSnapshot>,
}

impl InstrumentationState {
    /// Build from config and install the sink. Replaces any previous setup and
    /// flushes the replaced sink in the background; this synchronous method
    /// never waits on I/O.
    pub fn apply(&self, config: &InstrumentationConfig, release: &str) -> InstrumentationStatus {
        let outcome = moltis_observability::build(config, release);
        let snapshot = match outcome.built {
            Some(built) => {
                moltis_observability::set_global_sink(Arc::clone(&built.sink));
                info!(backends = ?built.backends, "agent instrumentation active");
                InstrumentationSnapshot {
                    sink: Some(built.sink),
                    backends: built.backends,
                    skipped: outcome.skipped,
                    scores: built.scores,
                    #[cfg(feature = "langfuse")]
                    langfuse: built.langfuse,
                }
            },
            None => {
                // Tear down rather than leaving a stale sink installed, so
                // disabling instrumentation in the UI takes effect immediately.
                moltis_observability::clear_global_sink();
                if !outcome.skipped.is_empty() {
                    warn!(
                        skipped = ?outcome.skipped,
                        "instrumentation configured but no backend could start"
                    );
                }
                InstrumentationSnapshot {
                    skipped: outcome.skipped,
                    ..Default::default()
                }
            },
        };

        let status = snapshot.status();
        if let Some(old_sink) = self.replace(snapshot) {
            tokio::spawn(async move {
                if let Err(error) = old_sink.flush(Duration::from_secs(5)).await {
                    warn!(%error, "replaced instrumentation sink flush failed");
                }
            });
        }

        status
    }

    /// Current status.
    #[must_use]
    pub fn status(&self) -> InstrumentationStatus {
        let guard = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.status()
    }

    /// Whether a running backend can store scores.
    ///
    /// The feedback surfaces ask this instead of naming a backend: a thumb that
    /// nothing records is worse than no thumb at all.
    #[must_use]
    pub fn scores_available(&self) -> bool {
        let guard = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.scores
    }

    /// The Langfuse client, when that backend is running.
    #[cfg(feature = "langfuse")]
    #[must_use]
    pub fn langfuse(&self) -> Option<Arc<LangfuseClient>> {
        let guard = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.langfuse.clone()
    }

    /// Flush every backend, for a clean shutdown.
    pub async fn flush(&self, timeout: Duration) {
        let sink = {
            let guard = self
                .inner
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.sink.clone()
        };
        let Some(sink) = sink else {
            return;
        };
        if let Err(error) = sink.flush(timeout).await {
            warn!(%error, "instrumentation flush failed during shutdown");
        }
    }

    /// Stop accepting new events and flush the currently installed sink.
    pub async fn shutdown(&self, timeout: Duration) {
        moltis_observability::clear_global_sink();
        let started = tokio::time::Instant::now();
        let flush_reserve = timeout.min(Duration::from_millis(100));
        let wait_budget = timeout.saturating_sub(flush_reserve);
        if !moltis_observability::wait_for_active_turns(wait_budget).await {
            warn!("instrumentation shutdown timed out waiting for active turns");
        }
        self.flush(timeout.saturating_sub(started.elapsed())).await;
    }

    fn replace(&self, value: InstrumentationSnapshot) -> Option<Arc<dyn ObservationSink>> {
        match self.inner.write() {
            Ok(mut guard) => std::mem::replace(&mut *guard, value).sink,
            Err(poisoned) => std::mem::replace(&mut *poisoned.into_inner(), value).sink,
        }
    }
}

impl InstrumentationSnapshot {
    fn status(&self) -> InstrumentationStatus {
        InstrumentationStatus {
            active: self.sink.is_some(),
            backends: self.backends.clone(),
            skipped: self.skipped.clone(),
            delivery: self
                .sink
                .as_ref()
                .map_or_else(Vec::new, |sink| sink.delivery_stats()),
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use {
        async_trait::async_trait,
        moltis_config::LangfuseSettings,
        secrecy::Secret,
        std::sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        tokio::sync::oneshot,
    };

    use super::*;

    #[cfg(feature = "langfuse")]
    fn valid_langfuse_config() -> InstrumentationConfig {
        InstrumentationConfig {
            enabled: true,
            langfuse: LangfuseSettings {
                enabled: true,
                host: "https://cloud.langfuse.com".into(),
                public_key: "pk-lf-1".into(),
                secret_key: Some(Secret::new("sk-lf-1".to_string())),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    struct FlushTrackingSink {
        flushed: Mutex<Option<oneshot::Sender<()>>>,
    }

    struct TimeoutAwareFlushSink {
        delivered: Mutex<Option<oneshot::Sender<usize>>>,
        queued: AtomicUsize,
    }

    #[async_trait]
    impl ObservationSink for FlushTrackingSink {
        fn name(&self) -> &str {
            "tracking"
        }

        fn record(&self, _event: moltis_observability::Event) {}

        async fn flush(&self, _timeout: Duration) -> anyhow::Result<()> {
            if let Some(flushed) = self
                .flushed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                let _ = flushed.send(());
            }
            Ok(())
        }
    }

    #[async_trait]
    impl ObservationSink for TimeoutAwareFlushSink {
        fn name(&self) -> &str {
            "timeout-aware"
        }

        fn record(&self, _event: moltis_observability::Event) {
            self.queued.fetch_add(1, Ordering::Relaxed);
        }

        async fn flush(&self, timeout: Duration) -> anyhow::Result<()> {
            anyhow::ensure!(
                timeout >= Duration::from_millis(25),
                "insufficient flush budget"
            );
            let queued = self.queued.swap(0, Ordering::Relaxed);
            if let Some(delivered) = self
                .delivered
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                let _ = delivered.send(queued);
            }
            Ok(())
        }
    }

    #[tokio::test]
    #[serial_test::serial(instrumentation_global_sink)]
    async fn disabled_config_reports_inactive_and_installs_no_sink() {
        let state = InstrumentationState::default();
        let status = state.apply(&InstrumentationConfig::default(), "test");

        assert!(!status.active);
        assert!(status.backends.is_empty());
        assert!(!moltis_observability::is_enabled());

        moltis_observability::clear_global_sink();
    }

    #[cfg(feature = "langfuse")]
    #[tokio::test]
    #[serial_test::serial(instrumentation_global_sink)]
    async fn applying_a_valid_config_installs_the_sink() {
        let state = InstrumentationState::default();
        let status = state.apply(&valid_langfuse_config(), "20260726.01");

        assert!(status.active);
        assert_eq!(status.backends, vec!["langfuse"]);
        assert_eq!(
            status
                .delivery
                .iter()
                .map(|stats| stats.name.as_str())
                .collect::<Vec<_>>(),
            vec!["langfuse", "langfuse-scores"]
        );
        assert!(status.delivery.iter().all(|stats| stats.accepted == 0));
        assert!(moltis_observability::is_enabled());
        assert!(state.langfuse().is_some());

        moltis_observability::clear_global_sink();
    }

    #[cfg(feature = "langfuse")]
    #[tokio::test]
    #[serial_test::serial(instrumentation_global_sink)]
    async fn reapplying_a_disabled_config_tears_the_sink_down() {
        let state = InstrumentationState::default();
        state.apply(&valid_langfuse_config(), "test");
        assert!(moltis_observability::is_enabled());

        // Turning instrumentation off in the UI must take effect immediately
        // rather than leaving the previous sink exporting.
        let status = state.apply(&InstrumentationConfig::default(), "test");

        assert!(!status.active);
        assert!(!moltis_observability::is_enabled());
        assert!(state.langfuse().is_none());
    }

    #[cfg(feature = "langfuse")]
    #[tokio::test]
    #[serial_test::serial(instrumentation_global_sink)]
    async fn skipped_backends_are_reported_not_just_logged() {
        let mut config = valid_langfuse_config();
        config.langfuse.public_key = String::new();

        let state = InstrumentationState::default();
        let status = state.apply(&config, "test");

        assert!(!status.active);
        assert_eq!(status.skipped.len(), 1);
        assert_eq!(status.skipped[0].name, "langfuse");
        assert!(status.skipped[0].reason.contains("public_key"));

        let current = state.status();
        assert_eq!(current.skipped, status.skipped);

        moltis_observability::clear_global_sink();
    }

    #[cfg(not(feature = "langfuse"))]
    #[tokio::test]
    #[serial_test::serial(instrumentation_global_sink)]
    async fn a_backend_this_build_lacks_is_reported_as_skipped() {
        // Without the exporter compiled in, enabling it in config must explain
        // itself rather than looking like a credential mistake.
        let config = InstrumentationConfig {
            enabled: true,
            langfuse: LangfuseSettings {
                enabled: true,
                host: "https://cloud.langfuse.com".into(),
                public_key: "pk-lf-1".into(),
                secret_key: Some(Secret::new("sk-lf-1".to_string())),
                ..Default::default()
            },
            ..Default::default()
        };

        let state = InstrumentationState::default();
        let status = state.apply(&config, "test");

        assert!(!status.active);
        assert!(!state.scores_available());
        assert_eq!(status.skipped.len(), 1);
        assert_eq!(status.skipped[0].name, "langfuse");
        assert_eq!(status.skipped[0].reason, "not compiled into this build");
    }

    #[tokio::test]
    async fn status_before_any_apply_is_inactive() {
        let state = InstrumentationState::default();
        let status = state.status();

        assert!(!status.active);
        assert!(status.backends.is_empty());
    }

    #[tokio::test]
    #[serial_test::serial(instrumentation_global_sink)]
    async fn flush_without_a_sink_is_a_no_op() {
        moltis_observability::clear_global_sink();
        let state = InstrumentationState::default();
        state.flush(Duration::from_millis(10)).await;
    }

    #[tokio::test]
    #[serial_test::serial(instrumentation_global_sink)]
    async fn reconfiguration_flushes_the_replaced_sink_without_blocking_apply() {
        let (flushed_tx, flushed_rx) = oneshot::channel();
        let old_sink: Arc<dyn ObservationSink> = Arc::new(FlushTrackingSink {
            flushed: Mutex::new(Some(flushed_tx)),
        });
        let state = InstrumentationState::default();
        moltis_observability::set_global_sink(Arc::clone(&old_sink));
        state.replace(InstrumentationSnapshot {
            sink: Some(old_sink),
            backends: vec!["tracking".into()],
            ..Default::default()
        });

        let status = state.apply(&InstrumentationConfig::default(), "test");

        assert!(!status.active);
        tokio::time::timeout(Duration::from_secs(1), flushed_rx)
            .await
            .expect("replaced sink flush should be scheduled")
            .expect("flush notification should be sent");
        assert!(!moltis_observability::is_enabled());
    }

    #[tokio::test]
    #[serial_test::serial(instrumentation_global_sink)]
    async fn shutdown_stops_new_events_and_flushes_the_active_sink() {
        let (flushed_tx, flushed_rx) = oneshot::channel();
        let sink: Arc<dyn ObservationSink> = Arc::new(FlushTrackingSink {
            flushed: Mutex::new(Some(flushed_tx)),
        });
        let state = InstrumentationState::default();
        moltis_observability::set_global_sink(Arc::clone(&sink));
        state.replace(InstrumentationSnapshot {
            sink: Some(sink),
            backends: vec!["tracking".into()],
            ..Default::default()
        });

        state.shutdown(Duration::from_secs(1)).await;

        flushed_rx
            .await
            .expect("active sink should be flushed during shutdown");
        assert!(!moltis_observability::is_enabled());
    }

    #[tokio::test]
    #[serial_test::serial(instrumentation_global_sink)]
    async fn shutdown_waits_for_active_turns_before_flushing() {
        let (flushed_tx, flushed_rx) = oneshot::channel();
        let sink: Arc<dyn ObservationSink> = Arc::new(FlushTrackingSink {
            flushed: Mutex::new(Some(flushed_tx)),
        });
        let state = InstrumentationState::default();
        moltis_observability::set_global_sink(Arc::clone(&sink));
        state.replace(InstrumentationSnapshot {
            sink: Some(sink),
            backends: vec!["tracking".into()],
            ..Default::default()
        });
        let recorder = moltis_observability::TurnRecorder::begin(
            "active",
            moltis_observability::TraceScope::default(),
            moltis_observability::RecorderSettings::default(),
        )
        .expect("global sink installed");

        let shutdown = state.shutdown(Duration::from_secs(1));
        tokio::pin!(shutdown);
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut shutdown)
                .await
                .is_err(),
            "shutdown must wait for the active recorder"
        );
        drop(recorder);
        shutdown.await;

        flushed_rx
            .await
            .expect("sink should flush after the turn closes");
    }

    #[tokio::test]
    #[serial_test::serial(instrumentation_global_sink)]
    async fn shutdown_flushes_queued_data_when_active_turns_time_out() {
        let (delivered_tx, delivered_rx) = oneshot::channel();
        let sink = Arc::new(TimeoutAwareFlushSink {
            delivered: Mutex::new(Some(delivered_tx)),
            queued: AtomicUsize::new(0),
        });
        let installed_sink: Arc<dyn ObservationSink> = sink;
        let state = InstrumentationState::default();
        moltis_observability::set_global_sink(Arc::clone(&installed_sink));
        state.replace(InstrumentationSnapshot {
            sink: Some(installed_sink),
            backends: vec!["tracking".into()],
            ..Default::default()
        });
        let completed = moltis_observability::TurnRecorder::begin(
            "completed",
            moltis_observability::TraceScope::default(),
            moltis_observability::RecorderSettings::default(),
        )
        .expect("global sink installed");
        completed.finish();
        let recorder = moltis_observability::TurnRecorder::begin(
            "active",
            moltis_observability::TraceScope::default(),
            moltis_observability::RecorderSettings::default(),
        )
        .expect("global sink installed");

        state.shutdown(Duration::from_millis(100)).await;

        let delivered = tokio::time::timeout(Duration::from_secs(1), delivered_rx)
            .await
            .expect("shutdown should retain enough time to flush")
            .expect("queued data should flush despite the active turn");
        assert!(delivered > 0, "the completed turn should reach the sink");
        drop(recorder);
    }
}
