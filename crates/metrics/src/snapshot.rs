//! Metrics snapshot for internal API consumption.
//!
//! This module provides a way to get metrics data as structured JSON
//! for display in the web UI, separate from the Prometheus text format.

use {
    serde::{Deserialize, Serialize},
    std::collections::HashMap,
};

/// Type of metric
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MetricType {
    Counter,
    Gauge,
    Histogram,
}

/// A single metric value with its labels
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSnapshot {
    /// The metric name
    pub name: String,
    /// The metric type
    #[serde(rename = "type")]
    pub metric_type: MetricType,
    /// Labels attached to this metric
    pub labels: HashMap<String, String>,
    /// The current value (for counters and gauges)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    /// Histogram data (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub histogram: Option<HistogramSnapshot>,
    /// Human-readable description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Histogram bucket and summary data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistogramSnapshot {
    /// Total count of observations
    pub count: u64,
    /// Sum of all observed values
    pub sum: f64,
    /// Bucket boundaries and their cumulative counts
    pub buckets: Vec<HistogramBucket>,
    /// Calculated percentiles
    pub percentiles: PercentilesSnapshot,
}

/// A single histogram bucket
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistogramBucket {
    /// Upper bound of this bucket (exclusive, except +Inf)
    pub le: f64,
    /// Cumulative count of observations <= le
    pub count: u64,
}

/// Pre-calculated percentiles for histograms
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PercentilesSnapshot {
    pub p50: f64,
    pub p90: f64,
    pub p95: f64,
    pub p99: f64,
}

/// A complete snapshot of all metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    /// Timestamp when the snapshot was taken (Unix millis)
    pub timestamp: u64,
    /// All metric values
    pub metrics: Vec<MetricSnapshot>,
    /// Metrics grouped by category
    pub categories: MetricCategories,
}

/// Metrics organized by category for easier UI consumption
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MetricCategories {
    pub http: CategoryMetrics,
    pub websocket: CategoryMetrics,
    pub llm: LlmCategoryMetrics,
    pub session: CategoryMetrics,
    pub tools: CategoryMetrics,
    pub mcp: CategoryMetrics,
    pub memory: CategoryMetrics,
    pub system: SystemMetrics,
}

/// Generic category metrics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CategoryMetrics {
    /// Total requests/operations
    pub total: u64,
    /// Error count
    pub errors: u64,
    /// Currently active/in-flight
    pub active: u64,
    /// Average duration in seconds
    pub avg_duration_seconds: Option<f64>,
    /// P99 duration in seconds
    pub p99_duration_seconds: Option<f64>,
}

/// LLM-specific metrics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmCategoryMetrics {
    /// Total completions requested
    pub completions_total: u64,
    /// Total errors
    pub errors: u64,
    /// Total input tokens
    pub input_tokens: u64,
    /// Total output tokens
    pub output_tokens: u64,
    /// Total cache read tokens
    pub cache_read_tokens: u64,
    /// Total cache write tokens
    pub cache_write_tokens: u64,
    /// Average completion duration
    pub avg_duration_seconds: Option<f64>,
    /// Average time to first token
    pub avg_ttft_seconds: Option<f64>,
    /// Average tokens per second
    pub avg_tokens_per_second: Option<f64>,
    /// Breakdown by provider
    pub by_provider: HashMap<String, ProviderMetrics>,
    /// Breakdown by model
    pub by_model: HashMap<String, ModelMetrics>,
}

/// Per-provider metrics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderMetrics {
    pub completions: u64,
    pub errors: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Per-model metrics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelMetrics {
    pub completions: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub avg_duration_seconds: Option<f64>,
}

/// System-level metrics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SystemMetrics {
    /// Uptime in seconds
    pub uptime_seconds: f64,
    /// Number of connected clients
    pub connected_clients: u64,
    /// Active sessions
    pub active_sessions: u64,
    /// Build version
    pub version: Option<String>,
}

impl MetricsSnapshot {
    /// Create a new empty snapshot
    #[must_use]
    pub fn new() -> Self {
        Self {
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            metrics: Vec::new(),
            categories: MetricCategories::default(),
        }
    }

    /// Parse Prometheus text format into a structured snapshot.
    ///
    /// This is a best-effort parser that extracts metric values from
    /// Prometheus exposition format.
    #[must_use]
    pub fn from_prometheus_text(text: &str) -> Self {
        let mut snapshot = Self::new();

        for line in text.lines() {
            let line = line.trim();

            // Skip comments and empty lines
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Parse metric line: name{labels} value
            if let Some(metric) = parse_prometheus_line(line) {
                // Update category aggregates
                update_categories(&mut snapshot.categories, &metric);
                snapshot.metrics.push(metric);
            }
        }

        snapshot
    }
}

impl Default for MetricsSnapshot {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse a single Prometheus metric line
fn parse_prometheus_line(line: &str) -> Option<MetricSnapshot> {
    // Format: metric_name{label1="value1",label2="value2"} value
    // or: metric_name value

    let (name_and_labels, value_str) = line.rsplit_once(' ')?;
    let value: f64 = value_str.parse().ok()?;

    let (name, labels) = if let Some(brace_start) = name_and_labels.find('{') {
        let name = &name_and_labels[..brace_start];
        let labels_str = name_and_labels
            .get(brace_start + 1..name_and_labels.len() - 1)
            .unwrap_or("");
        let labels = parse_labels(labels_str);
        (name, labels)
    } else {
        (name_and_labels, HashMap::new())
    };

    // Determine metric type from name suffix
    let metric_type = if name.ends_with("_total") || name.ends_with("_count") {
        MetricType::Counter
    } else if name.ends_with("_bucket") || name.ends_with("_sum") {
        // These are histogram components, skip for now
        return None;
    } else {
        MetricType::Gauge
    };

    Some(MetricSnapshot {
        name: name.to_string(),
        metric_type,
        labels,
        value: Some(value),
        histogram: None,
        description: None,
    })
}

/// Parse Prometheus label format: key1="value1",key2="value2"
fn parse_labels(labels_str: &str) -> HashMap<String, String> {
    let mut labels = HashMap::new();

    if labels_str.is_empty() {
        return labels;
    }

    // Simple parser - doesn't handle escaped quotes in values
    for part in labels_str.split(',') {
        if let Some((key, value)) = part.split_once('=') {
            let value = value.trim_matches('"');
            labels.insert(key.to_string(), value.to_string());
        }
    }

    labels
}

/// Update category aggregates based on a metric
fn update_categories(categories: &mut MetricCategories, metric: &MetricSnapshot) {
    let name = &metric.name;
    let value = metric.value.unwrap_or(0.0) as u64;

    // HTTP metrics
    if name.starts_with("moltis_http_requests_total") {
        categories.http.total += value;
    } else if name.starts_with("moltis_http_requests_in_flight") {
        categories.http.active = value;
    }
    // WebSocket metrics
    else if name.starts_with("moltis_websocket_connections_total") {
        categories.websocket.total += value;
    } else if name.starts_with("moltis_websocket_connections_active") {
        categories.websocket.active = value;
    }
    // LLM metrics
    else if name.starts_with("moltis_llm_completions_total") {
        categories.llm.completions_total += value;

        // Track by provider and model
        if let Some(provider) = metric.labels.get("provider") {
            let entry = categories
                .llm
                .by_provider
                .entry(provider.clone())
                .or_default();
            entry.completions += value;
        }
        if let Some(model) = metric.labels.get("model") {
            let entry = categories.llm.by_model.entry(model.clone()).or_default();
            entry.completions += value;
        }
    } else if name.starts_with("moltis_llm_completion_errors_total") {
        categories.llm.errors += value;
    } else if name.starts_with("moltis_llm_input_tokens_total") {
        categories.llm.input_tokens += value;
        if let Some(provider) = metric.labels.get("provider") {
            let entry = categories
                .llm
                .by_provider
                .entry(provider.clone())
                .or_default();
            entry.input_tokens += value;
        }
        if let Some(model) = metric.labels.get("model") {
            let entry = categories.llm.by_model.entry(model.clone()).or_default();
            entry.input_tokens += value;
        }
    } else if name.starts_with("moltis_llm_output_tokens_total") {
        categories.llm.output_tokens += value;
        if let Some(provider) = metric.labels.get("provider") {
            let entry = categories
                .llm
                .by_provider
                .entry(provider.clone())
                .or_default();
            entry.output_tokens += value;
        }
        if let Some(model) = metric.labels.get("model") {
            let entry = categories.llm.by_model.entry(model.clone()).or_default();
            entry.output_tokens += value;
        }
    } else if name.starts_with("moltis_llm_cache_read_tokens_total") {
        categories.llm.cache_read_tokens += value;
    } else if name.starts_with("moltis_llm_cache_write_tokens_total") {
        categories.llm.cache_write_tokens += value;
    }
    // Session metrics
    else if name.starts_with("moltis_sessions_created_total") {
        categories.session.total += value;
    } else if name.starts_with("moltis_sessions_active") {
        categories.session.active = value;
        categories.system.active_sessions = value;
    }
    // Tool metrics
    else if name.starts_with("moltis_tool_executions_total") {
        categories.tools.total += value;
    } else if name.starts_with("moltis_tool_execution_errors_total") {
        categories.tools.errors += value;
    } else if name.starts_with("moltis_tool_executions_in_flight") {
        categories.tools.active = value;
    }
    // MCP metrics
    else if name.starts_with("moltis_mcp_tool_calls_total") {
        categories.mcp.total += value;
    } else if name.starts_with("moltis_mcp_tool_call_errors_total") {
        categories.mcp.errors += value;
    } else if name.starts_with("moltis_mcp_servers_connected") {
        categories.mcp.active = value;
    }
    // Memory metrics
    else if name.starts_with("moltis_memory_searches_total") {
        categories.memory.total += value;
    }
    // System metrics
    else if name.starts_with("moltis_uptime_seconds") {
        categories.system.uptime_seconds = metric.value.unwrap_or(0.0);
    } else if name.starts_with("moltis_connected_clients") {
        categories.system.connected_clients = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_prometheus_line_simple() {
        let metric = parse_prometheus_line("moltis_http_requests_total 42").unwrap();
        assert_eq!(metric.name, "moltis_http_requests_total");
        assert_eq!(metric.value, Some(42.0));
        assert!(metric.labels.is_empty());
    }

    #[test]
    fn test_parse_prometheus_line_with_labels() {
        let metric =
            parse_prometheus_line(r#"moltis_http_requests_total{method="GET",status="200"} 100"#)
                .unwrap();
        assert_eq!(metric.name, "moltis_http_requests_total");
        assert_eq!(metric.value, Some(100.0));
        assert_eq!(metric.labels.get("method"), Some(&"GET".to_string()));
        assert_eq!(metric.labels.get("status"), Some(&"200".to_string()));
    }

    #[test]
    fn test_snapshot_from_prometheus_text() {
        let text = r#"
# HELP moltis_http_requests_total Total HTTP requests
# TYPE moltis_http_requests_total counter
moltis_http_requests_total{method="GET"} 100
moltis_http_requests_total{method="POST"} 50
moltis_llm_completions_total{provider="anthropic",model="claude-3"} 25
"#;

        let snapshot = MetricsSnapshot::from_prometheus_text(text);
        assert_eq!(snapshot.metrics.len(), 3);
        assert_eq!(snapshot.categories.http.total, 150);
        assert_eq!(snapshot.categories.llm.completions_total, 25);
    }
}
