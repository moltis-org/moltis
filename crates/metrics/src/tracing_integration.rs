//! Tracing integration for metrics.
//!
//! This module provides integration between the `tracing` and `metrics` crates,
//! allowing metrics to be automatically labeled with span context.

#[cfg(feature = "tracing")]
use metrics_tracing_context::MetricsLayer;

#[cfg(feature = "tracing")]
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Initialize tracing with metrics context integration.
///
/// This sets up a tracing subscriber that propagates span labels to metrics.
/// Call this before initializing the metrics recorder for full integration.
///
/// # Example
///
/// ```rust,ignore
/// use moltis_metrics::tracing_integration::init_tracing;
///
/// init_tracing();
/// // Spans will now propagate labels to metrics
/// ```
#[cfg(feature = "tracing")]
pub fn init_tracing() {
    let metrics_layer = MetricsLayer::new();

    tracing_subscriber::registry()
        .with(metrics_layer)
        .with(tracing_subscriber::fmt::layer())
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();
}

/// Labels that are propagated from tracing spans to metrics.
///
/// When using the tracing integration, these span fields will
/// automatically be added as metric labels.
pub mod span_labels {
    /// The span name/target
    pub const SPAN_NAME: &str = "span.name";
    /// The operation being performed
    pub const OPERATION: &str = "operation";
    /// The component/module
    pub const COMPONENT: &str = "component";
}
