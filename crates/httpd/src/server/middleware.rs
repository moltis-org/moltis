//! HTTP middleware stack: CORS, security headers, tracing, compression.

use {
    axum::Router,
    tower_http::{
        catch_panic::CatchPanicLayer,
        compression::CompressionLayer,
        cors::{AllowOrigin, Any, CorsLayer},
        request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
        sensitive_headers::SetSensitiveHeadersLayer,
        set_header::SetResponseHeaderLayer,
        trace::{DefaultOnRequest, DefaultOnResponse, TraceLayer},
    },
    tracing::Level,
};

use super::handlers::is_same_origin;

/// 2 MiB global request body limit -- sufficient for any JSON API payload, small
/// enough to limit abuse. The upload endpoint has its own 25 MiB limit.
pub(super) const REQUEST_BODY_LIMIT: usize = 2 * 1024 * 1024;

/// Build the CORS layer with dynamic host-based origin validation.
///
/// Instead of `allow_origin(Any)`, this validates the `Origin` header against the
/// request's `Host` header using the same `is_same_origin` logic as the WebSocket
/// CSWSH protection. This is secure for Docker/cloud deployments where the hostname
/// is unknown at build time -- the server dynamically allows its own origin at
/// request time.
pub(super) fn build_cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(
            |origin: &axum::http::HeaderValue, parts: &axum::http::request::Parts| {
                let origin_str = origin.to_str().unwrap_or("");
                let host = parts
                    .headers
                    .get(axum::http::header::HOST)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                is_same_origin(origin_str, host)
            },
        ))
        .allow_methods(Any)
        .allow_headers(Any)
}

/// Apply the full middleware stack to the router.
///
/// Layer order (outermost -> innermost for requests):
/// 1. `CatchPanicLayer` -- converts handler panics to 500s
/// 2. `SetSensitiveHeadersLayer` -- marks Authorization/Cookie as redacted
/// 3. `SetRequestIdLayer` -- generates x-request-id before tracing
/// 4. `TraceLayer` (optional) -- logs requests with redacted headers + request ID
/// 5. `CorsLayer` -- handles preflight; logged by trace
/// 6. `PropagateRequestIdLayer` -- copies x-request-id to response
/// 7. Security response headers -- X-Content-Type-Options, X-Frame-Options, etc.
/// 8. `RequestBodyLimitLayer` -- rejects oversized bodies
/// 9. `CompressionLayer` (innermost) -- compresses response body
pub(super) fn apply_middleware_stack<S>(
    router: Router<S>,
    cors: CorsLayer,
    http_request_logs: bool,
) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    use axum::http::{HeaderValue, header};

    // Inner layers: compression, body limit, security headers, request ID propagation.
    let router = router
        .layer(CompressionLayer::new())
        .layer(tower_http::limit::RequestBodyLimitLayer::new(
            REQUEST_BODY_LIMIT,
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::HeaderName::from_static("x-frame-options"),
            HeaderValue::from_static("sameorigin"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::HeaderName::from_static("referrer-policy"),
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static("camera=(), microphone=(self), geolocation=(), payment=()"),
        ))
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(cors);

    // Optional trace layer -- sees redacted headers and request ID.
    let router = apply_http_trace_layer(router, http_request_logs);

    // Outer layers: request ID generation, sensitive header marking, panic catching.
    router
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .layer(SetSensitiveHeadersLayer::new([
            header::AUTHORIZATION,
            header::COOKIE,
            header::SET_COOKIE,
        ]))
        .layer(CatchPanicLayer::new())
}

/// Apply optional HTTP request/response tracing layer.
fn apply_http_trace_layer<S>(router: Router<S>, enabled: bool) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    if enabled {
        let http_trace = TraceLayer::new_for_http()
            .make_span_with(|request: &axum::http::Request<_>| {
                let request_id = request
                    .headers()
                    .get("x-request-id")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("-")
                    .to_owned();
                let user_agent = request
                    .headers()
                    .get(axum::http::header::USER_AGENT)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("-")
                    .to_owned();
                let referer = request
                    .headers()
                    .get(axum::http::header::REFERER)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("-")
                    .to_owned();
                tracing::info_span!(
                    "http_request",
                    method = %request.method(),
                    uri = %request.uri(),
                    request_id = %request_id,
                    user_agent = %user_agent,
                    referer = %referer
                )
            })
            .on_request(DefaultOnRequest::new().level(Level::INFO))
            .on_response(DefaultOnResponse::new().level(Level::INFO));
        router.layer(http_trace)
    } else {
        router
    }
}
