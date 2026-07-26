//! Startup wiring for agent instrumentation.
//!
//! Builds every configured backend and installs the resulting fanout as the
//! process-wide sink, which the agent runner then discovers without any
//! plumbing through its call signatures.

use std::sync::{Arc, RwLock};

use {
    moltis_config::InstrumentationConfig,
    moltis_observability::{
        BuiltInstrumentation, SkippedBackend, exporters::langfuse::LangfuseClient,
    },
    tracing::{info, warn},
};

/// Live instrumentation state, surfaced by the `instrumentation.*` RPC methods.
#[derive(Default)]
pub struct InstrumentationState {
    inner: RwLock<Option<ActiveInstrumentation>>,
}

/// What is currently running.
struct ActiveInstrumentation {
    backends: Vec<String>,
    skipped: Vec<SkippedBackend>,
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
}

impl InstrumentationState {
    /// Build from config and install the sink. Replaces any previous setup, so
    /// the settings UI can reconfigure without a restart.
    pub fn apply(&self, config: &InstrumentationConfig, release: &str) -> InstrumentationStatus {
        let outcome = moltis_observability::build(config, release);

        let status = match &outcome.built {
            Some(built) => InstrumentationStatus {
                active: true,
                backends: built.backends.clone(),
                skipped: outcome.skipped.clone(),
            },
            None => InstrumentationStatus {
                active: false,
                backends: Vec::new(),
                skipped: outcome.skipped.clone(),
            },
        };

        match outcome.built {
            Some(BuiltInstrumentation {
                sink,
                langfuse,
                backends,
                ..
            }) => {
                moltis_observability::set_global_sink(sink);
                info!(backends = ?backends, "agent instrumentation active");
                self.store(Some(ActiveInstrumentation {
                    backends,
                    skipped: outcome.skipped,
                    langfuse,
                }));
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
                self.store(None);
            },
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
        guard.as_ref().map_or_else(
            || InstrumentationStatus {
                active: false,
                backends: Vec::new(),
                skipped: Vec::new(),
            },
            |active| InstrumentationStatus {
                active: true,
                backends: active.backends.clone(),
                skipped: active.skipped.clone(),
            },
        )
    }

    /// The Langfuse client, when that backend is running.
    #[must_use]
    pub fn langfuse(&self) -> Option<Arc<LangfuseClient>> {
        let guard = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.as_ref().and_then(|a| a.langfuse.clone())
    }

    /// Flush every backend, for a clean shutdown.
    pub async fn flush(&self, timeout: std::time::Duration) {
        let Some(sink) = moltis_observability::global_sink() else {
            return;
        };
        if let Err(error) = sink.flush(timeout).await {
            warn!(%error, "instrumentation flush failed during shutdown");
        }
    }

    fn store(&self, value: Option<ActiveInstrumentation>) {
        match self.inner.write() {
            Ok(mut guard) => *guard = value,
            Err(poisoned) => *poisoned.into_inner() = value,
        }
    }
}

#[cfg(test)]
mod tests {
    use {moltis_config::LangfuseSettings, secrecy::Secret};

    use super::*;

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

    #[tokio::test]
    async fn disabled_config_reports_inactive_and_installs_no_sink() {
        let state = InstrumentationState::default();
        let status = state.apply(&InstrumentationConfig::default(), "test");

        assert!(!status.active);
        assert!(status.backends.is_empty());
        assert!(!moltis_observability::is_enabled());

        moltis_observability::clear_global_sink();
    }

    #[tokio::test]
    async fn applying_a_valid_config_installs_the_sink() {
        let state = InstrumentationState::default();
        let status = state.apply(&valid_langfuse_config(), "20260726.01");

        assert!(status.active);
        assert_eq!(status.backends, vec!["langfuse"]);
        assert!(moltis_observability::is_enabled());
        assert!(state.langfuse().is_some());

        moltis_observability::clear_global_sink();
    }

    #[tokio::test]
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

    #[tokio::test]
    async fn skipped_backends_are_reported_not_just_logged() {
        let mut config = valid_langfuse_config();
        config.langfuse.public_key = String::new();

        let state = InstrumentationState::default();
        let status = state.apply(&config, "test");

        assert!(!status.active);
        assert_eq!(status.skipped.len(), 1);
        assert_eq!(status.skipped[0].name, "langfuse");
        assert!(status.skipped[0].reason.contains("public_key"));

        moltis_observability::clear_global_sink();
    }

    #[tokio::test]
    async fn status_before_any_apply_is_inactive() {
        let state = InstrumentationState::default();
        let status = state.status();

        assert!(!status.active);
        assert!(status.backends.is_empty());
    }

    #[tokio::test]
    async fn flush_without_a_sink_is_a_no_op() {
        moltis_observability::clear_global_sink();
        let state = InstrumentationState::default();
        state.flush(std::time::Duration::from_millis(10)).await;
    }
}
