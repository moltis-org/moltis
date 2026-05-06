//! Metrics API routes for Prometheus scraping and internal UI.

#[cfg(feature = "metrics")]
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
};

#[cfg(feature = "prometheus")]
use axum::{http::header, response::Response};

#[cfg(feature = "metrics")]
use moltis_metrics::MetricsSnapshot;

#[cfg(feature = "metrics")]
use crate::server::AppState;

#[cfg(feature = "metrics")]
const METRICS_NOT_ENABLED: &str = "METRICS_NOT_ENABLED";

/// Prometheus metrics endpoint handler.
///
/// Returns metrics in Prometheus text exposition format, suitable for scraping
/// by Prometheus, Victoria Metrics, or other compatible collectors.
///
/// This endpoint is unauthenticated to allow metric scrapers to access it.
#[cfg(feature = "prometheus")]
pub async fn prometheus_metrics_handler(State(state): State<AppState>) -> impl IntoResponse {
    let metrics_handle = state.gateway.metrics_handle.as_ref();

    match metrics_handle {
        Some(handle) => {
            let body = handle.render();
            #[allow(clippy::unwrap_used)] // building response with valid static headers
            Response::builder()
                .status(StatusCode::OK)
                .header(
                    header::CONTENT_TYPE,
                    "text/plain; version=0.0.4; charset=utf-8",
                )
                .body(body)
                .unwrap()
        },
        None => {
            #[allow(clippy::unwrap_used)] // building response with valid static headers
            Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .header(header::CONTENT_TYPE, "text/plain")
                .body("Metrics not enabled".to_string())
                .unwrap()
        },
    }
}

/// Internal metrics API handler for the web UI.
///
/// Returns metrics as structured JSON, with pre-computed aggregates and
/// category breakdowns suitable for dashboard display.
#[cfg(feature = "metrics")]
pub async fn api_metrics_handler(State(state): State<AppState>) -> impl IntoResponse {
    let metrics_handle = state.gateway.metrics_handle.as_ref();

    match metrics_handle {
        Some(handle) => {
            let prometheus_text = handle.render();
            let snapshot = MetricsSnapshot::from_prometheus_text(&prometheus_text);
            Json(snapshot).into_response()
        },
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "code": METRICS_NOT_ENABLED,
                "error": "Metrics not enabled"
            })),
        )
            .into_response(),
    }
}

/// Metrics summary for the navigation badge.
///
/// Returns a minimal summary suitable for displaying in the UI navigation.
#[cfg(feature = "metrics")]
pub async fn api_metrics_summary_handler(State(state): State<AppState>) -> impl IntoResponse {
    let metrics_handle = state.gateway.metrics_handle.as_ref();

    match metrics_handle {
        Some(handle) => {
            let prometheus_text = handle.render();
            let snapshot = MetricsSnapshot::from_prometheus_text(&prometheus_text);

            Json(serde_json::json!({
                "enabled": true,
                "llm": {
                    "completions": snapshot.categories.llm.completions_total,
                    "input_tokens": snapshot.categories.llm.input_tokens,
                    "output_tokens": snapshot.categories.llm.output_tokens,
                    "errors": snapshot.categories.llm.errors,
                },
                "http": {
                    "requests": snapshot.categories.http.total,
                    "active": snapshot.categories.http.active,
                },
                "websocket": {
                    "connections": snapshot.categories.websocket.total,
                    "active": snapshot.categories.websocket.active,
                },
                "sessions": {
                    "active": snapshot.categories.system.active_sessions,
                },
                "tools": {
                    "executions": snapshot.categories.tools.total,
                    "errors": snapshot.categories.tools.errors,
                },
                "uptime_seconds": snapshot.categories.system.uptime_seconds,
            }))
            .into_response()
        },
        None => Json(serde_json::json!({
            "enabled": false
        }))
        .into_response(),
    }
}

/// Historical metrics data for time-series charts.
///
/// Returns metrics snapshots (sampled every 30 seconds)
/// for rendering charts in the monitoring UI.
#[cfg(feature = "metrics")]
pub async fn api_metrics_history_handler(State(state): State<AppState>) -> impl IntoResponse {
    let inner = state.gateway.inner.read().await;
    let max_points = inner.metrics_history.capacity();
    let points: Vec<_> = inner.metrics_history.iter().collect();

    Json(serde_json::json!({
        "enabled": true,
        "interval_seconds": 30,
        "max_points": max_points,
        "points": points,
    }))
}

/// Insights endpoint: longer-range usage analytics from the SQLite metrics store.
///
/// Query parameters:
/// - `days`: Number of days to look back (default: 30, max: 365)
#[cfg(feature = "metrics")]
pub async fn api_metrics_insights_handler(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let days: u64 = params
        .get("days")
        .and_then(|v| v.parse().ok())
        .unwrap_or(30)
        .min(365);

    let Some(ref store) = state.gateway.metrics_store else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": METRICS_NOT_ENABLED })),
        )
            .into_response();
    };

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let since_ms = now_ms.saturating_sub(days * 24 * 60 * 60 * 1000);

    let history = match store.load_history(since_ms, 100_000).await {
        Ok(h) => h,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("failed to load history: {e}") })),
            )
                .into_response();
        },
    };

    if history.is_empty() {
        return Json(serde_json::json!({
            "days": days,
            "completions": 0,
            "input_tokens": 0,
            "output_tokens": 0,
            "errors": 0,
            "tool_executions": 0,
            "tool_errors": 0,
            "by_provider": {},
            "data_points": 0,
        }))
        .into_response();
    }

    // Compute deltas between consecutive points (metrics are cumulative counters).
    let mut total_completions: u64 = 0;
    let mut total_input_tokens: u64 = 0;
    let mut total_output_tokens: u64 = 0;
    let mut total_errors: u64 = 0;
    let mut total_tool_executions: u64 = 0;
    let mut total_tool_errors: u64 = 0;
    let mut provider_totals: std::collections::HashMap<String, serde_json::Value> =
        std::collections::HashMap::new();

    let mut prev: Option<&moltis_metrics::MetricsHistoryPoint> = None;
    for point in &history {
        if let Some(p) = prev {
            total_completions += point.llm_completions.saturating_sub(p.llm_completions);
            total_input_tokens += point.llm_input_tokens.saturating_sub(p.llm_input_tokens);
            total_output_tokens += point.llm_output_tokens.saturating_sub(p.llm_output_tokens);
            total_errors += point.llm_errors.saturating_sub(p.llm_errors);
            total_tool_executions += point.tool_executions.saturating_sub(p.tool_executions);
            total_tool_errors += point.tool_errors.saturating_sub(p.tool_errors);

            for (provider, tokens) in &point.by_provider {
                let prev_tokens = p.by_provider.get(provider);
                let delta_in = tokens
                    .input_tokens
                    .saturating_sub(prev_tokens.map_or(0, |pt| pt.input_tokens));
                let delta_out = tokens
                    .output_tokens
                    .saturating_sub(prev_tokens.map_or(0, |pt| pt.output_tokens));
                let delta_completions = tokens
                    .completions
                    .saturating_sub(prev_tokens.map_or(0, |pt| pt.completions));

                let entry = provider_totals.entry(provider.clone()).or_insert_with(|| {
                    serde_json::json!({
                        "input_tokens": 0u64,
                        "output_tokens": 0u64,
                        "completions": 0u64,
                    })
                });
                if let Some(obj) = entry.as_object_mut() {
                    *obj.entry("input_tokens").or_insert(serde_json::json!(0)) =
                        serde_json::json!(obj["input_tokens"].as_u64().unwrap_or(0) + delta_in);
                    *obj.entry("output_tokens").or_insert(serde_json::json!(0)) =
                        serde_json::json!(obj["output_tokens"].as_u64().unwrap_or(0) + delta_out);
                    *obj.entry("completions").or_insert(serde_json::json!(0)) = serde_json::json!(
                        obj["completions"].as_u64().unwrap_or(0) + delta_completions
                    );
                }
            }
        }
        prev = Some(point);
    }

    let first_ts = history.first().map(|p| p.timestamp).unwrap_or(0);
    let last_ts = history.last().map(|p| p.timestamp).unwrap_or(0);
    let span_hours = (last_ts.saturating_sub(first_ts)) as f64 / 3_600_000.0;

    Json(serde_json::json!({
        "days": days,
        "completions": total_completions,
        "input_tokens": total_input_tokens,
        "output_tokens": total_output_tokens,
        "total_tokens": total_input_tokens + total_output_tokens,
        "errors": total_errors,
        "tool_executions": total_tool_executions,
        "tool_errors": total_tool_errors,
        "by_provider": provider_totals,
        "data_points": history.len(),
        "span_hours": span_hours,
        "first_timestamp": first_ts,
        "last_timestamp": last_ts,
    }))
    .into_response()
}
