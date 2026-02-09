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
            Response::builder()
                .status(StatusCode::OK)
                .header(
                    header::CONTENT_TYPE,
                    "text/plain; version=0.0.4; charset=utf-8",
                )
                .body(body)
                .unwrap()
        },
        None => Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .header(header::CONTENT_TYPE, "text/plain")
            .body("Metrics not enabled".to_string())
            .unwrap(),
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
/// Returns the last hour of metrics snapshots (sampled every 10 seconds)
/// for rendering charts in the monitoring UI.
#[cfg(feature = "metrics")]
pub async fn api_metrics_history_handler(State(state): State<AppState>) -> impl IntoResponse {
    let inner = state.gateway.inner.read().await;
    let points: Vec<_> = inner.metrics_history.iter().collect();

    Json(serde_json::json!({
        "enabled": true,
        "interval_seconds": 10,
        "max_points": 360,
        "points": points,
    }))
}
