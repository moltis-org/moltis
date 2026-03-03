//! HTTP request metrics middleware.
//!
//! This middleware collects Prometheus metrics for all HTTP requests passing through
//! the gateway. It tracks request counts, durations, and in-flight requests.

#[cfg(feature = "metrics")]
use std::time::Instant;

#[cfg(feature = "metrics")]
use axum::{body::Body, http::Request, middleware::Next, response::Response};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, gauge, histogram};

#[cfg(feature = "metrics")]
use moltis_metrics::{http as http_metrics, labels};

/// Middleware that collects HTTP request metrics.
///
/// This records:
/// - `moltis_http_requests_total`: Counter of total requests by endpoint, method, and status
/// - `moltis_http_request_duration_seconds`: Histogram of request durations
/// - `moltis_http_requests_in_flight`: Gauge of currently processing requests
#[cfg(feature = "metrics")]
pub async fn http_metrics_middleware(request: Request<Body>, next: Next) -> Response {
    let start = Instant::now();
    let method = request.method().to_string();
    let path = request.uri().path().to_string();

    // Normalize the path for metrics (remove dynamic segments)
    let endpoint = normalize_path(&path);

    // Increment in-flight gauge
    gauge!(http_metrics::REQUESTS_IN_FLIGHT, labels::ENDPOINT => endpoint.clone(), labels::METHOD => method.clone())
        .increment(1.0);

    // Process the request
    let response = next.run(request).await;

    // Get response status
    let status = response.status().as_u16().to_string();

    // Record metrics
    let duration = start.elapsed().as_secs_f64();

    counter!(
        http_metrics::REQUESTS_TOTAL,
        labels::ENDPOINT => endpoint.clone(),
        labels::METHOD => method.clone(),
        labels::STATUS => status.clone()
    )
    .increment(1);

    histogram!(
        http_metrics::REQUEST_DURATION_SECONDS,
        labels::ENDPOINT => endpoint.clone(),
        labels::METHOD => method.clone(),
        labels::STATUS => status
    )
    .record(duration);

    // Decrement in-flight gauge
    gauge!(http_metrics::REQUESTS_IN_FLIGHT, labels::ENDPOINT => endpoint, labels::METHOD => method)
        .decrement(1.0);

    response
}

/// Normalize a URL path for metric labels.
///
/// This replaces dynamic segments (UUIDs, IDs) with placeholders to prevent
/// high cardinality in metric labels.
#[cfg(feature = "metrics")]
fn normalize_path(path: &str) -> String {
    // Handle common patterns
    let normalized = path
        // Replace UUIDs and numeric IDs
        .split('/')
        .map(|segment| {
            // Check if segment looks like a UUID or numeric ID
            let is_dynamic = looks_like_uuid(segment)
                || (segment.chars().all(|c| c.is_ascii_digit()) && !segment.is_empty());
            if is_dynamic { "{id}" } else { segment }
        })
        .collect::<Vec<_>>()
        .join("/");

    // Collapse consecutive slashes and trailing slashes
    let mut result = normalized;
    while result.contains("//") {
        result = result.replace("//", "/");
    }
    if result.len() > 1 && result.ends_with('/') {
        result.pop();
    }
    if result.is_empty() {
        "/".to_string()
    } else {
        result
    }
}

/// Check if a string looks like a UUID.
#[cfg(feature = "metrics")]
fn looks_like_uuid(s: &str) -> bool {
    // UUID format: 8-4-4-4-12 hex chars with dashes
    // Also match compact UUIDs (32 hex chars)
    if s.len() == 36 {
        let parts: Vec<&str> = s.split('-').collect();
        parts.len() == 5
            && parts[0].len() == 8
            && parts[1].len() == 4
            && parts[2].len() == 4
            && parts[3].len() == 4
            && parts[4].len() == 12
            && s.chars()
                .filter(|c| *c != '-')
                .all(|c| c.is_ascii_hexdigit())
    } else if s.len() == 32 {
        s.chars().all(|c| c.is_ascii_hexdigit())
    } else {
        false
    }
}

#[cfg(all(test, feature = "metrics"))]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_path() {
        assert_eq!(normalize_path("/api/sessions"), "/api/sessions");
        assert_eq!(
            normalize_path("/api/sessions/123e4567-e89b-12d3-a456-426614174000"),
            "/api/sessions/{id}"
        );
        assert_eq!(normalize_path("/api/users/12345"), "/api/users/{id}");
        assert_eq!(normalize_path("/"), "/");
        assert_eq!(normalize_path("/api/"), "/api");
    }

    #[test]
    fn test_looks_like_uuid() {
        assert!(looks_like_uuid("123e4567-e89b-12d3-a456-426614174000"));
        assert!(looks_like_uuid("123e4567e89b12d3a456426614174000"));
        assert!(!looks_like_uuid("not-a-uuid"));
        assert!(!looks_like_uuid("12345"));
    }
}
