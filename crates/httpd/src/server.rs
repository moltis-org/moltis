//! HTTP server entry points, middleware stack, and router construction.
//!
//! This module contains the HTTP-specific layer of the moltis gateway:
//! `AppState`, router building, middleware, handlers, and server startup.
//! Core business logic lives in `moltis-gateway`; this crate depends on it
//! but never the reverse.

use std::{collections::HashMap, net::SocketAddr, path::PathBuf, sync::Arc};

use {
    axum::{
        Router,
        extract::{ConnectInfo, State, WebSocketUpgrade},
        http::StatusCode,
        response::{IntoResponse, Json},
        routing::get,
    },
    tower_http::{
        catch_panic::CatchPanicLayer,
        compression::CompressionLayer,
        cors::{AllowOrigin, Any, CorsLayer},
        request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
        sensitive_headers::SetSensitiveHeadersLayer,
        set_header::SetResponseHeaderLayer,
        trace::{DefaultOnRequest, DefaultOnResponse, TraceLayer},
    },
    tracing::{Level, info, warn},
};

use {moltis_channels::ChannelPlugin, moltis_protocol::TICK_INTERVAL_MS};

use moltis_sessions::session_events::{SessionEvent, SessionEventBus};

use moltis_gateway::{
    auth,
    auth_webauthn::SharedWebAuthnRegistry,
    broadcast::{BroadcastOpts, broadcast, broadcast_tick},
    methods::MethodRegistry,
    server::{PreparedGatewayCore, prepare_gateway_core},
    state::GatewayState,
    update_check::{UPDATE_CHECK_INTERVAL, fetch_update_availability, resolve_releases_url},
};

use crate::{
    auth_routes::{AuthState, auth_router},
    ws::handle_connection,
};

#[cfg(feature = "tailscale")]
use moltis_gateway::tailscale::{CliTailscaleManager, TailscaleManager, TailscaleMode};

#[cfg(feature = "tls")]
use moltis_gateway::tls::CertManager;

/// Options for tailscale serve/funnel passed from CLI flags.
#[cfg(feature = "tailscale")]
pub use moltis_gateway::server::TailscaleOpts;

#[cfg(test)]
fn should_prebuild_sandbox_image(
    mode: &moltis_tools::sandbox::SandboxMode,
    packages: &[String],
) -> bool {
    !matches!(mode, moltis_tools::sandbox::SandboxMode::Off) && !packages.is_empty()
}

fn instance_slug(config: &moltis_config::MoltisConfig) -> String {
    let mut raw_name = config.identity.name.clone();
    if let Some(file_identity) = moltis_config::load_identity()
        && file_identity.name.is_some()
    {
        raw_name = file_identity.name;
    }

    let base = raw_name
        .unwrap_or_else(|| "moltis".to_string())
        .to_lowercase();
    let mut out = String::new();
    let mut last_dash = false;
    for ch in base.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            ch
        } else {
            '-'
        };
        if mapped == '-' {
            if !last_dash {
                out.push(mapped);
            }
            last_dash = true;
        } else {
            out.push(mapped);
            last_dash = false;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "moltis".to_string()
    } else {
        out
    }
}

// ── Shared app state ─────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub gateway: Arc<GatewayState>,
    pub methods: Arc<MethodRegistry>,
    pub request_throttle: Arc<crate::request_throttle::RequestThrottle>,
    pub webauthn_registry: Option<SharedWebAuthnRegistry>,
    #[cfg(feature = "push-notifications")]
    pub push_service: Option<Arc<moltis_gateway::push::PushService>>,
    #[cfg(feature = "graphql")]
    pub graphql_schema: moltis_graphql::MoltisSchema,
}

/// Function signature for adding extra routes (e.g. web-UI) to the gateway.
pub type RouteEnhancer = fn() -> Router<AppState>;

// ── Server startup ───────────────────────────────────────────────────────────

/// Build the API routes (shared between both build_gateway_app versions).
///
/// Auth is enforced by `auth_gate` middleware on the whole router — these
/// routes no longer carry their own auth layer.
/// Add feature-specific routes to API routes.
/// Build the CORS layer with dynamic host-based origin validation.
///
/// Instead of `allow_origin(Any)`, this validates the `Origin` header against the
/// request's `Host` header using the same `is_same_origin` logic as the WebSocket
/// CSWSH protection. This is secure for Docker/cloud deployments where the hostname
/// is unknown at build time — the server dynamically allows its own origin at
/// request time.
fn build_cors_layer() -> CorsLayer {
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

/// 2 MiB global request body limit — sufficient for any JSON API payload, small
/// enough to limit abuse. The upload endpoint has its own 25 MiB limit.
const REQUEST_BODY_LIMIT: usize = 2 * 1024 * 1024;

/// Apply the full middleware stack to the router.
///
/// Layer order (outermost → innermost for requests):
/// 1. `CatchPanicLayer` — converts handler panics to 500s
/// 2. `SetSensitiveHeadersLayer` — marks Authorization/Cookie as redacted
/// 3. `SetRequestIdLayer` — generates x-request-id before tracing
/// 4. `TraceLayer` (optional) — logs requests with redacted headers + request ID
/// 5. `CorsLayer` — handles preflight; logged by trace
/// 6. `PropagateRequestIdLayer` — copies x-request-id to response
/// 7. Security response headers — X-Content-Type-Options, X-Frame-Options, etc.
/// 8. `RequestBodyLimitLayer` — rejects oversized bodies
/// 9. `CompressionLayer` (innermost) — compresses response body
fn apply_middleware_stack<S>(
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
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(cors);

    // Optional trace layer — sees redacted headers and request ID.
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

/// Build the gateway base router and `AppState` without throttle, middleware,
/// or state consumption. Callers can merge additional routes (e.g. web-UI)
/// before calling [`finalize_gateway_app`].
#[cfg(feature = "push-notifications")]
pub fn build_gateway_base(
    state: Arc<GatewayState>,
    methods: Arc<MethodRegistry>,
    push_service: Option<Arc<moltis_gateway::push::PushService>>,
    webauthn_registry: Option<SharedWebAuthnRegistry>,
) -> (Router<AppState>, AppState) {
    let mut router = Router::new()
        .route("/health", get(health_handler))
        .route("/ws/chat", get(ws_upgrade_handler));

    // Nest auth routes if credential store is available.
    if let Some(ref cred_store) = state.credential_store {
        let auth_state = AuthState {
            credential_store: Arc::clone(cred_store),
            webauthn_registry: webauthn_registry.clone(),
            gateway_state: Arc::clone(&state),
        };
        router = router.nest("/api/auth", auth_router().with_state(auth_state));
    }

    #[cfg(feature = "graphql")]
    let graphql_schema = {
        let system_info = Arc::new(crate::graphql_routes::GatewaySystemInfoService {
            state: Arc::clone(&state),
        });
        let services = state.services.to_services(system_info);
        moltis_graphql::build_schema(services, state.graphql_broadcast.clone())
    };

    let app_state = AppState {
        gateway: state,
        methods,
        request_throttle: Arc::new(crate::request_throttle::RequestThrottle::new()),
        webauthn_registry: webauthn_registry.clone(),
        push_service,
        #[cfg(feature = "graphql")]
        graphql_schema,
    };

    // GraphQL routes — auth is handled by the global auth_gate in
    // finalize_gateway_app.
    #[cfg(feature = "graphql")]
    {
        router = router.route(
            "/graphql",
            get(crate::graphql_routes::graphql_get_handler)
                .post(crate::graphql_routes::graphql_handler),
        );
    }

    (router, app_state)
}

/// Build the gateway base router and `AppState` without throttle, middleware,
/// or state consumption. Callers can merge additional routes (e.g. web-UI)
/// before calling [`finalize_gateway_app`].
#[cfg(not(feature = "push-notifications"))]
pub fn build_gateway_base(
    state: Arc<GatewayState>,
    methods: Arc<MethodRegistry>,
    webauthn_registry: Option<SharedWebAuthnRegistry>,
) -> (Router<AppState>, AppState) {
    let mut router = Router::new()
        .route("/health", get(health_handler))
        .route("/ws/chat", get(ws_upgrade_handler));

    // Add Prometheus metrics endpoint (unauthenticated for scraping).
    #[cfg(feature = "prometheus")]
    {
        router = router.route(
            "/metrics",
            get(crate::metrics_routes::prometheus_metrics_handler),
        );
    }

    // Nest auth routes if credential store is available.
    if let Some(ref cred_store) = state.credential_store {
        let auth_state = AuthState {
            credential_store: Arc::clone(cred_store),
            webauthn_registry: webauthn_registry.clone(),
            gateway_state: Arc::clone(&state),
        };
        router = router.nest("/api/auth", auth_router().with_state(auth_state));
    }

    #[cfg(feature = "graphql")]
    let graphql_schema = {
        let system_info = Arc::new(crate::graphql_routes::GatewaySystemInfoService {
            state: Arc::clone(&state),
        });
        let services = state.services.to_services(system_info);
        moltis_graphql::build_schema(services, state.graphql_broadcast.clone())
    };

    let app_state = AppState {
        gateway: state,
        methods,
        request_throttle: Arc::new(crate::request_throttle::RequestThrottle::new()),
        webauthn_registry: webauthn_registry.clone(),
        #[cfg(feature = "graphql")]
        graphql_schema,
    };

    // GraphQL routes — auth is handled by the global auth_gate in
    // finalize_gateway_app.
    #[cfg(feature = "graphql")]
    {
        router = router.route(
            "/graphql",
            get(crate::graphql_routes::graphql_get_handler)
                .post(crate::graphql_routes::graphql_handler),
        );
    }

    (router, app_state)
}

/// Apply throttle, auth gate, middleware, and state to a base router,
/// producing the final `Router` ready for `axum::serve`.
pub fn finalize_gateway_app(
    router: Router<AppState>,
    app_state: AppState,
    http_request_logs: bool,
) -> Router {
    let cors = build_cors_layer();
    // Auth gate covers the entire router — public paths are exempted inside
    // `is_public_path()`.  Only compiled when the web-ui feature is enabled
    // (matches the old architecture where auth_gate was global).
    #[cfg(feature = "web-ui")]
    let router = router.layer(axum::middleware::from_fn_with_state(
        app_state.clone(),
        crate::auth_middleware::auth_gate,
    ));
    // Vault guard blocks API requests when the vault is sealed (not
    // uninitialized). Applied after auth_gate so sealed state is checked
    // only for authenticated requests.
    #[cfg(feature = "vault")]
    let router = router.layer(axum::middleware::from_fn_with_state(
        app_state.clone(),
        crate::auth_middleware::vault_guard,
    ));
    let router = router.layer(axum::middleware::from_fn_with_state(
        app_state.clone(),
        crate::request_throttle::throttle_gate,
    ));
    let router = apply_middleware_stack(router, cors, http_request_logs);
    router.with_state(app_state)
}

/// Convenience wrapper: build base + finalize in one call (used by tests).
#[cfg(feature = "push-notifications")]
pub fn build_gateway_app(
    state: Arc<GatewayState>,
    methods: Arc<MethodRegistry>,
    push_service: Option<Arc<moltis_gateway::push::PushService>>,
    http_request_logs: bool,
    webauthn_registry: Option<SharedWebAuthnRegistry>,
) -> Router {
    let (router, app_state) = build_gateway_base(state, methods, push_service, webauthn_registry);
    finalize_gateway_app(router, app_state, http_request_logs)
}

/// Convenience wrapper: build base + finalize in one call (used by tests).
#[cfg(not(feature = "push-notifications"))]
pub fn build_gateway_app(
    state: Arc<GatewayState>,
    methods: Arc<MethodRegistry>,
    http_request_logs: bool,
    webauthn_registry: Option<SharedWebAuthnRegistry>,
) -> Router {
    let (router, app_state) = build_gateway_base(state, methods, webauthn_registry);
    finalize_gateway_app(router, app_state, http_request_logs)
}
/// A fully wired gateway (app router + shared state), ready to be served.
///
/// Created by [`prepare_gateway`]. Callers bind their own TCP listener and
/// feed `app` to `axum::serve` (or an equivalent). Background tasks (metrics,
/// MCP health, cron, etc.) are already spawned on the current tokio runtime.
pub struct PreparedGateway {
    /// The composed application router.
    pub app: Router,
    /// Shared gateway state (sessions, services, config, etc.).
    pub state: Arc<GatewayState>,
    /// The port the gateway was configured for.
    pub port: u16,
    /// Metadata collected during setup, used by [`start_gateway`] for the
    /// startup banner. Not relevant for bridge callers.
    pub(crate) banner: BannerMeta,
    /// Network audit buffer for real-time streaming (present when
    /// the `trusted-network` feature is enabled and the proxy is active).
    #[cfg(feature = "trusted-network")]
    pub audit_buffer: Option<moltis_gateway::network_audit::NetworkAuditBuffer>,
}

/// Internal metadata for the startup banner printed by [`start_gateway`].
pub struct BannerMeta {
    pub provider_summary: String,
    pub mcp_configured_count: usize,
    pub method_count: usize,
    pub sandbox_backend_name: String,
    pub data_dir: PathBuf,
    pub openclaw_status: String,
    pub setup_code_display: Option<String>,
    pub webauthn_registry: Option<SharedWebAuthnRegistry>,
    pub browser_for_lifecycle: Arc<dyn moltis_gateway::services::BrowserService>,
    pub config: moltis_config::schema::MoltisConfig,
    #[cfg(feature = "tailscale")]
    pub tailscale_mode: TailscaleMode,
    #[cfg(feature = "tailscale")]
    pub tailscale_reset_on_exit: bool,
}

/// Prepare the full gateway: load config, run migrations, wire services,
/// spawn background tasks, and return the composed axum application.
///
/// This is the HTTP layer on top of [`prepare_gateway_core`]. The swift-bridge
/// calls this directly and manages its own TCP listener + graceful shutdown.
///
/// `extra_routes` is an optional callback that returns additional routes
/// (e.g. the web-UI) to merge before finalization.
#[allow(clippy::expect_used)]
pub async fn prepare_gateway(
    bind: &str,
    port: u16,
    no_tls: bool,
    log_buffer: Option<moltis_gateway::logs::LogBuffer>,
    config_dir: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    #[cfg(feature = "tailscale")] tailscale_opts: Option<TailscaleOpts>,
    extra_routes: Option<RouteEnhancer>,
    session_event_bus: Option<SessionEventBus>,
) -> anyhow::Result<PreparedGateway> {
    let core = prepare_gateway_core(
        bind,
        port,
        no_tls,
        log_buffer,
        config_dir,
        data_dir,
        #[cfg(feature = "tailscale")]
        tailscale_opts,
        session_event_bus,
    )
    .await?;

    let PreparedGatewayCore {
        state,
        methods,
        webauthn_registry,
        msteams_webhook_plugin,
        #[cfg(feature = "slack")]
        slack_webhook_plugin,
        #[cfg(feature = "push-notifications")]
        push_service,
        #[cfg(feature = "trusted-network")]
            audit_buffer: audit_buffer_for_broadcast,
        sandbox_router,
        browser_for_lifecycle,
        cron_service,
        log_buffer,
        config,
        data_dir,
        provider_summary,
        mcp_configured_count,
        openclaw_status: openclaw_startup_status,
        setup_code_display,
        port,
        tls_enabled: tls_enabled_for_gateway,
        #[cfg(feature = "tailscale")]
        tailscale_mode,
        #[cfg(feature = "tailscale")]
        tailscale_reset_on_exit,
    } = core;

    #[cfg(feature = "push-notifications")]
    let (router, app_state) = build_gateway_base(
        Arc::clone(&state),
        Arc::clone(&methods),
        push_service,
        webauthn_registry.clone(),
    );
    #[cfg(not(feature = "push-notifications"))]
    let (router, app_state) = build_gateway_base(
        Arc::clone(&state),
        Arc::clone(&methods),
        webauthn_registry.clone(),
    );

    // Merge caller-provided routes (e.g. web-UI) before finalization.
    let router = if let Some(enhance) = extra_routes {
        router.merge(enhance())
    } else {
        router
    };

    let mut app = finalize_gateway_app(router, app_state, config.server.http_request_logs);

    {
        let teams_plugin_for_webhook = Arc::clone(&msteams_webhook_plugin);
        let state_for_teams_webhook = Arc::clone(&state);
        app = app.route(
            "/api/channels/msteams/{account_id}/webhook",
            axum::routing::post(
                move |axum::extract::Path(account_id): axum::extract::Path<String>,
                      axum::extract::Query(query): axum::extract::Query<HashMap<String, String>>,
                      headers: axum::http::HeaderMap,
                      body: axum::body::Bytes| {
                    let teams_plugin = Arc::clone(&teams_plugin_for_webhook);
                    let gw_state = Arc::clone(&state_for_teams_webhook);
                    async move {
                        // Get the verifier from the plugin.
                        let verifier = {
                            let plugin = teams_plugin.read().await;
                            plugin.channel_webhook_verifier(&account_id)
                        };
                        let Some(verifier) = verifier else {
                            return (
                                StatusCode::NOT_FOUND,
                                Json(serde_json::json!({ "ok": false, "error": "unknown Teams account" })),
                            )
                                .into_response();
                        };

                        // Inject query-param secret as header for the verifier.
                        let mut merged_headers = headers;
                        if let Some(secret) = query.get("secret")
                            && let Ok(val) = secret.parse()
                        {
                            merged_headers.insert("x-moltis-webhook-secret", val);
                        }

                        // Run the middleware pipeline.
                        match moltis_gateway::channel_webhook_middleware::channel_webhook_gate(
                            verifier.as_ref(),
                            &gw_state.channel_webhook_dedup,
                            &gw_state.channel_webhook_rate_limiter,
                            &account_id,
                            &merged_headers,
                            &body,
                        ) {
                            Err(rejection) => {
                                moltis_gateway::channel_webhook_middleware::rejection_into_response(rejection)
                            },
                            Ok((_, moltis_channels::ChannelWebhookDedupeResult::Duplicate)) => (
                                StatusCode::OK,
                                Json(serde_json::json!({ "ok": true, "deduplicated": true })),
                            )
                                .into_response(),
                            Ok((verified, moltis_channels::ChannelWebhookDedupeResult::New)) => {
                                // Parse verified body and dispatch.
                                let payload: serde_json::Value =
                                    match serde_json::from_slice(&verified.body) {
                                        Ok(v) => v,
                                        Err(e) => {
                                            return (
                                                StatusCode::BAD_REQUEST,
                                                Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
                                            )
                                                .into_response();
                                        },
                                    };
                                let result = {
                                    let plugin = teams_plugin.read().await;
                                    plugin
                                        .ingest_verified_activity(&account_id, payload)
                                        .await
                                };
                                match result {
                                    Ok(()) => (
                                        StatusCode::OK,
                                        Json(serde_json::json!({ "ok": true })),
                                    )
                                        .into_response(),
                                    Err(e) => {
                                        let msg = e.to_string();
                                        if msg.contains("unknown Teams account") {
                                            (
                                                StatusCode::NOT_FOUND,
                                                Json(serde_json::json!({ "ok": false, "error": msg })),
                                            )
                                                .into_response()
                                        } else {
                                            (
                                                StatusCode::BAD_REQUEST,
                                                Json(serde_json::json!({ "ok": false, "error": msg })),
                                            )
                                                .into_response()
                                        }
                                    },
                                }
                            },
                        }
                    }
                },
            ),
        );
    }

    #[cfg(feature = "slack")]
    {
        // Slack Events API webhook — receives event callbacks.
        let slack_events_plugin = Arc::clone(&slack_webhook_plugin);
        let state_for_slack_events = Arc::clone(&state);
        app = app.route(
            "/api/channels/slack/{account_id}/events",
            axum::routing::post(
                move |axum::extract::Path(account_id): axum::extract::Path<String>,
                      headers: axum::http::HeaderMap,
                      body: axum::body::Bytes| {
                    let plugin = Arc::clone(&slack_events_plugin);
                    let gw_state = Arc::clone(&state_for_slack_events);
                    async move {
                        // Get the verifier from the plugin.
                        let verifier = {
                            let p = plugin.read().await;
                            p.channel_webhook_verifier(&account_id)
                        };
                        let Some(verifier) = verifier else {
                            return (
                                StatusCode::NOT_FOUND,
                                Json(serde_json::json!({ "ok": false, "error": "unknown Slack account" })),
                            )
                                .into_response();
                        };

                        // Run the middleware pipeline.
                        match moltis_gateway::channel_webhook_middleware::channel_webhook_gate(
                            verifier.as_ref(),
                            &gw_state.channel_webhook_dedup,
                            &gw_state.channel_webhook_rate_limiter,
                            &account_id,
                            &headers,
                            &body,
                        ) {
                            Err(rejection) => {
                                moltis_gateway::channel_webhook_middleware::rejection_into_response(rejection)
                            },
                            Ok((_, moltis_channels::ChannelWebhookDedupeResult::Duplicate)) => (
                                StatusCode::OK,
                                Json(serde_json::json!({ "ok": true, "deduplicated": true })),
                            )
                                .into_response(),
                            Ok((verified, moltis_channels::ChannelWebhookDedupeResult::New)) => {
                                // Dispatch to Slack plugin with verified body.
                                let result = {
                                    let p = plugin.read().await;
                                    p.ingest_verified_webhook(&account_id, &verified.body)
                                        .await
                                };
                                match result {
                                    Ok(Some(challenge)) => (
                                        StatusCode::OK,
                                        Json(serde_json::json!({ "challenge": challenge })),
                                    )
                                        .into_response(),
                                    Ok(None) => (
                                        StatusCode::OK,
                                        Json(serde_json::json!({ "ok": true })),
                                    )
                                        .into_response(),
                                    Err(e) => {
                                        let msg = e.to_string();
                                        if msg.contains("unknown") {
                                            (
                                                StatusCode::NOT_FOUND,
                                                Json(serde_json::json!({ "ok": false, "error": msg })),
                                            )
                                                .into_response()
                                        } else {
                                            (
                                                StatusCode::BAD_REQUEST,
                                                Json(serde_json::json!({ "ok": false, "error": msg })),
                                            )
                                                .into_response()
                                        }
                                    },
                                }
                            },
                        }
                    }
                },
            ),
        );

        // Slack interaction webhook — receives button click payloads.
        let slack_interact_plugin = Arc::clone(&slack_webhook_plugin);
        let state_for_slack_interact = Arc::clone(&state);
        app = app.route(
            "/api/channels/slack/{account_id}/interactions",
            axum::routing::post(
                move |axum::extract::Path(account_id): axum::extract::Path<String>,
                      headers: axum::http::HeaderMap,
                      body: axum::body::Bytes| {
                    let plugin = Arc::clone(&slack_interact_plugin);
                    let gw_state = Arc::clone(&state_for_slack_interact);
                    async move {
                        // Get the verifier from the plugin.
                        let verifier = {
                            let p = plugin.read().await;
                            p.channel_webhook_verifier(&account_id)
                        };
                        let Some(verifier) = verifier else {
                            return (
                                StatusCode::NOT_FOUND,
                                Json(serde_json::json!({ "ok": false, "error": "unknown Slack account" })),
                            )
                                .into_response();
                        };

                        // Run the middleware pipeline.
                        match moltis_gateway::channel_webhook_middleware::channel_webhook_gate(
                            verifier.as_ref(),
                            &gw_state.channel_webhook_dedup,
                            &gw_state.channel_webhook_rate_limiter,
                            &account_id,
                            &headers,
                            &body,
                        ) {
                            Err(rejection) => {
                                moltis_gateway::channel_webhook_middleware::rejection_into_response(rejection)
                            },
                            Ok((_, moltis_channels::ChannelWebhookDedupeResult::Duplicate)) => (
                                StatusCode::OK,
                                Json(serde_json::json!({ "ok": true, "deduplicated": true })),
                            )
                                .into_response(),
                            Ok((verified, moltis_channels::ChannelWebhookDedupeResult::New)) => {
                                // Dispatch to Slack plugin with verified body.
                                let result = {
                                    let p = plugin.read().await;
                                    p.ingest_verified_interaction_webhook(
                                        &account_id,
                                        &verified.body,
                                    )
                                    .await
                                };
                                match result {
                                    Ok(()) => (
                                        StatusCode::OK,
                                        Json(serde_json::json!({ "ok": true })),
                                    )
                                        .into_response(),
                                    Err(e) => (
                                        StatusCode::BAD_REQUEST,
                                        Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
                                    )
                                        .into_response(),
                                }
                            },
                        }
                    }
                },
            ),
        );
    }

    // Resolve TLS configuration (only when compiled with the `tls` feature).
    let tls_active = tls_enabled_for_gateway;

    #[cfg(feature = "tls")]
    if tls_active {
        let tls_config = &config.tls;
        let (ca_path, _cert_path, _key_path) = if let (Some(cert_str), Some(key_str)) =
            (&tls_config.cert_path, &tls_config.key_path)
        {
            // User-provided certs.
            let cert = PathBuf::from(cert_str);
            let key = PathBuf::from(key_str);
            let ca = tls_config.ca_cert_path.as_ref().map(PathBuf::from);
            (ca, cert, key)
        } else if tls_config.auto_generate {
            // Auto-generate certificates.
            let mgr = moltis_gateway::tls::FsCertManager::new()?;
            let (ca, cert, key) = mgr.ensure_certs()?;
            (Some(ca), cert, key)
        } else {
            anyhow::bail!(
                "TLS is enabled but no certificates configured and auto_generate is false"
            );
        };

        // Add /certs/ca.pem route to the main HTTPS app if we have a CA cert.
        if let Some(ref ca) = ca_path {
            let ca_bytes = Arc::new(std::fs::read(ca)?);
            let ca_clone = Arc::clone(&ca_bytes);
            app = app.route(
                "/certs/ca.pem",
                get(move || {
                    let data = Arc::clone(&ca_clone);
                    async move {
                        (
                            [
                                ("content-type", "application/x-pem-file"),
                                (
                                    "content-disposition",
                                    "attachment; filename=\"moltis-ca.pem\"",
                                ),
                            ],
                            data.as_ref().clone(),
                        )
                    }
                }),
            );
        }
    }

    // NOTE: the startup banner and GatewayStart hook dispatch are handled
    // by start_gateway (the CLI entry point) after prepare_gateway returns.
    // prepare_gateway only spawns background tasks and returns PreparedGateway.

    // Spawn periodic browser cleanup task (every 30s, removes idle instances).
    {
        let browser_for_cleanup = Arc::clone(&browser_for_lifecycle);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            interval.tick().await; // skip immediate first tick
            loop {
                interval.tick().await;
                browser_for_cleanup.cleanup_idle().await;
            }
        });
    }

    // Spawn tick timer.
    let tick_state = Arc::clone(&state);
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_millis(TICK_INTERVAL_MS));
        let mut sys = sysinfo::System::new();
        let pid = sysinfo::get_current_pid().ok();
        loop {
            interval.tick().await;
            sys.refresh_memory();
            if let Some(pid) = pid {
                sys.refresh_processes_specifics(
                    sysinfo::ProcessesToUpdate::Some(&[pid]),
                    false,
                    sysinfo::ProcessRefreshKind::nothing().with_memory(),
                );
            }
            let process_mem = pid
                .and_then(|p| sys.process(p))
                .map(|p| p.memory())
                .unwrap_or(0);
            let total = sys.total_memory();
            let available = match sys.available_memory() {
                0 => total.saturating_sub(sys.used_memory()),
                v => v,
            };
            broadcast_tick(&tick_state, process_mem, available, total).await;
        }
    });

    // Spawn session event → WebSocket forwarder.
    // Events published by the swift-bridge (or any other bus producer) are
    // relayed to all connected WebSocket clients as `"session"` events.
    {
        let ws_state = Arc::clone(&state);
        let mut rx = state.session_event_bus.subscribe();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        let (kind, session_key) = match &event {
                            SessionEvent::Created { session_key } => {
                                ("created", session_key.as_str())
                            },
                            SessionEvent::Deleted { session_key } => {
                                ("deleted", session_key.as_str())
                            },
                            SessionEvent::Patched { session_key } => {
                                ("patched", session_key.as_str())
                            },
                        };
                        broadcast(
                            &ws_state,
                            "session",
                            serde_json::json!({
                                "kind": kind,
                                "sessionKey": session_key,
                            }),
                            BroadcastOpts {
                                drop_if_slow: true,
                                ..Default::default()
                            },
                        )
                        .await;
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("session event WS forwarder lagged, skipped {n} events");
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    // Spawn periodic update check against the releases manifest.
    let update_state = Arc::clone(&state);
    let releases_url = resolve_releases_url(config.server.update_releases_url.as_deref());
    tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .user_agent(format!("moltis-gateway/{}", update_state.version))
            .timeout(std::time::Duration::from_secs(12))
            .build()
        {
            Ok(client) => client,
            Err(e) => {
                warn!("failed to initialize update checker HTTP client: {e}");
                return;
            },
        };

        let mut interval = tokio::time::interval(UPDATE_CHECK_INTERVAL);
        loop {
            interval.tick().await;
            let next = fetch_update_availability(&client, &releases_url, &update_state.version).await;
            let changed = {
                let mut inner = update_state.inner.write().await;
                let update = &mut inner.update;
                if *update == next {
                    false
                } else {
                    *update = next.clone();
                    true
                }
            };
            if changed && let Ok(payload) = serde_json::to_value(&next) {
                broadcast(&update_state, "update.available", payload, BroadcastOpts {
                    drop_if_slow: true,
                    ..Default::default()
                })
                .await;
            }
        }
    });

    // Spawn metrics history collection and broadcast task (every 10 seconds).
    #[cfg(feature = "metrics")]
    {
        let metrics_state = Arc::clone(&state);
        let server_start = std::time::Instant::now();
        tokio::spawn(async move {
            enum MetricsPersistJob {
                Save(moltis_gateway::state::MetricsHistoryPoint),
                CleanupBefore(u64),
            }

            // Load history from persistent store on startup.
            if let Some(ref store) = metrics_state.metrics_store {
                let max_points = metrics_state.inner.read().await.metrics_history.capacity();
                // Load enough history to fill the in-memory buffer.
                let window_secs = max_points as u64 * 10; // 10-second intervals
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let since = now_ms.saturating_sub(window_secs * 1000);
                match store.load_history(since, max_points).await {
                    Ok(points) => {
                        let mut inner = metrics_state.inner.write().await;
                        for point in points {
                            inner.metrics_history.push(point);
                        }
                        let loaded = inner.metrics_history.iter().count();
                        drop(inner);
                        info!("Loaded {loaded} historical metrics points from store");
                    },
                    Err(e) => {
                        warn!("Failed to load metrics history: {e}");
                    },
                }
            }

            // Serialize all metrics DB writes through one background writer task.
            let metrics_persist_tx = metrics_state.metrics_store.as_ref().map(|store| {
                let store = Arc::clone(store);
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<MetricsPersistJob>();
                tokio::spawn(async move {
                    while let Some(job) = rx.recv().await {
                        match job {
                            MetricsPersistJob::Save(point) => {
                                if let Err(e) = store.save_point(&point).await {
                                    warn!("Failed to persist metrics point: {e}");
                                }
                            },
                            MetricsPersistJob::CleanupBefore(cutoff) => {
                                match store.cleanup_before(cutoff).await {
                                    Ok(deleted) if deleted > 0 => {
                                        info!("Cleaned up {} old metrics points", deleted);
                                    },
                                    Err(e) => {
                                        warn!("Failed to cleanup old metrics: {e}");
                                    },
                                    _ => {},
                                }
                            },
                        }
                    }
                });
                tx
            });

            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            let mut cleanup_counter = 0u32;
            loop {
                interval.tick().await;
                if let Some(ref handle) = metrics_state.metrics_handle {
                    // Update gauges that are derived from server state, not events.
                    moltis_metrics::gauge!(moltis_metrics::system::UPTIME_SECONDS)
                        .set(server_start.elapsed().as_secs_f64());
                    let session_count =
                        metrics_state.inner.read().await.active_sessions.len() as f64;
                    moltis_metrics::gauge!(moltis_metrics::session::ACTIVE).set(session_count);

                    let prometheus_text = handle.render();
                    let snapshot =
                        moltis_metrics::MetricsSnapshot::from_prometheus_text(&prometheus_text);
                    // Convert per-provider metrics to history format.
                    let by_provider = snapshot
                        .categories
                        .llm
                        .by_provider
                        .iter()
                        .map(|(name, metrics)| {
                            (name.clone(), moltis_metrics::ProviderTokens {
                                input_tokens: metrics.input_tokens,
                                output_tokens: metrics.output_tokens,
                                completions: metrics.completions,
                                errors: metrics.errors,
                            })
                        })
                        .collect();

                    let point = moltis_gateway::state::MetricsHistoryPoint {
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64,
                        llm_completions: snapshot.categories.llm.completions_total,
                        llm_input_tokens: snapshot.categories.llm.input_tokens,
                        llm_output_tokens: snapshot.categories.llm.output_tokens,
                        llm_errors: snapshot.categories.llm.errors,
                        by_provider,
                        http_requests: snapshot.categories.http.total,
                        http_active: snapshot.categories.http.active,
                        ws_connections: snapshot.categories.websocket.total,
                        ws_active: snapshot.categories.websocket.active,
                        tool_executions: snapshot.categories.tools.total,
                        tool_errors: snapshot.categories.tools.errors,
                        mcp_calls: snapshot.categories.mcp.total,
                        active_sessions: snapshot.categories.system.active_sessions,
                    };

                    // Push to in-memory history.
                    metrics_state
                        .inner
                        .write()
                        .await
                        .metrics_history
                        .push(point.clone());

                    // Persist via the dedicated writer, without stalling collection.
                    if let Some(tx) = metrics_persist_tx.as_ref()
                        && tx.send(MetricsPersistJob::Save(point.clone())).is_err()
                    {
                        warn!("metrics persistence writer task is unavailable");
                    }

                    // Broadcast metrics update to all connected clients.
                    let payload = moltis_gateway::state::MetricsUpdatePayload { snapshot, point };
                    if let Ok(payload_json) = serde_json::to_value(&payload) {
                        broadcast(
                            &metrics_state,
                            "metrics.update",
                            payload_json,
                            BroadcastOpts {
                                drop_if_slow: true,
                                ..Default::default()
                            },
                        )
                        .await;
                    }

                    // Cleanup old data once per hour (360 ticks at 10s interval).
                    cleanup_counter += 1;
                    if cleanup_counter >= 360 {
                        cleanup_counter = 0;
                        if let Some(tx) = metrics_persist_tx.as_ref() {
                            // Keep 7 days of history.
                            let cutoff = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as u64
                                - (7 * 24 * 60 * 60 * 1000);
                            if tx.send(MetricsPersistJob::CleanupBefore(cutoff)).is_err() {
                                warn!("metrics persistence writer task is unavailable");
                            }
                        }
                    }
                }
            }
        });
    }

    // Spawn sandbox event broadcast task: forwards sandbox lifecycle events to WS clients.
    {
        let event_state = Arc::clone(&state);
        let mut event_rx = sandbox_router.subscribe_events();
        tokio::spawn(async move {
            loop {
                match event_rx.recv().await {
                    Ok(event) => {
                        let (event_name, payload) = match event {
                            moltis_tools::sandbox::SandboxEvent::Preparing {
                                session_key,
                                backend,
                                image,
                            } => (
                                "sandbox.prepare",
                                serde_json::json!({
                                    "phase": "start",
                                    "session_key": session_key,
                                    "backend": backend,
                                    "image": image,
                                }),
                            ),
                            moltis_tools::sandbox::SandboxEvent::Prepared {
                                session_key,
                                backend,
                                image,
                            } => (
                                "sandbox.prepare",
                                serde_json::json!({
                                    "phase": "done",
                                    "session_key": session_key,
                                    "backend": backend,
                                    "image": image,
                                }),
                            ),
                            moltis_tools::sandbox::SandboxEvent::PrepareFailed {
                                session_key,
                                backend,
                                image,
                                error,
                            } => (
                                "sandbox.prepare",
                                serde_json::json!({
                                    "phase": "error",
                                    "session_key": session_key,
                                    "backend": backend,
                                    "image": image,
                                    "error": error,
                                }),
                            ),
                            moltis_tools::sandbox::SandboxEvent::Provisioning {
                                container,
                                packages,
                            } => (
                                "sandbox.image.provision",
                                serde_json::json!({
                                    "phase": "start",
                                    "container": container,
                                    "packages": packages,
                                }),
                            ),
                            moltis_tools::sandbox::SandboxEvent::Provisioned { container } => (
                                "sandbox.image.provision",
                                serde_json::json!({
                                    "phase": "done",
                                    "container": container,
                                }),
                            ),
                            moltis_tools::sandbox::SandboxEvent::ProvisionFailed {
                                container,
                                error,
                            } => (
                                "sandbox.image.provision",
                                serde_json::json!({
                                    "phase": "error",
                                    "container": container,
                                    "error": error,
                                }),
                            ),
                        };
                        broadcast(&event_state, event_name, payload, BroadcastOpts {
                            drop_if_slow: true,
                            ..Default::default()
                        })
                        .await;
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        });
    }

    // Spawn network audit broadcast task: forwards audit entries to WS clients.
    #[cfg(feature = "trusted-network")]
    if let Some(ref audit_buf) = audit_buffer_for_broadcast {
        let audit_state = Arc::clone(&state);
        let mut audit_rx = audit_buf.subscribe();
        tokio::spawn(async move {
            loop {
                match audit_rx.recv().await {
                    Ok(entry) => {
                        if let Ok(payload) = serde_json::to_value(&entry) {
                            broadcast(
                                &audit_state,
                                "network.audit.entry",
                                payload,
                                BroadcastOpts {
                                    drop_if_slow: true,
                                    ..Default::default()
                                },
                            )
                            .await;
                        }
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        });
    }

    // Spawn log broadcast task: forwards captured tracing events to WS clients.
    if let Some(buf) = log_buffer {
        let log_state = Arc::clone(&state);
        tokio::spawn(async move {
            let mut rx = buf.subscribe();
            loop {
                match rx.recv().await {
                    Ok(entry) => {
                        if let Ok(payload) = serde_json::to_value(&entry) {
                            broadcast(&log_state, "logs.entry", payload, BroadcastOpts {
                                drop_if_slow: true,
                                ..Default::default()
                            })
                            .await;
                        }
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        });
    }

    // Start the cron scheduler (loads persisted jobs, arms the timer).
    if let Err(e) = cron_service.start().await {
        tracing::warn!("failed to start cron scheduler: {e}");
    }

    // Upsert the built-in heartbeat job from config.
    // Use a fixed ID so run history persists across restarts.
    {
        use moltis_cron::{
            heartbeat::{
                DEFAULT_INTERVAL_MS, HeartbeatPromptSource, parse_interval_ms,
                resolve_heartbeat_prompt,
            },
            types::{CronJobCreate, CronJobPatch, CronPayload, CronSchedule, SessionTarget},
        };
        const HEARTBEAT_JOB_ID: &str = "__heartbeat__";

        let hb = &config.heartbeat;
        let interval_ms = parse_interval_ms(&hb.every).unwrap_or(DEFAULT_INTERVAL_MS);
        let heartbeat_md = moltis_config::load_heartbeat_md();
        let (prompt, prompt_source) =
            resolve_heartbeat_prompt(hb.prompt.as_deref(), heartbeat_md.as_deref());
        if prompt_source == HeartbeatPromptSource::HeartbeatMd {
            tracing::info!("loaded heartbeat prompt from HEARTBEAT.md");
        }
        if hb.prompt.as_deref().is_some_and(|p| !p.trim().is_empty())
            && heartbeat_md
                .as_deref()
                .is_some_and(|p| !p.trim().is_empty())
            && prompt_source == HeartbeatPromptSource::Config
        {
            tracing::warn!(
                "heartbeat prompt source conflict: config heartbeat.prompt overrides HEARTBEAT.md"
            );
        }

        // Check if heartbeat job already exists.
        let existing = cron_service.list().await;
        let existing_job = existing.iter().find(|j| j.id == HEARTBEAT_JOB_ID);

        // Skip heartbeat when there is no meaningful prompt (no config prompt,
        // no HEARTBEAT.md content). The built-in default prompt is generic and
        // wastes LLM calls when the user hasn't configured anything.
        let has_prompt = prompt_source != HeartbeatPromptSource::Default;

        if hb.enabled && has_prompt {
            if existing_job.is_some() {
                // Update existing job to match config.
                let patch = CronJobPatch {
                    schedule: Some(CronSchedule::Every {
                        every_ms: interval_ms,
                        anchor_ms: None,
                    }),
                    payload: Some(CronPayload::AgentTurn {
                        message: prompt,
                        model: hb.model.clone(),
                        timeout_secs: None,
                        deliver: hb.deliver,
                        channel: hb.channel.clone(),
                        to: hb.to.clone(),
                    }),
                    enabled: Some(true),
                    sandbox: Some(moltis_cron::types::CronSandboxConfig {
                        enabled: hb.sandbox_enabled,
                        image: hb.sandbox_image.clone(),
                    }),
                    ..Default::default()
                };
                match cron_service.update(HEARTBEAT_JOB_ID, patch).await {
                    Ok(job) => tracing::info!(id = %job.id, "heartbeat job updated"),
                    Err(e) => tracing::warn!("failed to update heartbeat job: {e}"),
                }
            } else {
                // Create new job with fixed ID.
                let create = CronJobCreate {
                    id: Some(HEARTBEAT_JOB_ID.into()),
                    name: "__heartbeat__".into(),
                    schedule: CronSchedule::Every {
                        every_ms: interval_ms,
                        anchor_ms: None,
                    },
                    payload: CronPayload::AgentTurn {
                        message: prompt,
                        model: hb.model.clone(),
                        timeout_secs: None,
                        deliver: hb.deliver,
                        channel: hb.channel.clone(),
                        to: hb.to.clone(),
                    },
                    session_target: SessionTarget::Named("heartbeat".into()),
                    delete_after_run: false,
                    enabled: true,
                    system: true,
                    sandbox: moltis_cron::types::CronSandboxConfig {
                        enabled: hb.sandbox_enabled,
                        image: hb.sandbox_image.clone(),
                    },
                    wake_mode: moltis_cron::types::CronWakeMode::default(),
                };
                match cron_service.add(create).await {
                    Ok(job) => tracing::info!(id = %job.id, "heartbeat job created"),
                    Err(e) => tracing::warn!("failed to create heartbeat job: {e}"),
                }
            }
        } else if existing_job.is_some() {
            // Heartbeat is disabled or has no prompt content — remove the job.
            let _ = cron_service.remove(HEARTBEAT_JOB_ID).await;
            if !hb.enabled {
                tracing::info!("heartbeat job removed (disabled)");
            } else {
                tracing::info!("heartbeat job removed (no prompt configured)");
            }
        } else if hb.enabled && !has_prompt {
            tracing::info!("heartbeat skipped: no prompt in config and HEARTBEAT.md is empty");
        }
    }

    Ok(PreparedGateway {
        app,
        state: Arc::clone(&state),
        port,
        banner: BannerMeta {
            provider_summary,
            mcp_configured_count,
            method_count: methods.method_names().len(),
            sandbox_backend_name: sandbox_router.backend_name().to_owned(),
            data_dir,
            openclaw_status: openclaw_startup_status,
            setup_code_display,
            webauthn_registry,
            browser_for_lifecycle,
            config,
            #[cfg(feature = "tailscale")]
            tailscale_mode,
            #[cfg(feature = "tailscale")]
            tailscale_reset_on_exit,
        },
        #[cfg(feature = "trusted-network")]
        audit_buffer: audit_buffer_for_broadcast,
    })
}

/// Prepare the full gateway for embedded callers (for example swift-bridge)
/// using a feature-stable argument list.
///
/// This wrapper intentionally hides `tailscale_opts`, which only exists when
/// the `tailscale` feature is enabled on `moltis-gateway`.
#[allow(clippy::expect_used)]
pub async fn prepare_gateway_embedded(
    bind: &str,
    port: u16,
    no_tls: bool,
    log_buffer: Option<moltis_gateway::logs::LogBuffer>,
    config_dir: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    extra_routes: Option<RouteEnhancer>,
    session_event_bus: Option<SessionEventBus>,
) -> anyhow::Result<PreparedGateway> {
    prepare_gateway(
        bind,
        port,
        no_tls,
        log_buffer,
        config_dir,
        data_dir,
        #[cfg(feature = "tailscale")]
        None,
        extra_routes,
        session_event_bus,
    )
    .await
}

/// Alias for [`prepare_gateway_embedded`] used by swift-bridge consumers.
pub use prepare_gateway_embedded as prepare_httpd_embedded;

/// Re-export `openclaw_detected_for_ui` from gateway for web-UI templates.
pub use moltis_gateway::server::openclaw_detected_for_ui;

/// Start the gateway HTTP + WebSocket server.
///
/// Thin wrapper around [`prepare_gateway`] that adds the startup banner,
/// ctrl-c handler, TLS termination, and blocks on `axum::serve`.
#[allow(clippy::expect_used)]
pub async fn start_gateway(
    bind: &str,
    port: u16,
    no_tls: bool,
    log_buffer: Option<moltis_gateway::logs::LogBuffer>,
    config_dir: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    #[cfg(feature = "tailscale")] tailscale_opts: Option<TailscaleOpts>,
    extra_routes: Option<RouteEnhancer>,
) -> anyhow::Result<()> {
    let prepared = prepare_gateway(
        bind,
        port,
        no_tls,
        log_buffer,
        config_dir,
        data_dir,
        #[cfg(feature = "tailscale")]
        tailscale_opts,
        extra_routes,
        None, // session_event_bus — CLI creates its own
    )
    .await?;

    let state = &prepared.state;
    let banner = &prepared.banner;
    #[cfg_attr(not(feature = "tls"), allow(unused_variables))]
    let config = &banner.config;

    let addr: SocketAddr = format!("{bind}:{port}").parse()?;

    #[cfg_attr(not(feature = "tls"), allow(unused_variables))]
    let is_localhost =
        matches!(bind, "127.0.0.1" | "::1" | "localhost") || bind.ends_with(".localhost");

    // Register the gateway as a Bonjour/mDNS service so LAN clients can
    // discover it without typing the URL manually.
    #[cfg(feature = "mdns")]
    let _mdns_daemon = {
        let host = hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "moltis".to_string());
        let instance = format!("Moltis on {host}");
        match moltis_gateway::mdns::register(
            &instance,
            port,
            env!("CARGO_PKG_VERSION"),
            Some(&instance_slug(config)),
        ) {
            Ok(daemon) => Some(daemon),
            Err(e) => {
                tracing::warn!("mDNS registration failed: {e}");
                None
            },
        }
    };

    // Resolve TLS configuration (only when compiled with the `tls` feature).
    #[cfg(feature = "tls")]
    let tls_active = config.tls.enabled;
    #[cfg(not(feature = "tls"))]
    let tls_active = false;

    #[cfg(feature = "tls")]
    let mut ca_cert_path: Option<PathBuf> = None;
    #[cfg(feature = "tls")]
    let mut rustls_config: Option<rustls::ServerConfig> = None;

    let app = prepared.app;

    #[cfg(feature = "tls")]
    if tls_active {
        let tls_config = &config.tls;
        let (ca_path, cert_path, key_path) = if let (Some(cert_str), Some(key_str)) =
            (&tls_config.cert_path, &tls_config.key_path)
        {
            // User-provided certs.
            let cert = PathBuf::from(cert_str);
            let key = PathBuf::from(key_str);
            let ca = tls_config.ca_cert_path.as_ref().map(PathBuf::from);
            (ca, cert, key)
        } else if tls_config.auto_generate {
            // Auto-generate certificates.
            let mgr = moltis_gateway::tls::FsCertManager::new()?;
            let (ca, cert, key) = mgr.ensure_certs()?;
            (Some(ca), cert, key)
        } else {
            anyhow::bail!(
                "TLS is enabled but no certificates configured and auto_generate is false"
            );
        };

        ca_cert_path = ca_path.clone();

        let mgr = moltis_gateway::tls::FsCertManager::new()?;
        rustls_config = Some(mgr.build_rustls_config(&cert_path, &key_path)?);
        // Note: /certs/ca.pem route is already registered by prepare_gateway.
    }

    // Count enabled skills and repos for startup banner.
    let (skill_count, repo_count) = {
        use moltis_skills::discover::{FsSkillDiscoverer, SkillDiscoverer};
        let discoverer = FsSkillDiscoverer::new(FsSkillDiscoverer::default_paths());
        let sc = discoverer.discover().await.map(|s| s.len()).unwrap_or(0);
        let rc = moltis_skills::manifest::ManifestStore::default_path()
            .ok()
            .map(|p| {
                let store = moltis_skills::manifest::ManifestStore::new(p);
                store.load().map(|m| m.repos.len()).unwrap_or(0)
            })
            .unwrap_or(0);
        (sc, rc)
    };

    // Startup banner.
    let scheme = if tls_active {
        "https"
    } else {
        "http"
    };
    // When bound to an unspecified address (0.0.0.0 / ::), resolve the
    // machine's outbound IP so the printed URL is clickable.
    let display_ip = if addr.ip().is_unspecified() {
        resolve_outbound_ip(addr.ip().is_ipv6())
            .map(|ip| SocketAddr::new(ip, port))
            .unwrap_or(addr)
    } else {
        addr
    };
    // Use plain localhost for display URLs when bound to loopback with TLS.
    #[cfg(feature = "tls")]
    let display_host = if is_localhost && tls_active {
        format!("localhost:{port}")
    } else {
        display_ip.to_string()
    };
    #[cfg(not(feature = "tls"))]
    let display_host = display_ip.to_string();
    let passkey_origins = if let Some(registry) = banner.webauthn_registry.as_ref() {
        registry.read().await.get_all_origins()
    } else {
        Vec::new()
    };
    #[cfg_attr(not(feature = "tls"), allow(unused_mut))]
    let mut lines = vec![
        format!("moltis gateway v{}", state.version),
        format!(
            "protocol v{}, listening on {}://{} ({})",
            moltis_protocol::PROTOCOL_VERSION,
            scheme,
            display_host,
            if tls_active {
                "HTTP/2 + HTTP/1.1"
            } else {
                "HTTP/1.1"
            },
        ),
        startup_bind_line(addr),
        format!("{} methods registered", banner.method_count),
        format!("llm: {}", banner.provider_summary),
        format!(
            "skills: {} enabled, {} repo{}",
            skill_count,
            repo_count,
            if repo_count == 1 {
                ""
            } else {
                "s"
            }
        ),
        format!(
            "mcp: {} configured{}",
            banner.mcp_configured_count,
            if banner.mcp_configured_count > 0 {
                " (starting in background)"
            } else {
                ""
            }
        ),
        format!("sandbox: {} backend", banner.sandbox_backend_name),
        format!(
            "config: {}",
            moltis_config::find_or_default_config_path().display()
        ),
        format!("data: {}", banner.data_dir.display()),
        format!("openclaw: {}", banner.openclaw_status),
    ];
    lines.extend(startup_passkey_origin_lines(&passkey_origins));
    // Hint about Apple Container on macOS when using Docker or Podman.
    #[cfg(target_os = "macos")]
    if banner.sandbox_backend_name == "docker" || banner.sandbox_backend_name == "podman" {
        lines.push(
            "hint: install Apple Container for VM-isolated sandboxing (see docs/sandbox.md)".into(),
        );
    }
    // Warn when no sandbox backend is available.
    if banner.sandbox_backend_name == "none" {
        lines.push("⚠ no container runtime found; commands run on host".into());
    }
    // Display setup code if one was generated.
    if let Some(ref code) = banner.setup_code_display {
        lines.extend(startup_setup_code_lines(code));
    }
    #[cfg(feature = "tls")]
    if tls_active {
        if let Some(ref ca) = ca_cert_path {
            let http_port = config.tls.http_redirect_port.unwrap_or(port + 1);
            let ca_host = if is_localhost {
                "localhost"
            } else {
                bind
            };
            lines.push(format!(
                "CA cert: http://{}:{}/certs/ca.pem",
                ca_host, http_port
            ));
            lines.push(format!("  or: {}", ca.display()));
        }
        lines.push("run `moltis trust-ca` to remove browser warnings".into());
    }
    // Tailscale: enable serve/funnel and show in banner.
    #[cfg(feature = "tailscale")]
    {
        let tailscale_mode = banner.tailscale_mode;
        if tailscale_mode != TailscaleMode::Off {
            let manager = CliTailscaleManager::new();
            let ts_result = match tailscale_mode {
                TailscaleMode::Serve => manager.enable_serve(port, tls_active).await,
                TailscaleMode::Funnel => manager.enable_funnel(port, tls_active).await,
                TailscaleMode::Off => unreachable!(),
            };
            match ts_result {
                Ok(()) => {
                    if let Ok(Some(hostname)) = manager.hostname().await {
                        lines.push(format!("tailscale {tailscale_mode}: https://{hostname}"));
                    } else {
                        lines.push(format!("tailscale {tailscale_mode}: enabled"));
                    }
                },
                Err(e) => {
                    warn!("failed to enable tailscale {tailscale_mode}: {e}");
                    lines.push(format!("tailscale {tailscale_mode}: FAILED ({e})"));
                },
            }
        }
    }
    let width = lines.iter().map(|l| l.len()).max().unwrap_or(0) + 4;
    info!("┌{}┐", "─".repeat(width));
    for line in &lines {
        info!("│  {:<w$}│", line, w = width - 2);
    }
    info!("└{}┘", "─".repeat(width));

    // Dispatch GatewayStart hook.
    if let Some(ref hooks) = state.inner.read().await.hook_registry {
        let payload = moltis_common::hooks::HookPayload::GatewayStart {
            address: addr.to_string(),
        };
        if let Err(e) = hooks.dispatch(&payload).await {
            tracing::warn!("GatewayStart hook dispatch failed: {e}");
        }
    }

    // Spawn shutdown handler:
    // - unregister mDNS service (when configured)
    // - reset tailscale state on exit (when configured)
    // - give browser pool 5s to shut down gracefully
    // - force process exit to avoid hanging after ctrl-c
    {
        let browser_for_shutdown = Arc::clone(&banner.browser_for_lifecycle);
        #[cfg(feature = "tailscale")]
        let reset_tailscale_on_exit =
            banner.tailscale_mode != TailscaleMode::Off && banner.tailscale_reset_on_exit;
        #[cfg(feature = "tailscale")]
        let ts_mode = banner.tailscale_mode;
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_err() {
                return;
            }

            #[cfg(feature = "mdns")]
            if let Some(ref daemon) = _mdns_daemon {
                moltis_gateway::mdns::shutdown(daemon);
            }

            #[cfg(feature = "tailscale")]
            if reset_tailscale_on_exit {
                info!("shutting down tailscale {ts_mode}");
                let manager = CliTailscaleManager::new();
                if let Err(e) = manager.disable().await {
                    warn!("failed to reset tailscale on exit: {e}");
                }
            }

            let shutdown_grace = std::time::Duration::from_secs(5);
            info!(
                grace_secs = shutdown_grace.as_secs(),
                "shutting down browser pool"
            );
            if browser_for_shutdown
                .shutdown_with_grace(shutdown_grace)
                .await
            {
                info!(
                    grace_secs = shutdown_grace.as_secs(),
                    "browser pool shut down"
                );
            } else {
                warn!(
                    grace_secs = shutdown_grace.as_secs(),
                    "browser pool shutdown exceeded grace period, forcing process exit"
                );
            }

            std::process::exit(0);
        });
    }

    #[cfg(feature = "tls")]
    if tls_active {
        // Spawn HTTP redirect server on secondary port (serves CA cert download).
        if let Some(ref ca) = ca_cert_path {
            let http_port = config.tls.http_redirect_port.unwrap_or(port + 1);
            let bind_clone = bind.to_string();
            let ca_clone = ca.clone();
            tokio::spawn(async move {
                if let Err(e) = moltis_gateway::tls::start_http_redirect_server(
                    &bind_clone,
                    http_port,
                    port,
                    &ca_clone,
                )
                .await
                {
                    tracing::error!("HTTP redirect server failed: {e}");
                }
            });
        }

        // Run HTTPS server with automatic HTTP-to-HTTPS redirect on the same port.
        // Plain HTTP requests to this port get a 301 redirect instead of a TLS error.
        let tls_cfg = rustls_config.expect("rustls config must be set when TLS is active");
        let tcp_listener = tokio::net::TcpListener::bind(addr).await?;
        moltis_gateway::tls::serve_tls_with_http_redirect(
            tcp_listener,
            Arc::new(tls_cfg),
            app,
            port,
            bind,
        )
        .await?;
        return Ok(());
    }

    // Plain HTTP server (existing behavior, or TLS feature disabled).
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

// ── Handlers ─────────────────────────────────────────────────────────────────

async fn health_handler(State(state): State<AppState>) -> impl IntoResponse {
    let count = state.gateway.client_count().await;
    Json(serde_json::json!({
        "status": "ok",
        "version": state.gateway.version,
        "protocol": moltis_protocol::PROTOCOL_VERSION,
        "connections": count,
    }))
}

async fn ws_upgrade_handler(
    ws: WebSocketUpgrade,
    headers: axum::http::HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    // ── CSWSH protection ────────────────────────────────────────────────
    // Reject cross-origin WebSocket upgrades.  Browsers always send an
    // Origin header on cross-origin requests; non-browser clients (CLI,
    // SDKs) typically omit it — those are allowed through.
    if let Some(origin) = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok())
    {
        let host = websocket_origin_host(&headers, state.gateway.behind_proxy).unwrap_or_default();
        if !is_same_origin(origin, &host) {
            tracing::warn!(
                origin,
                host = %host,
                remote = %addr,
                "rejected cross-origin WebSocket upgrade"
            );
            return (
                StatusCode::FORBIDDEN,
                "cross-origin WebSocket connections are not allowed",
            )
                .into_response();
        }
    }

    let accept_language = headers
        .get(axum::http::header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    // Extract the real client IP (respecting proxy headers) and only keep it
    // when it resolves to a public address — private/loopback IPs are not useful
    // for the LLM to reason about locale or location.
    let remote_ip = extract_ws_client_ip(&headers, addr).filter(|ip| is_public_ip(ip));

    let is_local = is_local_connection(&headers, addr, state.gateway.behind_proxy);
    let header_authenticated =
        websocket_header_authenticated(&headers, state.gateway.credential_store.as_ref(), is_local)
            .await;
    ws.on_upgrade(move |socket| {
        handle_connection(
            socket,
            state.gateway,
            state.methods,
            addr,
            accept_language,
            remote_ip,
            header_authenticated,
            is_local,
        )
    })
    .into_response()
}

fn websocket_origin_host(headers: &axum::http::HeaderMap, behind_proxy: bool) -> Option<String> {
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned);
    if !behind_proxy {
        return host;
    }
    headers
        .get("x-forwarded-host")
        .and_then(|v| v.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or(host)
}

/// Dedicated host terminal WebSocket stream (`Settings > Terminal`).
/// Extract the client IP from proxy headers, falling back to the direct connection address.
fn extract_ws_client_ip(headers: &axum::http::HeaderMap, conn_addr: SocketAddr) -> Option<String> {
    // X-Forwarded-For (may contain multiple IPs — take the leftmost/client IP)
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok())
        && let Some(first_ip) = xff.split(',').next()
    {
        let ip = first_ip.trim();
        if !ip.is_empty() {
            return Some(ip.to_string());
        }
    }

    // X-Real-IP (common with nginx)
    if let Some(xri) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        let ip = xri.trim();
        if !ip.is_empty() {
            return Some(ip.to_string());
        }
    }

    // CF-Connecting-IP (Cloudflare)
    if let Some(cf_ip) = headers
        .get("cf-connecting-ip")
        .and_then(|v| v.to_str().ok())
    {
        let ip = cf_ip.trim();
        if !ip.is_empty() {
            return Some(ip.to_string());
        }
    }

    Some(conn_addr.ip().to_string())
}

/// Returns `true` if the IP string parses to a public (non-private, non-loopback) address.
fn is_public_ip(ip: &str) -> bool {
    use std::net::IpAddr;
    let Ok(addr) = ip.parse::<IpAddr>() else {
        return false;
    };
    match addr {
        IpAddr::V4(v4) => {
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                // 100.64.0.0/10 (CGNAT)
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64)
                // 192.0.0.0/24
                || (v4.octets()[0] == 192 && v4.octets()[1] == 0 && v4.octets()[2] == 0))
        },
        IpAddr::V6(v6) => {
            !(v6.is_loopback()
                || v6.is_unspecified()
                // fc00::/7 (unique local)
                || (v6.segments()[0] & 0xFE00) == 0xFC00
                // fe80::/10 (link-local)
                || (v6.segments()[0] & 0xFFC0) == 0xFE80)
        },
    }
}

pub(crate) use moltis_auth::locality::is_local_connection;

async fn websocket_header_authenticated(
    headers: &axum::http::HeaderMap,
    credential_store: Option<&Arc<auth::CredentialStore>>,
    is_local: bool,
) -> bool {
    let Some(store) = credential_store else {
        return false;
    };

    matches!(
        crate::auth_middleware::check_auth(store, headers, is_local).await,
        crate::auth_middleware::AuthResult::Allowed(_)
    )
}

/// Resolve the machine's primary outbound IP address.
///
/// Connects a UDP socket to a public DNS address (no traffic is sent) and
/// reads back the local address the OS chose.  Returns `None` when no
/// routable interface is available.
fn resolve_outbound_ip(ipv6: bool) -> Option<std::net::IpAddr> {
    use std::net::UdpSocket;
    let (bind, target) = if ipv6 {
        (":::0", "[2001:4860:4860::8888]:80")
    } else {
        ("0.0.0.0:0", "8.8.8.8:80")
    };
    let socket = UdpSocket::bind(bind).ok()?;
    socket.connect(target).ok()?;
    Some(socket.local_addr().ok()?.ip())
}

fn startup_bind_line(addr: SocketAddr) -> String {
    format!("bind (--bind): {addr}")
}

fn startup_passkey_origin_lines(origins: &[String]) -> Vec<String> {
    origins
        .iter()
        .map(|origin| format!("passkey origin: {origin}"))
        .collect()
}

fn startup_setup_code_lines(code: &str) -> Vec<String> {
    vec![
        String::new(),
        format!("setup code: {code}"),
        "enter this code to set your password or register a passkey".to_string(),
        String::new(),
    ]
}

/// Check whether a WebSocket `Origin` header matches the request `Host`.
///
/// Extracts the host portion of the origin URL and compares it to the Host
/// header.  Accepts `localhost`, `127.0.0.1`, and `[::1]` interchangeably
/// so that `http://localhost:8080` matches a Host of `127.0.0.1:8080`.
pub fn is_same_origin(origin: &str, host: &str) -> bool {
    fn default_port_for_scheme(scheme: &str) -> Option<&'static str> {
        match scheme {
            "http" | "ws" => Some("80"),
            "https" | "wss" => Some("443"),
            _ => None,
        }
    }

    let origin_scheme = origin
        .split("://")
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    // Origin is a full URL (e.g. "https://localhost:8080"), Host is just
    // "host:port" or "host".
    let origin_host = origin
        .split("://")
        .nth(1)
        .unwrap_or(origin)
        .split('/')
        .next()
        .unwrap_or("");

    fn strip_port(h: &str) -> &str {
        if h.starts_with('[') {
            // IPv6: [::1]:port
            h.rsplit_once("]:")
                .map_or(h, |(addr, _)| addr)
                .trim_start_matches('[')
                .trim_end_matches(']')
        } else {
            h.rsplit_once(':').map_or(h, |(addr, _)| addr)
        }
    }
    fn get_port(h: &str) -> Option<&str> {
        if h.starts_with('[') {
            h.rsplit_once("]:").map(|(_, p)| p)
        } else {
            h.rsplit_once(':').map(|(_, p)| p)
        }
    }

    let origin_port = get_port(origin_host).or_else(|| default_port_for_scheme(&origin_scheme));
    let host_port = get_port(host).or_else(|| default_port_for_scheme(&origin_scheme));

    let oh = strip_port(origin_host);
    let hh = strip_port(host);

    // Normalise loopback variants so 127.0.0.1 == localhost == ::1.
    // Subdomains of .localhost (e.g. moltis.localhost) are also loopback per RFC 6761.
    let is_loopback =
        |h: &str| matches!(h, "localhost" | "127.0.0.1" | "::1") || h.ends_with(".localhost");

    (oh == hh || (is_loopback(oh) && is_loopback(hh))) && origin_port == host_port
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_origin_exact_match() {
        assert!(is_same_origin(
            "https://example.com:8080",
            "example.com:8080"
        ));
        assert!(is_same_origin(
            "http://example.com:3000",
            "example.com:3000"
        ));
    }

    #[test]
    fn same_origin_treats_default_ports_as_equivalent() {
        assert!(is_same_origin("https://example.com", "example.com:443"));
        assert!(is_same_origin("https://example.com:443", "example.com"));
        assert!(is_same_origin("http://example.com", "example.com:80"));
        assert!(is_same_origin("http://example.com:80", "example.com"));
    }

    #[test]
    fn same_origin_localhost_variants() {
        // localhost ↔ 127.0.0.1
        assert!(is_same_origin("http://localhost:8080", "127.0.0.1:8080"));
        assert!(is_same_origin("https://127.0.0.1:8080", "localhost:8080"));
        // localhost ↔ ::1
        assert!(is_same_origin("http://localhost:8080", "[::1]:8080"));
        assert!(is_same_origin("http://[::1]:8080", "localhost:8080"));
        // 127.0.0.1 ↔ ::1
        assert!(is_same_origin("http://127.0.0.1:8080", "[::1]:8080"));
    }

    #[test]
    fn cross_origin_rejected() {
        // Different host
        assert!(!is_same_origin("https://attacker.com", "localhost:8080"));
        assert!(!is_same_origin("https://evil.com:8080", "localhost:8080"));
        // Different port
        assert!(!is_same_origin("http://localhost:9999", "localhost:8080"));
    }

    #[test]
    fn same_origin_no_port() {
        assert!(is_same_origin("https://example.com", "example.com"));
        assert!(is_same_origin("http://localhost", "localhost"));
        assert!(is_same_origin("http://localhost", "127.0.0.1"));
    }

    #[test]
    fn cross_origin_port_mismatch() {
        // One has port, other doesn't — different origins.
        assert!(!is_same_origin("http://localhost:8080", "localhost"));
        assert!(!is_same_origin("http://localhost", "localhost:8080"));
    }

    // share_labels and share_social_image tests moved to share_render::tests

    // share_template, map_share_message_views tests moved to share_render::tests

    #[test]
    fn same_origin_moltis_localhost() {
        // moltis.localhost ↔ localhost loopback variants
        assert!(is_same_origin(
            "https://moltis.localhost:8080",
            "localhost:8080"
        ));
        assert!(is_same_origin(
            "https://moltis.localhost:8080",
            "127.0.0.1:8080"
        ));
        assert!(is_same_origin(
            "http://localhost:8080",
            "moltis.localhost:8080"
        ));
        // Any .localhost subdomain is treated as loopback (RFC 6761).
        assert!(is_same_origin(
            "https://app.moltis.localhost:8080",
            "localhost:8080"
        ));
    }

    #[test]
    fn websocket_origin_host_prefers_forwarded_host_when_behind_proxy() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::HOST, "127.0.0.1:13131".parse().unwrap());
        headers.insert("x-forwarded-host", "chat.example.com".parse().unwrap());
        assert_eq!(
            websocket_origin_host(&headers, true).as_deref(),
            Some("chat.example.com")
        );
    }

    #[test]
    fn websocket_origin_host_uses_host_without_proxy_mode() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::HOST,
            "gateway.example.com:8443".parse().unwrap(),
        );
        headers.insert("x-forwarded-host", "chat.example.com".parse().unwrap());
        assert_eq!(
            websocket_origin_host(&headers, false).as_deref(),
            Some("gateway.example.com:8443")
        );
    }

    #[test]
    fn prebuild_runs_only_when_mode_enabled_and_packages_present() {
        let packages = vec!["curl".to_string()];
        assert!(should_prebuild_sandbox_image(
            &moltis_tools::sandbox::SandboxMode::All,
            &packages
        ));
        assert!(should_prebuild_sandbox_image(
            &moltis_tools::sandbox::SandboxMode::NonMain,
            &packages
        ));
        assert!(!should_prebuild_sandbox_image(
            &moltis_tools::sandbox::SandboxMode::Off,
            &packages
        ));
        assert!(!should_prebuild_sandbox_image(
            &moltis_tools::sandbox::SandboxMode::All,
            &[]
        ));
    }

    #[test]
    fn resolve_outbound_ip_returns_non_loopback() {
        // This test requires network connectivity; skip gracefully otherwise.
        if let Some(ip) = resolve_outbound_ip(false) {
            assert!(!ip.is_loopback(), "expected a non-loopback IP, got {ip}");
            assert!(!ip.is_unspecified(), "expected a routable IP, got {ip}");
        }
    }

    #[test]
    fn display_host_uses_real_ip_for_unspecified_bind() {
        let addr: SocketAddr = "0.0.0.0:9999".parse().unwrap();
        assert!(addr.ip().is_unspecified());

        if let Some(ip) = resolve_outbound_ip(false) {
            let display = SocketAddr::new(ip, addr.port());
            assert!(!display.ip().is_unspecified());
            assert_eq!(display.port(), 9999);
        }
    }

    #[test]
    fn startup_bind_line_includes_bind_flag_and_address() {
        let addr: SocketAddr = "0.0.0.0:49494".parse().unwrap();
        assert_eq!(startup_bind_line(addr), "bind (--bind): 0.0.0.0:49494");
    }

    #[test]
    fn startup_passkey_origin_lines_emits_clickable_urls() {
        let lines = startup_passkey_origin_lines(&[
            "https://localhost:49494".to_string(),
            "https://m4max.local:49494".to_string(),
        ]);
        assert_eq!(lines, vec![
            "passkey origin: https://localhost:49494",
            "passkey origin: https://m4max.local:49494",
        ]);
    }

    #[test]
    fn startup_setup_code_lines_adds_spacers() {
        let lines = startup_setup_code_lines("493413");
        assert_eq!(lines, vec![
            "",
            "setup code: 493413",
            "enter this code to set your password or register a passkey",
            "",
        ]);
    }
}
