//! Metrics recorder initialization and configuration.

use {anyhow::Result, tracing::info};

/// Handle to the metrics system, providing access to exported metrics.
#[derive(Clone)]
pub struct MetricsHandle {
    #[cfg(feature = "prometheus")]
    prometheus_handle: metrics_exporter_prometheus::PrometheusHandle,
}

impl MetricsHandle {
    /// Render metrics in Prometheus text format.
    ///
    /// Returns the metrics as a string suitable for the `/metrics` endpoint.
    #[must_use]
    pub fn render(&self) -> String {
        #[cfg(feature = "prometheus")]
        {
            self.prometheus_handle.render()
        }
        #[cfg(not(feature = "prometheus"))]
        {
            String::new()
        }
    }
}

/// Configuration for the metrics system.
#[derive(Debug, Clone, Default)]
pub struct MetricsRecorderConfig {
    /// Whether metrics collection is enabled
    pub enabled: bool,
    /// Prefix for all metric names (default: "moltis")
    pub prefix: Option<String>,
    /// Global labels to add to all metrics
    pub global_labels: Vec<(String, String)>,
}

/// Initialize the metrics system.
///
/// This should be called once at application startup. When the `prometheus` feature
/// is enabled, this sets up the Prometheus exporter. Otherwise, it uses a no-op
/// recorder that discards all metrics.
///
/// # Returns
///
/// A `MetricsHandle` that can be used to render metrics for the `/metrics` endpoint.
///
/// # Errors
///
/// Returns an error if the metrics system fails to initialize.
pub fn init_metrics(config: MetricsRecorderConfig) -> Result<MetricsHandle> {
    if !config.enabled {
        info!("Metrics collection is disabled");
        return Ok(MetricsHandle {
            #[cfg(feature = "prometheus")]
            prometheus_handle: init_prometheus_disabled()?,
        });
    }

    #[cfg(feature = "prometheus")]
    {
        let handle = init_prometheus(config)?;
        info!("Prometheus metrics exporter initialized");
        Ok(MetricsHandle {
            prometheus_handle: handle,
        })
    }

    #[cfg(not(feature = "prometheus"))]
    {
        info!("Metrics feature not enabled at compile time");
        Ok(MetricsHandle {})
    }
}

#[cfg(feature = "prometheus")]
fn init_prometheus(
    config: MetricsRecorderConfig,
) -> Result<metrics_exporter_prometheus::PrometheusHandle> {
    use {crate::buckets, metrics_exporter_prometheus::PrometheusBuilder};

    let mut builder = PrometheusBuilder::new();

    // Set histogram buckets for specific metrics
    builder = builder
        // HTTP request durations
        .set_buckets_for_metric(
            metrics_exporter_prometheus::Matcher::Suffix("_duration_seconds".to_string()),
            &buckets::HTTP_DURATION,
        )?
        // LLM-specific durations (longer tail)
        .set_buckets_for_metric(
            metrics_exporter_prometheus::Matcher::Prefix("moltis_llm_completion".to_string()),
            &buckets::LLM_DURATION,
        )?
        // Time to first token
        .set_buckets_for_metric(
            metrics_exporter_prometheus::Matcher::Full(
                crate::llm::TIME_TO_FIRST_TOKEN_SECONDS.to_string(),
            ),
            &buckets::TTFT,
        )?
        // Tokens per second
        .set_buckets_for_metric(
            metrics_exporter_prometheus::Matcher::Full(crate::llm::TOKENS_PER_SECOND.to_string()),
            &buckets::TOKENS_PER_SECOND,
        )?;

    // Add global labels
    for (key, value) in config.global_labels {
        builder = builder.add_global_label(key, value);
    }

    // Build and install the recorder, returning a handle for rendering metrics.
    // We use install_recorder() which installs the recorder globally and returns
    // a handle that can be used to render metrics (without spawning an HTTP server).
    let handle = builder.install_recorder()?;

    Ok(handle)
}

#[cfg(feature = "prometheus")]
fn init_prometheus_disabled() -> Result<metrics_exporter_prometheus::PrometheusHandle> {
    // Create a minimal prometheus handle that returns empty metrics
    let handle = metrics_exporter_prometheus::PrometheusBuilder::new().install_recorder()?;
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_disabled() {
        let config = MetricsRecorderConfig {
            enabled: false,
            ..Default::default()
        };
        let handle = init_metrics(config).unwrap();
        // Should return empty or minimal output when disabled
        let output = handle.render();
        // Prometheus always includes some metadata even when empty
        assert!(output.is_empty() || output.contains('#'));
    }
}
