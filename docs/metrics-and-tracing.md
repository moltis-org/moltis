# Metrics and Tracing

Moltis includes comprehensive observability support through Prometheus metrics and
tracing integration. This document explains how to enable, configure, and use
these features.

## Overview

The metrics system is built on the [`metrics`](https://docs.rs/metrics) crate
facade, which provides a unified interface similar to the `log` crate. When the
`prometheus` feature is enabled, metrics are exported in Prometheus text format
for scraping by Grafana, Prometheus, or other monitoring tools.

All metrics are **feature-gated** — they add zero overhead when disabled.

## Feature Flags

Metrics are controlled by two feature flags:

| Feature | Description | Default |
|---------|-------------|---------|
| `metrics` | Enables metrics collection and the `/api/metrics` JSON API | Enabled |
| `prometheus` | Enables the `/metrics` Prometheus endpoint (requires `metrics`) | Enabled |

### Compile-Time Configuration

```toml
# Enable only metrics collection (no Prometheus endpoint)
moltis-gateway = { version = "0.1", features = ["metrics"] }

# Enable metrics with Prometheus export (default)
moltis-gateway = { version = "0.1", features = ["metrics", "prometheus"] }

# Enable metrics for specific crates
moltis-agents = { version = "0.1", features = ["metrics"] }
moltis-cron = { version = "0.1", features = ["metrics"] }
```

To build without metrics entirely:

```bash
cargo build --release --no-default-features --features "file-watcher,tailscale,tls,web-ui"
```

## Prometheus Endpoint

When the `prometheus` feature is enabled, the gateway exposes a `/metrics` endpoint:

```
GET http://localhost:18789/metrics
```

This endpoint is **unauthenticated** to allow Prometheus scrapers to access it.
It returns metrics in Prometheus text format:

```
# HELP moltis_http_requests_total Total number of HTTP requests handled
# TYPE moltis_http_requests_total counter
moltis_http_requests_total{method="GET",status="200",endpoint="/api/chat"} 42

# HELP moltis_llm_completion_duration_seconds Duration of LLM completion requests
# TYPE moltis_llm_completion_duration_seconds histogram
moltis_llm_completion_duration_seconds_bucket{provider="anthropic",model="claude-3-opus",le="1.0"} 5
```

### Grafana Integration

To scrape metrics with Prometheus and visualize in Grafana:

1. Add moltis to your `prometheus.yml`:

```yaml
scrape_configs:
  - job_name: 'moltis'
    static_configs:
      - targets: ['localhost:18789']
    metrics_path: /metrics
    scrape_interval: 15s
```

2. Import or create Grafana dashboards using the `moltis_*` metrics.

## JSON API Endpoints

For the web UI dashboard and programmatic access, authenticated JSON endpoints
are available:

| Endpoint | Description |
|----------|-------------|
| `GET /api/metrics` | Full metrics snapshot with aggregates and per-provider breakdown |
| `GET /api/metrics/summary` | Lightweight counts for navigation badges |
| `GET /api/metrics/history` | Time-series data points for charts (last hour, 10s intervals) |

### History Endpoint

The `/api/metrics/history` endpoint returns historical metrics data for rendering
time-series charts:

```json
{
  "enabled": true,
  "interval_seconds": 10,
  "max_points": 60480,
  "points": [
    {
      "timestamp": 1706832000000,
      "llm_completions": 42,
      "llm_input_tokens": 15000,
      "llm_output_tokens": 8000,
      "http_requests": 150,
      "ws_active": 3,
      "tool_executions": 25,
      "mcp_calls": 12,
      "active_sessions": 2
    }
  ]
}
```

## Metrics Persistence

Metrics history is persisted to SQLite, so historical data survives server
restarts. The database is stored at `~/.moltis/metrics.db` (or the configured
data directory).

Key features:

- **7-day retention**: History is kept for 7 days (60,480 data points at
  10-second intervals)
- **Automatic cleanup**: Old data is automatically removed hourly
- **Startup recovery**: History is loaded from the database when the server
  starts

The storage backend uses a trait-based design (`MetricsStore`), allowing
alternative implementations (e.g., TimescaleDB) for larger deployments.

### Storage Architecture

```rust
// The MetricsStore trait defines the storage interface
#[async_trait]
pub trait MetricsStore: Send + Sync {
    async fn save_point(&self, point: &MetricsHistoryPoint) -> Result<()>;
    async fn load_history(&self, since: u64, limit: usize) -> Result<Vec<MetricsHistoryPoint>>;
    async fn cleanup_before(&self, before: u64) -> Result<u64>;
    async fn latest_point(&self) -> Result<Option<MetricsHistoryPoint>>;
}
```

The default `SqliteMetricsStore` implementation stores data in a single table
with an index on the timestamp column for efficient range queries.

## Web UI Dashboard

The gateway includes a built-in metrics dashboard at `/monitoring` in the web UI.
This page displays:

**Overview Tab:**
- System metrics (uptime, connected clients, active sessions)
- LLM usage (completions, tokens, cache statistics)
- Tool execution statistics
- MCP server status
- Provider breakdown table
- Prometheus endpoint (with copy button)

**Charts Tab:**
- Token usage over time (input/output)
- HTTP requests and LLM completions
- WebSocket connections and active sessions
- Tool executions and MCP calls

The dashboard uses [uPlot](https://github.com/leeoniya/uPlot) for lightweight,
high-performance time-series charts. Data updates every 10 seconds for current
metrics and every 30 seconds for history.

## Available Metrics

### HTTP Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `moltis_http_requests_total` | Counter | method, status, endpoint | Total HTTP requests |
| `moltis_http_request_duration_seconds` | Histogram | method, status, endpoint | Request latency |
| `moltis_http_requests_in_flight` | Gauge | — | Currently processing requests |

### LLM/Agent Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `moltis_llm_completions_total` | Counter | provider, model | Total completions requested |
| `moltis_llm_completion_duration_seconds` | Histogram | provider, model | Completion latency |
| `moltis_llm_input_tokens_total` | Counter | provider, model | Input tokens processed |
| `moltis_llm_output_tokens_total` | Counter | provider, model | Output tokens generated |
| `moltis_llm_completion_errors_total` | Counter | provider, model, error_type | Completion failures |
| `moltis_llm_time_to_first_token_seconds` | Histogram | provider, model | Streaming TTFT |

#### Provider Aliases

When you have multiple instances of the same provider type (e.g., separate API keys
for work and personal use), you can use the `alias` configuration option to
differentiate them in metrics:

```toml
[providers.anthropic]
api_key = "sk-work-..."
alias = "anthropic-work"

# Note: You would need separate config sections for multiple instances
# of the same provider. This is a placeholder for future functionality.
```

The alias appears in the `provider` label of all LLM metrics:

```
moltis_llm_input_tokens_total{provider="anthropic-work", model="claude-3-opus"} 5000
moltis_llm_input_tokens_total{provider="anthropic-personal", model="claude-3-opus"} 3000
```

This allows you to:
- Track token usage separately for billing purposes
- Create separate Grafana dashboards per provider instance
- Monitor rate limits and quotas independently

### MCP (Model Context Protocol) Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `moltis_mcp_tool_calls_total` | Counter | server, tool | Tool invocations |
| `moltis_mcp_tool_call_duration_seconds` | Histogram | server, tool | Tool call latency |
| `moltis_mcp_tool_call_errors_total` | Counter | server, tool, error_type | Tool call failures |
| `moltis_mcp_servers_connected` | Gauge | — | Active MCP server connections |

### Tool Execution Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `moltis_tool_executions_total` | Counter | tool | Tool executions |
| `moltis_tool_execution_duration_seconds` | Histogram | tool | Execution time |
| `moltis_sandbox_command_executions_total` | Counter | — | Sandbox commands run |

### Session Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `moltis_sessions_created_total` | Counter | — | Sessions created |
| `moltis_sessions_active` | Gauge | — | Currently active sessions |
| `moltis_session_messages_total` | Counter | role | Messages by role |

### Cron Job Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `moltis_cron_jobs_scheduled` | Gauge | — | Number of scheduled jobs |
| `moltis_cron_executions_total` | Counter | — | Job executions |
| `moltis_cron_execution_duration_seconds` | Histogram | — | Job duration |
| `moltis_cron_errors_total` | Counter | — | Failed jobs |
| `moltis_cron_stuck_jobs_cleared_total` | Counter | — | Jobs exceeding 2h timeout |
| `moltis_cron_input_tokens_total` | Counter | — | Input tokens from cron runs |
| `moltis_cron_output_tokens_total` | Counter | — | Output tokens from cron runs |

### Memory/Search Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `moltis_memory_searches_total` | Counter | search_type | Searches performed |
| `moltis_memory_search_duration_seconds` | Histogram | search_type | Search latency |
| `moltis_memory_embeddings_generated_total` | Counter | provider | Embeddings created |

### Channel Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `moltis_channels_active` | Gauge | — | Loaded channel plugins |
| `moltis_channel_messages_received_total` | Counter | channel | Inbound messages |
| `moltis_channel_messages_sent_total` | Counter | channel | Outbound messages |

### Telegram-Specific Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `moltis_telegram_messages_received_total` | Counter | — | Messages from Telegram |
| `moltis_telegram_access_control_denials_total` | Counter | — | Access denied events |
| `moltis_telegram_polling_duration_seconds` | Histogram | — | Message handling time |

### OAuth Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `moltis_oauth_flow_starts_total` | Counter | — | OAuth flows initiated |
| `moltis_oauth_flow_completions_total` | Counter | — | Successful completions |
| `moltis_oauth_token_refresh_total` | Counter | — | Token refreshes |
| `moltis_oauth_token_refresh_failures_total` | Counter | — | Refresh failures |

### Skills Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `moltis_skills_installation_attempts_total` | Counter | — | Installation attempts |
| `moltis_skills_installation_duration_seconds` | Histogram | — | Installation time |
| `moltis_skills_git_clone_total` | Counter | — | Successful git clones |
| `moltis_skills_git_clone_fallback_total` | Counter | — | Fallbacks to HTTP tarball |

## Tracing Integration

The `moltis-metrics` crate includes optional tracing integration via the
`tracing` feature. This allows span context to propagate to metric labels.

### Enabling Tracing

```toml
moltis-metrics = { version = "0.1", features = ["prometheus", "tracing"] }
```

### Initialization

```rust
use moltis_metrics::tracing_integration::init_tracing;

fn main() {
    // Initialize tracing with metrics context propagation
    init_tracing();

    // Now spans will add labels to metrics
}
```

### How It Works

When tracing is enabled, span fields are automatically added as metric labels:

```rust
use tracing::instrument;

#[instrument(fields(operation = "fetch_user", component = "api"))]
async fn fetch_user(id: u64) -> User {
    // Metrics recorded here will include:
    // - operation="fetch_user"
    // - component="api"
    counter!("api_calls_total").increment(1);
}
```

### Span Labels

The following span fields are propagated to metrics:

| Field | Description |
|-------|-------------|
| `operation` | The operation being performed |
| `component` | The component/module name |
| `span.name` | The span's target/name |

## Adding Custom Metrics

### In Your Code

Use the `metrics` macros re-exported from `moltis-metrics`:

```rust
use moltis_metrics::{counter, gauge, histogram, labels};

// Simple counter
counter!("my_custom_requests_total").increment(1);

// Counter with labels
counter!(
    "my_custom_requests_total",
    labels::ENDPOINT => "/api/users",
    labels::METHOD => "GET"
).increment(1);

// Gauge (current value)
gauge!("my_queue_size").set(42.0);

// Histogram (distribution)
histogram!("my_operation_duration_seconds").record(0.123);
```

### Feature-Gating

Always gate metrics code to avoid overhead when disabled:

```rust
#[cfg(feature = "metrics")]
use moltis_metrics::{counter, histogram};

pub async fn my_function() {
    #[cfg(feature = "metrics")]
    let start = std::time::Instant::now();

    // ... do work ...

    #[cfg(feature = "metrics")]
    {
        counter!("my_operations_total").increment(1);
        histogram!("my_operation_duration_seconds")
            .record(start.elapsed().as_secs_f64());
    }
}
```

### Adding New Metric Definitions

For consistency, add metric name constants to `crates/metrics/src/definitions.rs`:

```rust
/// My feature metrics
pub mod my_feature {
    /// Total operations performed
    pub const OPERATIONS_TOTAL: &str = "moltis_my_feature_operations_total";
    /// Operation duration in seconds
    pub const OPERATION_DURATION_SECONDS: &str = "moltis_my_feature_operation_duration_seconds";
}
```

Then use them:

```rust
use moltis_metrics::{counter, my_feature};

counter!(my_feature::OPERATIONS_TOTAL).increment(1);
```

## Configuration

Metrics configuration in `moltis.toml`:

```toml
[metrics]
enabled = true              # Enable metrics collection (default: true)
prometheus_endpoint = true  # Expose /metrics endpoint (default: true)
labels = { env = "prod" }   # Add custom labels to all metrics
```

Environment variables:

- `RUST_LOG=moltis_metrics=debug` — Enable debug logging for metrics initialization

## Best Practices

1. **Use consistent naming**: Follow the pattern `moltis_<subsystem>_<metric>_<unit>`
2. **Add units to names**: `_total` for counters, `_seconds` for durations, `_bytes` for sizes
3. **Keep cardinality low**: Avoid high-cardinality labels (like user IDs or request IDs)
4. **Feature-gate everything**: Use `#[cfg(feature = "metrics")]` to ensure zero overhead when disabled
5. **Use predefined buckets**: The `buckets` module has standard histogram buckets for common metric types

## Troubleshooting

### Metrics not appearing

1. Verify the `metrics` feature is enabled at compile time
2. Check that the metrics recorder is initialized (happens automatically in gateway)
3. Ensure you're hitting the correct `/metrics` endpoint
4. Check `moltis.toml` has `[metrics] enabled = true`

### Prometheus endpoint not available

1. Ensure the `prometheus` feature is enabled (it's separate from `metrics`)
2. Check your build: `cargo build --features prometheus`

### High memory usage

- Check for high-cardinality labels (many unique label combinations)
- Consider reducing histogram bucket counts

### Missing labels

- Ensure labels are passed consistently across all metric recordings
- Check that tracing spans include the expected fields
