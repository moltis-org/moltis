use std::{collections::HashSet, net::SocketAddr, sync::Arc};

use secrecy::ExposeSecret;

#[cfg(feature = "tls")]
use std::path::PathBuf;

#[cfg(feature = "web-ui")]
use askama::Template;

#[cfg(feature = "web-ui")]
use axum::response::{Html, Redirect};
use {
    axum::{
        Router,
        extract::{ConnectInfo, State, WebSocketUpgrade},
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
    tracing::{Level, debug, info, warn},
};

#[cfg(feature = "web-ui")]
use axum::{extract::Path, http::StatusCode};

use {moltis_channels::ChannelPlugin, moltis_protocol::TICK_INTERVAL_MS};

use moltis_agents::providers::ProviderRegistry;

use moltis_tools::{
    approval::{ApprovalManager, ApprovalMode, SecurityLevel},
    exec::EnvVarProvider,
    image_cache::ImageBuilder,
};

use {
    moltis_projects::ProjectStore,
    moltis_sessions::{
        metadata::{SessionMetadata, SqliteSessionMetadata},
        store::SessionStore,
    },
};

use crate::{
    approval::{GatewayApprovalBroadcaster, LiveExecApprovalService},
    auth,
    auth_routes::{AuthState, auth_router},
    broadcast::{BroadcastOpts, broadcast, broadcast_tick},
    chat::{LiveChatService, LiveModelService},
    methods::MethodRegistry,
    provider_setup::LiveProviderSetupService,
    services::GatewayServices,
    session::LiveSessionService,
    state::GatewayState,
    update_check::{
        UPDATE_CHECK_INTERVAL, fetch_update_availability, github_latest_release_api_url,
        resolve_repository_url,
    },
    ws::handle_connection,
};

#[cfg(feature = "tailscale")]
use crate::tailscale::{
    CliTailscaleManager, TailscaleManager, TailscaleMode, validate_tailscale_config,
};

#[cfg(feature = "tls")]
use crate::tls::CertManager;

/// Options for tailscale serve/funnel passed from CLI flags.
#[cfg(feature = "tailscale")]
pub struct TailscaleOpts {
    pub mode: String,
    pub reset_on_exit: bool,
}

// ── Location requester ───────────────────────────────────────────────────────

/// Gateway implementation of [`moltis_tools::location::LocationRequester`].
///
/// Uses the `PendingInvoke` + oneshot pattern to request the user's browser
/// geolocation and waits for `location.result` RPC to resolve it.
struct GatewayLocationRequester {
    state: Arc<GatewayState>,
}

#[async_trait::async_trait]
impl moltis_tools::location::LocationRequester for GatewayLocationRequester {
    async fn request_location(
        &self,
        conn_id: &str,
        precision: moltis_tools::location::LocationPrecision,
    ) -> anyhow::Result<moltis_tools::location::LocationResult> {
        use moltis_tools::location::{LocationError, LocationResult};

        let request_id = uuid::Uuid::new_v4().to_string();

        // Send a location.request event to the browser client, including
        // the requested precision so JS can adjust geolocation options.
        let event = moltis_protocol::EventFrame::new(
            "location.request",
            serde_json::json!({ "requestId": request_id, "precision": precision }),
            self.state.next_seq(),
        );
        let event_json = serde_json::to_string(&event)?;

        {
            let inner = self.state.inner.read().await;
            let clients = &inner.clients;
            let client = clients
                .get(conn_id)
                .ok_or_else(|| anyhow::anyhow!("no client connection for conn_id {conn_id}"))?;
            if !client.send(&event_json) {
                anyhow::bail!("failed to send location request to client {conn_id}");
            }
        }

        // Set up a oneshot for the result with timeout.
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut inner_w = self.state.inner.write().await;
            let invokes = &mut inner_w.pending_invokes;
            invokes.insert(request_id.clone(), crate::state::PendingInvoke {
                request_id: request_id.clone(),
                sender: tx,
                created_at: std::time::Instant::now(),
            });
        }

        // Wait up to 30 seconds for the user to grant/deny permission.
        let result = match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(value)) => value,
            Ok(Err(_)) => {
                // Sender dropped — clean up.
                self.state
                    .inner
                    .write()
                    .await
                    .pending_invokes
                    .remove(&request_id);
                return Ok(LocationResult {
                    location: None,
                    error: Some(LocationError::Timeout),
                });
            },
            Err(_) => {
                // Timeout — clean up.
                self.state
                    .inner
                    .write()
                    .await
                    .pending_invokes
                    .remove(&request_id);
                return Ok(LocationResult {
                    location: None,
                    error: Some(LocationError::Timeout),
                });
            },
        };

        // Parse the result from the browser.
        if let Some(loc) = result.get("location") {
            let lat = loc.get("latitude").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let lon = loc.get("longitude").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let accuracy = loc.get("accuracy").and_then(|v| v.as_f64()).unwrap_or(0.0);
            Ok(LocationResult {
                location: Some(moltis_tools::location::BrowserLocation {
                    latitude: lat,
                    longitude: lon,
                    accuracy,
                }),
                error: None,
            })
        } else if let Some(err) = result.get("error") {
            let code = err.get("code").and_then(|v| v.as_u64()).unwrap_or(0);
            let error = match code {
                1 => LocationError::PermissionDenied,
                2 => LocationError::PositionUnavailable,
                3 => LocationError::Timeout,
                _ => LocationError::NotSupported,
            };
            Ok(LocationResult {
                location: None,
                error: Some(error),
            })
        } else {
            Ok(LocationResult {
                location: None,
                error: Some(LocationError::PositionUnavailable),
            })
        }
    }

    fn cached_location(&self) -> Option<moltis_config::GeoLocation> {
        self.state.inner.try_read().ok()?.cached_location.clone()
    }

    async fn request_channel_location(
        &self,
        session_key: &str,
    ) -> anyhow::Result<moltis_tools::location::LocationResult> {
        use moltis_tools::location::{LocationError, LocationResult};

        // Look up channel binding from session metadata.
        let session_meta = self
            .state
            .services
            .session_metadata
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("session metadata not available"))?;
        let entry = session_meta
            .get(session_key)
            .await
            .ok_or_else(|| anyhow::anyhow!("no session metadata for key {session_key}"))?;
        let binding_json = entry
            .channel_binding
            .ok_or_else(|| anyhow::anyhow!("no channel binding for session {session_key}"))?;
        let reply_target: moltis_channels::ChannelReplyTarget =
            serde_json::from_str(&binding_json)?;

        // Send a message asking the user to share their location.
        let outbound = self
            .state
            .services
            .channel_outbound_arc()
            .ok_or_else(|| anyhow::anyhow!("no channel outbound available"))?;
        outbound
            .send_text(
                &reply_target.account_id,
                &reply_target.chat_id,
                "Please share your location using the attachment menu (📎 → Location).",
                None,
            )
            .await?;

        // Create a pending invoke keyed by session.
        let pending_key = format!("channel_location:{session_key}");
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut inner = self.state.inner.write().await;
            inner
                .pending_invokes
                .insert(pending_key.clone(), crate::state::PendingInvoke {
                    request_id: pending_key.clone(),
                    sender: tx,
                    created_at: std::time::Instant::now(),
                });
        }

        // Wait up to 60 seconds — user needs to navigate Telegram's UI.
        let result = match tokio::time::timeout(std::time::Duration::from_secs(60), rx).await {
            Ok(Ok(value)) => value,
            Ok(Err(_)) => {
                self.state
                    .inner
                    .write()
                    .await
                    .pending_invokes
                    .remove(&pending_key);
                return Ok(LocationResult {
                    location: None,
                    error: Some(LocationError::Timeout),
                });
            },
            Err(_) => {
                self.state
                    .inner
                    .write()
                    .await
                    .pending_invokes
                    .remove(&pending_key);
                return Ok(LocationResult {
                    location: None,
                    error: Some(LocationError::Timeout),
                });
            },
        };

        // Parse the result (same format as update_location sends).
        if let Some(loc) = result.get("location") {
            let lat = loc.get("latitude").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let lon = loc.get("longitude").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let accuracy = loc.get("accuracy").and_then(|v| v.as_f64()).unwrap_or(0.0);
            Ok(LocationResult {
                location: Some(moltis_tools::location::BrowserLocation {
                    latitude: lat,
                    longitude: lon,
                    accuracy,
                }),
                error: None,
            })
        } else {
            Ok(LocationResult {
                location: None,
                error: Some(LocationError::PositionUnavailable),
            })
        }
    }
}

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

fn sandbox_container_prefix(instance_slug: &str) -> String {
    format!("moltis-{instance_slug}-sandbox")
}

fn browser_container_prefix(instance_slug: &str) -> String {
    format!("moltis-{instance_slug}-browser")
}

async fn ollama_has_model(base_url: &str, model: &str) -> bool {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let response = match reqwest::Client::new().get(url).send().await {
        Ok(resp) => resp,
        Err(_) => return false,
    };
    if !response.status().is_success() {
        return false;
    }
    let value: serde_json::Value = match response.json().await {
        Ok(v) => v,
        Err(_) => return false,
    };
    value
        .get("models")
        .and_then(|m| m.as_array())
        .map(|models| {
            models.iter().any(|m| {
                let name = m.get("name").and_then(|n| n.as_str()).unwrap_or_default();
                name == model || name.starts_with(&format!("{model}:"))
            })
        })
        .unwrap_or(false)
}

async fn ensure_ollama_model(base_url: &str, model: &str) {
    if ollama_has_model(base_url, model).await {
        return;
    }

    warn!(
        model = %model,
        base_url = %base_url,
        "memory: missing Ollama embedding model, attempting auto-pull"
    );

    let url = format!("{}/api/pull", base_url.trim_end_matches('/'));
    let pull = reqwest::Client::new()
        .post(url)
        .json(&serde_json::json!({ "name": model, "stream": false }))
        .send()
        .await;

    match pull {
        Ok(resp) if resp.status().is_success() => {
            info!(model = %model, "memory: Ollama model pull complete");
        },
        Ok(resp) => {
            warn!(
                model = %model,
                status = %resp.status(),
                "memory: Ollama model pull failed"
            );
        },
        Err(e) => {
            warn!(model = %model, error = %e, "memory: Ollama model pull request failed");
        },
    }
}

fn approval_manager_from_config(config: &moltis_config::MoltisConfig) -> ApprovalManager {
    let mut manager = ApprovalManager::default();

    manager.mode = ApprovalMode::parse(&config.tools.exec.approval_mode).unwrap_or_else(|| {
        warn!(
            value = %config.tools.exec.approval_mode,
            "invalid tools.exec.approval_mode; falling back to 'on-miss'"
        );
        ApprovalMode::OnMiss
    });

    manager.security_level = SecurityLevel::parse(&config.tools.exec.security_level)
        .unwrap_or_else(|| {
            warn!(
                value = %config.tools.exec.security_level,
                "invalid tools.exec.security_level; falling back to 'allowlist'"
            );
            SecurityLevel::Allowlist
        });

    manager.allowlist = config.tools.exec.allowlist.clone();
    manager
}

// ── Shared app state ─────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub gateway: Arc<GatewayState>,
    pub methods: Arc<MethodRegistry>,
    pub request_throttle: Arc<crate::request_throttle::RequestThrottle>,
    #[cfg(feature = "push-notifications")]
    pub push_service: Option<Arc<crate::push::PushService>>,
}

// ── Server startup ───────────────────────────────────────────────────────────

/// Build the API routes (shared between both build_gateway_app versions).
///
/// Auth is enforced by `auth_gate` middleware on the whole router — these
/// routes no longer carry their own auth layer.
#[cfg(feature = "web-ui")]
fn build_api_routes() -> Router<AppState> {
    let protected = Router::new()
        .route("/api/bootstrap", get(api_bootstrap_handler))
        .route("/api/gon", get(api_gon_handler))
        .route("/api/skills", get(api_skills_handler))
        .route("/api/skills/search", get(api_skills_search_handler))
        .route("/api/mcp", get(api_mcp_handler))
        .route("/api/hooks", get(api_hooks_handler))
        .route(
            "/api/images/cached",
            get(api_cached_images_handler).delete(api_prune_cached_images_handler),
        )
        .route(
            "/api/images/cached/{tag}",
            axum::routing::delete(api_delete_cached_image_handler),
        )
        .route(
            "/api/images/build",
            axum::routing::post(api_build_image_handler),
        )
        .route(
            "/api/images/check-packages",
            axum::routing::post(api_check_packages_handler),
        )
        .route(
            "/api/images/default",
            get(api_get_default_image_handler).put(api_set_default_image_handler),
        )
        .route(
            "/api/env",
            get(crate::env_routes::env_list).post(crate::env_routes::env_set),
        )
        .route(
            "/api/env/{id}",
            axum::routing::delete(crate::env_routes::env_delete),
        )
        // Config editor routes (sensitive - requires auth)
        .route(
            "/api/config",
            get(crate::tools_routes::config_get).post(crate::tools_routes::config_save),
        )
        .route(
            "/api/config/validate",
            axum::routing::post(crate::tools_routes::config_validate),
        )
        .route(
            "/api/config/template",
            get(crate::tools_routes::config_template),
        )
        .route(
            "/api/restart",
            axum::routing::post(crate::tools_routes::restart),
        )
        .route(
            "/api/sessions/{session_key}/upload",
            axum::routing::post(crate::upload_routes::session_upload)
                .layer(axum::extract::DefaultBodyLimit::max(
                    crate::upload_routes::MAX_UPLOAD_SIZE,
                )),
        )
        .route(
            "/api/sessions/{session_key}/media/{filename}",
            get(api_session_media_handler),
        )
        .route("/api/logs/download", get(api_logs_download_handler));

    // Add metrics API routes (protected).
    #[cfg(feature = "metrics")]
    let protected = protected
        .route(
            "/api/metrics",
            get(crate::metrics_routes::api_metrics_handler),
        )
        .route(
            "/api/metrics/summary",
            get(crate::metrics_routes::api_metrics_summary_handler),
        )
        .route(
            "/api/metrics/history",
            get(crate::metrics_routes::api_metrics_history_handler),
        );

    protected
}

/// Add feature-specific routes to API routes.
#[cfg(feature = "web-ui")]
fn add_feature_routes(routes: Router<AppState>) -> Router<AppState> {
    // Mount tailscale routes when the feature is enabled.
    #[cfg(feature = "tailscale")]
    let routes = routes.nest(
        "/api/tailscale",
        crate::tailscale_routes::tailscale_router(),
    );

    // Mount push notification routes when the feature is enabled.
    #[cfg(feature = "push-notifications")]
    let routes = routes.nest("/api/push", crate::push_routes::push_router());

    routes
}

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
            HeaderValue::from_static("deny"),
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

/// Build the gateway router (shared between production startup and tests).
#[cfg(feature = "push-notifications")]
pub fn build_gateway_app(
    state: Arc<GatewayState>,
    methods: Arc<MethodRegistry>,
    push_service: Option<Arc<crate::push::PushService>>,
    http_request_logs: bool,
    webauthn_state: Option<Arc<crate::auth_webauthn::WebAuthnState>>,
) -> Router {
    let cors = build_cors_layer();

    let mut router = Router::new()
        .route("/health", get(health_handler))
        .route("/ws", get(ws_upgrade_handler));

    // Nest auth routes if credential store is available.
    if let Some(ref cred_store) = state.credential_store {
        let auth_state = AuthState {
            credential_store: Arc::clone(cred_store),
            webauthn_state: webauthn_state.clone(),
            gateway_state: Arc::clone(&state),
        };
        router = router.nest("/api/auth", auth_router().with_state(auth_state));
    }

    let app_state = AppState {
        gateway: state,
        methods,
        request_throttle: Arc::new(crate::request_throttle::RequestThrottle::new()),
        push_service,
    };

    #[cfg(feature = "web-ui")]
    let router = {
        let api = build_api_routes();
        let api = add_feature_routes(api);

        router
            .route("/auth/callback", get(oauth_callback_handler))
            .route("/onboarding", get(onboarding_handler))
            .route("/login", get(login_handler_page))
            .route("/assets/v/{version}/{*path}", get(versioned_asset_handler))
            .route("/assets/{*path}", get(asset_handler))
            .route("/manifest.json", get(manifest_handler))
            .route("/sw.js", get(service_worker_handler))
            .merge(api)
            .fallback(spa_fallback)
            .layer(axum::middleware::from_fn_with_state(
                app_state.clone(),
                crate::auth_middleware::auth_gate,
            ))
    };

    let router = router.layer(axum::middleware::from_fn_with_state(
        app_state.clone(),
        crate::request_throttle::throttle_gate,
    ));

    let router = apply_middleware_stack(router, cors, http_request_logs);

    router.with_state(app_state)
}

/// Build the gateway router (shared between production startup and tests).
#[cfg(not(feature = "push-notifications"))]
pub fn build_gateway_app(
    state: Arc<GatewayState>,
    methods: Arc<MethodRegistry>,
    http_request_logs: bool,
    webauthn_state: Option<Arc<crate::auth_webauthn::WebAuthnState>>,
) -> Router {
    let cors = build_cors_layer();

    let mut router = Router::new()
        .route("/health", get(health_handler))
        .route("/ws", get(ws_upgrade_handler));

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
            webauthn_state: webauthn_state.clone(),
            gateway_state: Arc::clone(&state),
        };
        router = router.nest("/api/auth", auth_router().with_state(auth_state));
    }

    let app_state = AppState {
        gateway: state,
        methods,
        request_throttle: Arc::new(crate::request_throttle::RequestThrottle::new()),
    };

    #[cfg(feature = "web-ui")]
    let router = {
        let api = build_api_routes();
        let api = add_feature_routes(api);

        router
            .route("/auth/callback", get(oauth_callback_handler))
            .route("/onboarding", get(onboarding_handler))
            .route("/login", get(login_handler_page))
            .route("/assets/v/{version}/{*path}", get(versioned_asset_handler))
            .route("/assets/{*path}", get(asset_handler))
            .route("/manifest.json", get(manifest_handler))
            .route("/sw.js", get(service_worker_handler))
            .merge(api)
            .fallback(spa_fallback)
            .layer(axum::middleware::from_fn_with_state(
                app_state.clone(),
                crate::auth_middleware::auth_gate,
            ))
    };

    let router = router.layer(axum::middleware::from_fn_with_state(
        app_state.clone(),
        crate::request_throttle::throttle_gate,
    ));

    let router = apply_middleware_stack(router, cors, http_request_logs);

    router.with_state(app_state)
}

/// Start the gateway HTTP + WebSocket server.
#[allow(clippy::expect_used)] // Startup fail-fast: DB, migrations, credential store must succeed.
pub async fn start_gateway(
    bind: &str,
    port: u16,
    no_tls: bool,
    log_buffer: Option<crate::logs::LogBuffer>,
    config_dir: Option<std::path::PathBuf>,
    data_dir: Option<std::path::PathBuf>,
    #[cfg(feature = "tailscale")] tailscale_opts: Option<TailscaleOpts>,
) -> anyhow::Result<()> {
    // Apply directory overrides before loading config.
    if let Some(dir) = config_dir {
        moltis_config::set_config_dir(dir);
    }
    if let Some(ref dir) = data_dir {
        moltis_config::set_data_dir(dir.clone());
    }

    // Resolve auth from environment (MOLTIS_TOKEN / MOLTIS_PASSWORD).
    let token = std::env::var("MOLTIS_TOKEN").ok();
    let password = std::env::var("MOLTIS_PASSWORD").ok();

    // Cloud deploy platform — hides local-only providers (local-llm, ollama).
    let deploy_platform = std::env::var("MOLTIS_DEPLOY_PLATFORM").ok();
    let resolved_auth = auth::resolve_auth(token, password.clone());

    // Load config file (moltis.toml / .yaml / .json) if present.
    let mut config = moltis_config::discover_and_load();
    let instance_slug_value = instance_slug(&config);
    let browser_container_prefix = browser_container_prefix(&instance_slug_value);
    let sandbox_container_prefix = sandbox_container_prefix(&instance_slug_value);

    // CLI --no-tls / MOLTIS_NO_TLS overrides config file TLS setting.
    if no_tls {
        config.tls.enabled = false;
    }

    let base_provider_config = config.providers.clone();

    // Merge any previously saved API keys into the provider config so they
    // survive gateway restarts without requiring env vars.
    let key_store = crate::provider_setup::KeyStore::new();
    let effective_providers =
        crate::provider_setup::config_with_saved_keys(&base_provider_config, &key_store);

    let has_explicit_provider_settings =
        crate::provider_setup::has_explicit_provider_settings(&config.providers);
    let auto_detected_provider_sources = if has_explicit_provider_settings {
        Vec::new()
    } else {
        crate::provider_setup::detect_auto_provider_sources(
            &config.providers,
            deploy_platform.as_deref(),
        )
    };

    // Discover LLM providers from env + config + saved keys.
    let registry = Arc::new(tokio::sync::RwLock::new(
        ProviderRegistry::from_env_with_config(&effective_providers),
    ));
    let (provider_summary, providers_available_at_startup) = {
        let reg = registry.read().await;
        (reg.provider_summary(), !reg.is_empty())
    };
    if !providers_available_at_startup {
        warn!(
            "no LLM providers at startup; model/chat services remain active and will pick up providers after credentials are saved"
        );
    }

    if !has_explicit_provider_settings {
        if auto_detected_provider_sources.is_empty() {
            info!("llm auto-detect: no providers detected from env/files");
        } else {
            for detected in &auto_detected_provider_sources {
                info!(
                    provider = %detected.provider,
                    source = %detected.source,
                    "llm auto-detected provider source"
                );
            }
            // Import external tokens (e.g. Codex CLI auth.json) into the
            // token store so all providers read from a single location.
            let import_token_store = moltis_oauth::TokenStore::new();
            crate::provider_setup::import_detected_oauth_tokens(
                &auto_detected_provider_sources,
                &import_token_store,
            );
        }
    }

    // Refresh dynamic provider model discovery hourly so long-lived sessions
    // pick up newly available models without requiring a restart.
    {
        let registry_for_refresh = Arc::clone(&registry);
        let provider_config_for_refresh = base_provider_config.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60 * 60));
            interval.tick().await;
            loop {
                interval.tick().await;
                let mut reg = registry_for_refresh.write().await;
                let refresh_results = reg.refresh_dynamic_models(&provider_config_for_refresh);
                for (provider_name, refreshed) in refresh_results {
                    if !refreshed {
                        continue;
                    }
                    let model_count = reg
                        .list_models()
                        .iter()
                        .filter(|m| m.provider == provider_name)
                        .count();
                    info!(
                        provider = %provider_name,
                        models = model_count,
                        "hourly dynamic provider model refresh complete"
                    );
                }
            }
        });
    }

    // Create shared approval manager from config.
    let approval_manager = Arc::new(approval_manager_from_config(&config));

    let mut services = GatewayServices::noop();

    // Wire live logs service if a log buffer is available.
    if let Some(ref buf) = log_buffer {
        services.logs = Arc::new(crate::logs::LiveLogsService::new(buf.clone()));
    }

    services.exec_approval = Arc::new(LiveExecApprovalService::new(Arc::clone(&approval_manager)));

    // Wire browser service if enabled.
    if let Some(browser_svc) =
        crate::services::RealBrowserService::from_config(&config, browser_container_prefix)
    {
        services.browser = Arc::new(browser_svc);
    }

    // Wire live onboarding service.
    let onboarding_config_path = moltis_config::find_or_default_config_path();
    let live_onboarding =
        moltis_onboarding::service::LiveOnboardingService::new(onboarding_config_path);
    services = services.with_onboarding(Arc::new(
        crate::onboarding::GatewayOnboardingService::new(live_onboarding),
    ));
    // Wire live local-llm service when the feature is enabled.
    #[cfg(feature = "local-llm")]
    let local_llm_service: Option<Arc<crate::local_llm_setup::LiveLocalLlmService>> = {
        let svc = Arc::new(crate::local_llm_setup::LiveLocalLlmService::new(
            Arc::clone(&registry),
        ));
        services =
            services.with_local_llm(Arc::clone(&svc) as Arc<dyn crate::services::LocalLlmService>);
        Some(svc)
    };
    // When local-llm feature is disabled, this variable is not needed since
    // the only usage is also feature-gated.

    // Wire live voice services when the feature is enabled.
    #[cfg(feature = "voice")]
    {
        use crate::voice::{LiveSttService, LiveTtsService, SttServiceConfig};

        // Services read fresh config from disk on each operation,
        // so we just need to create the instances here.
        services.tts = Arc::new(LiveTtsService::new(moltis_voice::TtsConfig::default()));
        services.stt = Arc::new(LiveSttService::new(SttServiceConfig::default()));
    }

    let model_store = Arc::new(tokio::sync::RwLock::new(
        crate::chat::DisabledModelsStore::load(),
    ));

    let live_model_service = Arc::new(LiveModelService::new(
        Arc::clone(&registry),
        Arc::clone(&model_store),
        config.chat.priority_models.clone(),
    ));
    services = services
        .with_model(Arc::clone(&live_model_service) as Arc<dyn crate::services::ModelService>);

    // Create provider setup after model service so we can share the
    // priority models handle for live dropdown reordering.
    let mut provider_setup = LiveProviderSetupService::new(
        Arc::clone(&registry),
        config.providers.clone(),
        deploy_platform.clone(),
    );
    provider_setup.set_priority_models(live_model_service.priority_models_handle());
    services.provider_setup = Arc::new(provider_setup);

    // Wire live MCP service.
    let mcp_configured_count;
    let live_mcp: Arc<crate::mcp_service::LiveMcpService>;
    {
        let mcp_registry_path = moltis_config::data_dir().join("mcp-servers.json");
        let mcp_reg = moltis_mcp::McpRegistry::load(&mcp_registry_path).unwrap_or_default();
        // Seed from config file servers that aren't already in the registry.
        let mut merged = mcp_reg;
        for (name, entry) in &config.mcp.servers {
            if !merged.servers.contains_key(name) {
                let transport = match entry.transport.as_str() {
                    "sse" => moltis_mcp::registry::TransportType::Sse,
                    _ => moltis_mcp::registry::TransportType::Stdio,
                };
                merged
                    .servers
                    .insert(name.clone(), moltis_mcp::McpServerConfig {
                        command: entry.command.clone(),
                        args: entry.args.clone(),
                        env: entry.env.clone(),
                        enabled: entry.enabled,
                        transport,
                        url: entry.url.clone(),
                    });
            }
        }
        mcp_configured_count = merged.servers.values().filter(|s| s.enabled).count();
        let mcp_manager = Arc::new(moltis_mcp::McpManager::new(merged));
        live_mcp = Arc::new(crate::mcp_service::LiveMcpService::new(Arc::clone(
            &mcp_manager,
        )));
        // Start enabled servers in the background; sync tools once done.
        let mgr = Arc::clone(&mcp_manager);
        let mcp_for_sync = Arc::clone(&live_mcp);
        tokio::spawn(async move {
            let started = mgr.start_enabled().await;
            if !started.is_empty() {
                tracing::info!(servers = ?started, "MCP servers started");
            }
            // Sync newly started tools into the agent tool registry.
            mcp_for_sync.sync_tools_if_ready().await;
        });
        services.mcp = live_mcp.clone() as Arc<dyn crate::services::McpService>;
    }

    // Initialize data directory and SQLite database.
    let data_dir = data_dir.unwrap_or_else(moltis_config::data_dir);
    std::fs::create_dir_all(&data_dir).unwrap_or_else(|e| {
        panic!(
            "failed to create data directory {}: {e}",
            data_dir.display()
        )
    });

    let config_dir =
        moltis_config::config_dir().unwrap_or_else(|| std::path::PathBuf::from(".moltis"));
    std::fs::create_dir_all(&config_dir).unwrap_or_else(|e| {
        panic!(
            "failed to create config directory {}: {e}",
            config_dir.display()
        )
    });

    // Enable log persistence so entries survive restarts.
    if let Some(ref buf) = log_buffer {
        buf.enable_persistence(data_dir.join("logs.jsonl"));
    }
    let db_path = data_dir.join("moltis.db");
    let db_url = format!("sqlite:{}?mode=rwc", db_path.display());
    let db_pool = sqlx::SqlitePool::connect(&db_url)
        .await
        .expect("failed to open moltis.db");

    // Run database migrations from each crate in dependency order.
    // Order matters: sessions depends on projects (FK reference).
    moltis_projects::run_migrations(&db_pool)
        .await
        .expect("failed to run projects migrations");
    moltis_sessions::run_migrations(&db_pool)
        .await
        .expect("failed to run sessions migrations");
    moltis_cron::run_migrations(&db_pool)
        .await
        .expect("failed to run cron migrations");
    // Gateway's own tables (auth, message_log, channels).
    crate::run_migrations(&db_pool)
        .await
        .expect("failed to run gateway migrations");

    // Migrate plugins data into unified skills system (idempotent, non-fatal).
    moltis_skills::migration::migrate_plugins_to_skills(&data_dir).await;

    // Initialize credential store (auth tables).
    let credential_store = Arc::new(
        auth::CredentialStore::new(db_pool.clone())
            .await
            .expect("failed to init credential store"),
    );

    // Initialize WebAuthn state for passkey support.
    // RP ID: explicit env > PaaS env (DO, Render, Fly, Railway) > "localhost"
    let rp_id = std::env::var("MOLTIS_WEBAUTHN_RP_ID")
        .or_else(|_| std::env::var("APP_DOMAIN"))
        .or_else(|_| std::env::var("RENDER_EXTERNAL_HOSTNAME"))
        .or_else(|_| std::env::var("FLY_APP_NAME").map(|name| format!("{name}.fly.dev")))
        .or_else(|_| std::env::var("RAILWAY_PUBLIC_DOMAIN"))
        .unwrap_or_else(|_| "localhost".into());
    let default_scheme = if config.tls.enabled {
        "https"
    } else {
        "http"
    };
    // Origin: explicit env > PaaS env (DO, Render, Fly) > scheme://rp_id(:port)
    // PaaS platforms proxy on standard ports, so skip the port when rp_id isn't localhost.
    let rp_origin_str = std::env::var("MOLTIS_WEBAUTHN_ORIGIN")
        .or_else(|_| std::env::var("APP_URL"))
        .or_else(|_| std::env::var("RENDER_EXTERNAL_URL"))
        .unwrap_or_else(|_| {
            if rp_id == "localhost" {
                format!("{default_scheme}://{rp_id}:{port}")
            } else {
                // PaaS platforms terminate TLS at the proxy, so the browser always
                // sees https even though the app runs with --no-tls.
                format!("https://{rp_id}")
            }
        });
    // Build extra allowed origins so passkeys work when accessed via mDNS
    // hostname (e.g. http://m4max.local:18080) in addition to localhost.
    let mut extra_origins = Vec::new();
    if rp_id == "localhost"
        && let Ok(url) =
            webauthn_rs::prelude::Url::parse(&format!("{default_scheme}://moltis.localhost:{port}"))
    {
        extra_origins.push(url);
    }
    if let Ok(hn) = hostname::get() {
        let hn_str = hn.to_string_lossy();
        if hn_str != rp_id && hn_str != "localhost" {
            if let Ok(url) =
                webauthn_rs::prelude::Url::parse(&format!("{default_scheme}://{hn_str}:{port}"))
            {
                extra_origins.push(url);
            }
            // Also accept the .local mDNS variant if the hostname doesn't
            // already end with .local.
            if !hn_str.ends_with(".local")
                && let Ok(url) = webauthn_rs::prelude::Url::parse(&format!(
                    "{default_scheme}://{hn_str}.local:{port}"
                ))
            {
                extra_origins.push(url);
            }
        }
    }

    let webauthn_state = match webauthn_rs::prelude::Url::parse(&rp_origin_str) {
        Ok(rp_origin) => {
            match crate::auth_webauthn::WebAuthnState::new(&rp_id, &rp_origin, &extra_origins) {
                Ok(wa) => {
                    let origins = wa.get_allowed_origins();
                    info!(rp_id = %rp_id, origins = ?origins, "WebAuthn passkeys enabled");
                    Some(Arc::new(wa))
                },
                Err(e) => {
                    tracing::warn!("failed to init WebAuthn: {e}");
                    None
                },
            }
        },
        Err(e) => {
            tracing::warn!("invalid WebAuthn origin URL '{rp_origin_str}': {e}");
            None
        },
    };

    // If MOLTIS_PASSWORD is set and no password in DB yet, migrate it.
    if let Some(ref pw) = password
        && !credential_store.is_setup_complete()
    {
        info!("migrating MOLTIS_PASSWORD env var to credential store");
        if let Err(e) = credential_store.set_initial_password(pw).await {
            tracing::warn!("failed to migrate env password: {e}");
        }
    }

    let message_log: Arc<dyn moltis_channels::message_log::MessageLog> = Arc::new(
        crate::message_log_store::SqliteMessageLog::new(db_pool.clone()),
    );

    // Migrate from projects.toml if it exists.
    let config_dir =
        moltis_config::config_dir().unwrap_or_else(|| std::path::PathBuf::from(".moltis"));
    let projects_toml_path = config_dir.join("projects.toml");
    if projects_toml_path.exists() {
        info!("migrating projects.toml to SQLite");
        let old_store = moltis_projects::TomlProjectStore::new(projects_toml_path.clone());
        let sqlite_store = moltis_projects::SqliteProjectStore::new(db_pool.clone());
        if let Ok(projects) =
            <moltis_projects::TomlProjectStore as moltis_projects::ProjectStore>::list(&old_store)
                .await
        {
            for p in projects {
                if let Err(e) = sqlite_store.upsert(p).await {
                    tracing::warn!("failed to migrate project: {e}");
                }
            }
        }
        let bak = projects_toml_path.with_extension("toml.bak");
        std::fs::rename(&projects_toml_path, &bak).ok();
    }

    // Migrate from metadata.json if it exists.
    let sessions_dir = data_dir.join("sessions");
    let metadata_json_path = sessions_dir.join("metadata.json");
    if metadata_json_path.exists() {
        info!("migrating metadata.json to SQLite");
        if let Ok(old_meta) = SessionMetadata::load(metadata_json_path.clone()) {
            let sqlite_meta = SqliteSessionMetadata::new(db_pool.clone());
            for entry in old_meta.list() {
                if let Err(e) = sqlite_meta.upsert(&entry.key, entry.label.clone()).await {
                    tracing::warn!("failed to migrate session {}: {e}", entry.key);
                }
                if entry.model.is_some() {
                    sqlite_meta.set_model(&entry.key, entry.model.clone()).await;
                }
                sqlite_meta.touch(&entry.key, entry.message_count).await;
                if entry.project_id.is_some() {
                    sqlite_meta
                        .set_project_id(&entry.key, entry.project_id.clone())
                        .await;
                }
            }
        }
        let bak = metadata_json_path.with_extension("json.bak");
        std::fs::rename(&metadata_json_path, &bak).ok();
    }

    // Wire stores.
    let project_store: Arc<dyn moltis_projects::ProjectStore> =
        Arc::new(moltis_projects::SqliteProjectStore::new(db_pool.clone()));
    let session_store = Arc::new(SessionStore::new(sessions_dir));
    let session_metadata = Arc::new(SqliteSessionMetadata::new(db_pool.clone()));
    let session_state_store = Arc::new(moltis_sessions::state_store::SessionStateStore::new(
        db_pool.clone(),
    ));

    // Session service wired below after sandbox_router is created.

    // Wire live project service.
    services.project = Arc::new(crate::project::LiveProjectService::new(Arc::clone(
        &project_store,
    )));

    // Initialize cron service with file-backed store.
    let cron_store: Arc<dyn moltis_cron::store::CronStore> =
        match moltis_cron::store_file::FileStore::default_path() {
            Ok(fs) => Arc::new(fs),
            Err(e) => {
                tracing::warn!("cron file store unavailable ({e}), using in-memory");
                Arc::new(moltis_cron::store_memory::InMemoryStore::new())
            },
        };

    // Deferred reference: populated once GatewayState is ready.
    let deferred_state: Arc<tokio::sync::OnceCell<Arc<GatewayState>>> =
        Arc::new(tokio::sync::OnceCell::new());

    // System event: inject text into the main session and trigger an agent response.
    let sys_state = Arc::clone(&deferred_state);
    let on_system_event: moltis_cron::service::SystemEventFn = Arc::new(move |text| {
        let st = Arc::clone(&sys_state);
        tokio::spawn(async move {
            if let Some(state) = st.get() {
                let chat = state.chat().await;
                let params = serde_json::json!({ "text": text });
                if let Err(e) = chat.send(params).await {
                    tracing::error!("cron system event failed: {e}");
                }
            }
        });
    });

    // Agent turn: run an LLM turn in a session determined by the job's session_target.
    let agent_state = Arc::clone(&deferred_state);
    let on_agent_turn: moltis_cron::service::AgentTurnFn = Arc::new(move |req| {
        let st = Arc::clone(&agent_state);
        Box::pin(async move {
            let state = st
                .get()
                .ok_or_else(|| anyhow::anyhow!("gateway not ready"))?;

            // OpenClaw-style cost guard: if HEARTBEAT.md exists but is effectively
            // empty (comments/blank scaffold) and there's no explicit
            // heartbeat.prompt override, skip the LLM turn entirely.
            let is_heartbeat_turn = matches!(
                &req.session_target,
                moltis_cron::types::SessionTarget::Named(name) if name == "heartbeat"
            );
            if is_heartbeat_turn {
                let hb_cfg = state.inner.read().await.heartbeat_config.clone();
                let has_prompt_override = hb_cfg
                    .prompt
                    .as_deref()
                    .is_some_and(|p| !p.trim().is_empty());
                let heartbeat_path = moltis_config::heartbeat_path();
                let heartbeat_file_exists = heartbeat_path.exists();
                let heartbeat_md = moltis_config::load_heartbeat_md();
                if heartbeat_file_exists && heartbeat_md.is_none() && !has_prompt_override {
                    tracing::info!(
                        path = %heartbeat_path.display(),
                        "skipping heartbeat LLM turn: HEARTBEAT.md is empty"
                    );
                    return Ok(moltis_cron::service::AgentTurnResult {
                        output: moltis_cron::heartbeat::HEARTBEAT_OK.to_string(),
                        input_tokens: None,
                        output_tokens: None,
                    });
                }
            }

            let chat = state.chat().await;
            let session_key = match &req.session_target {
                moltis_cron::types::SessionTarget::Named(name) => {
                    format!("cron:{name}")
                },
                _ => format!("cron:{}", uuid::Uuid::new_v4()),
            };

            // Clear session history for named cron sessions before execution
            // so the run starts fresh but the history remains readable for debugging.
            if matches!(
                req.session_target,
                moltis_cron::types::SessionTarget::Named(_)
            ) {
                let _ = chat
                    .clear(serde_json::json!({ "_session_key": session_key }))
                    .await;
            }

            // Apply sandbox overrides for this cron session.
            if let Some(ref router) = state.sandbox_router {
                router.set_override(&session_key, req.sandbox.enabled).await;
                if let Some(ref image) = req.sandbox.image {
                    router.set_image_override(&session_key, image.clone()).await;
                }
            }

            let mut params = serde_json::json!({
                "text": req.message,
                "_session_key": session_key,
            });
            if let Some(ref model) = req.model {
                params["model"] = serde_json::Value::String(model.clone());
            }
            let result = chat.send_sync(params).await.map_err(|e| anyhow::anyhow!(e));

            // Clean up sandbox overrides.
            if let Some(ref router) = state.sandbox_router {
                router.remove_override(&session_key).await;
            }

            let val = result?;
            let input_tokens = val.get("inputTokens").and_then(|v| v.as_u64());
            let output_tokens = val.get("outputTokens").and_then(|v| v.as_u64());
            let text = val
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Ok(moltis_cron::service::AgentTurnResult {
                output: text,
                input_tokens,
                output_tokens,
            })
        })
    });

    // Build cron notification callback that broadcasts job changes.
    let deferred_for_cron = Arc::clone(&deferred_state);
    let on_cron_notify: moltis_cron::service::NotifyFn =
        Arc::new(move |notification: moltis_cron::types::CronNotification| {
            let state_opt = deferred_for_cron.get();
            let Some(state) = state_opt else {
                return;
            };
            let (event, payload) = match &notification {
                moltis_cron::types::CronNotification::Created { job } => {
                    ("cron.job.created", serde_json::json!({ "job": job }))
                },
                moltis_cron::types::CronNotification::Updated { job } => {
                    ("cron.job.updated", serde_json::json!({ "job": job }))
                },
                moltis_cron::types::CronNotification::Removed { job_id } => {
                    ("cron.job.removed", serde_json::json!({ "jobId": job_id }))
                },
            };
            // Spawn async broadcast in a background task since we're in a sync callback.
            let state = Arc::clone(state);
            tokio::spawn(async move {
                crate::broadcast::broadcast(
                    &state,
                    event,
                    payload,
                    crate::broadcast::BroadcastOpts {
                        drop_if_slow: true,
                        ..Default::default()
                    },
                )
                .await;
            });
        });

    // Build rate limit config from moltis config.
    let rate_limit_config = moltis_cron::service::RateLimitConfig {
        max_per_window: config.cron.rate_limit_max,
        window_ms: config.cron.rate_limit_window_secs * 1000,
    };

    let cron_service = moltis_cron::service::CronService::with_config(
        cron_store,
        on_system_event,
        on_agent_turn,
        Some(on_cron_notify),
        rate_limit_config,
    );

    // Wire cron into gateway services.
    let live_cron = Arc::new(crate::cron::LiveCronService::new(Arc::clone(&cron_service)));
    services = services.with_cron(live_cron);

    // Build sandbox router from config (shared across sessions).
    let mut sandbox_config = moltis_tools::sandbox::SandboxConfig::from(&config.tools.exec.sandbox);
    sandbox_config.container_prefix = Some(sandbox_container_prefix);
    sandbox_config.timezone = config
        .user
        .timezone
        .as_ref()
        .map(|tz| tz.name().to_string());
    let sandbox_router = Arc::new(moltis_tools::sandbox::SandboxRouter::new(sandbox_config));

    // Spawn background image pre-build. This bakes configured packages into a
    // container image so container creation is instant. Backends that don't
    // support image building return Ok(None) and the spawn is harmless.
    {
        let router = Arc::clone(&sandbox_router);
        let backend = Arc::clone(router.backend());
        let packages = router.config().packages.clone();
        let base_image = router
            .config()
            .image
            .clone()
            .unwrap_or_else(|| moltis_tools::sandbox::DEFAULT_SANDBOX_IMAGE.to_string());

        if should_prebuild_sandbox_image(router.mode(), &packages) {
            let deferred_for_build = Arc::clone(&deferred_state);
            tokio::spawn(async move {
                // Broadcast build start event.
                if let Some(state) = deferred_for_build.get() {
                    crate::broadcast::broadcast(
                        state,
                        "sandbox.image.build",
                        serde_json::json!({ "phase": "start", "packages": packages }),
                        crate::broadcast::BroadcastOpts {
                            drop_if_slow: true,
                            ..Default::default()
                        },
                    )
                    .await;
                }

                match backend.build_image(&base_image, &packages).await {
                    Ok(Some(result)) => {
                        info!(
                            tag = %result.tag,
                            built = result.built,
                            "sandbox image pre-build complete"
                        );
                        router.set_global_image(Some(result.tag.clone())).await;

                        if let Some(state) = deferred_for_build.get() {
                            crate::broadcast::broadcast(
                                state,
                                "sandbox.image.build",
                                serde_json::json!({
                                    "phase": "done",
                                    "tag": result.tag,
                                    "built": result.built,
                                }),
                                crate::broadcast::BroadcastOpts {
                                    drop_if_slow: true,
                                    ..Default::default()
                                },
                            )
                            .await;
                        }
                    },
                    Ok(None) => {
                        debug!(
                            "sandbox image pre-build: no-op (no packages or unsupported backend)"
                        );
                    },
                    Err(e) => {
                        tracing::warn!("sandbox image pre-build failed: {e}");
                        if let Some(state) = deferred_for_build.get() {
                            crate::broadcast::broadcast(
                                state,
                                "sandbox.image.build",
                                serde_json::json!({
                                    "phase": "error",
                                    "error": e.to_string(),
                                }),
                                crate::broadcast::BroadcastOpts {
                                    drop_if_slow: true,
                                    ..Default::default()
                                },
                            )
                            .await;
                        }
                    },
                }
            });
        }
    }

    // When no container runtime is available and the host is Debian/Ubuntu,
    // install the configured sandbox packages directly on the host in the background.
    {
        let packages = sandbox_router.config().packages.clone();
        if sandbox_router.backend_name() == "none"
            && !packages.is_empty()
            && moltis_tools::sandbox::is_debian_host()
        {
            let deferred_for_host = Arc::clone(&deferred_state);
            let pkg_count = packages.len();
            tokio::spawn(async move {
                if let Some(state) = deferred_for_host.get() {
                    broadcast(
                        state,
                        "sandbox.host.provision",
                        serde_json::json!({
                            "phase": "start",
                            "count": pkg_count,
                        }),
                        BroadcastOpts {
                            drop_if_slow: true,
                            ..Default::default()
                        },
                    )
                    .await;
                }

                match moltis_tools::sandbox::provision_host_packages(&packages).await {
                    Ok(Some(result)) => {
                        info!(
                            installed = result.installed.len(),
                            skipped = result.skipped.len(),
                            sudo = result.used_sudo,
                            "host package provisioning complete"
                        );
                        if let Some(state) = deferred_for_host.get() {
                            broadcast(
                                state,
                                "sandbox.host.provision",
                                serde_json::json!({
                                    "phase": "done",
                                    "installed": result.installed.len(),
                                    "skipped": result.skipped.len(),
                                }),
                                BroadcastOpts {
                                    drop_if_slow: true,
                                    ..Default::default()
                                },
                            )
                            .await;
                        }
                    },
                    Ok(None) => {
                        debug!("host package provisioning: no-op (not debian or empty packages)");
                    },
                    Err(e) => {
                        warn!("host package provisioning failed: {e}");
                        if let Some(state) = deferred_for_host.get() {
                            broadcast(
                                state,
                                "sandbox.host.provision",
                                serde_json::json!({
                                    "phase": "error",
                                    "error": e.to_string(),
                                }),
                                BroadcastOpts {
                                    drop_if_slow: true,
                                    ..Default::default()
                                },
                            )
                            .await;
                        }
                    },
                }
            });
        }
    }

    // Pre-pull browser container image if browser is enabled and sandbox mode is available.
    // Browser sandbox mode follows session sandbox mode, so we pre-pull if sandboxing is available.
    // Don't pre-pull if sandbox is disabled (mode = Off).
    if config.tools.browser.enabled
        && !matches!(
            sandbox_router.config().mode,
            moltis_tools::sandbox::SandboxMode::Off
        )
        && sandbox_router.backend_name() != "none"
    {
        let sandbox_image = config.tools.browser.sandbox_image.clone();
        let deferred_for_browser = Arc::clone(&deferred_state);
        tokio::spawn(async move {
            // Broadcast pull start event.
            if let Some(state) = deferred_for_browser.get() {
                crate::broadcast::broadcast(
                    state,
                    "browser.image.pull",
                    serde_json::json!({
                        "phase": "start",
                        "image": sandbox_image,
                    }),
                    crate::broadcast::BroadcastOpts {
                        drop_if_slow: true,
                        ..Default::default()
                    },
                )
                .await;
            }

            match moltis_browser::container::ensure_image(&sandbox_image) {
                Ok(()) => {
                    info!(image = %sandbox_image, "browser container image ready");
                    if let Some(state) = deferred_for_browser.get() {
                        crate::broadcast::broadcast(
                            state,
                            "browser.image.pull",
                            serde_json::json!({
                                "phase": "done",
                                "image": sandbox_image,
                            }),
                            crate::broadcast::BroadcastOpts {
                                drop_if_slow: true,
                                ..Default::default()
                            },
                        )
                        .await;
                    }
                },
                Err(e) => {
                    tracing::warn!(image = %sandbox_image, error = %e, "browser container image pull failed");
                    if let Some(state) = deferred_for_browser.get() {
                        crate::broadcast::broadcast(
                            state,
                            "browser.image.pull",
                            serde_json::json!({
                                "phase": "error",
                                "image": sandbox_image,
                                "error": e.to_string(),
                            }),
                            crate::broadcast::BroadcastOpts {
                                drop_if_slow: true,
                                ..Default::default()
                            },
                        )
                        .await;
                    }
                },
            }
        });
    }

    // Load any persisted sandbox overrides from session metadata.
    {
        for entry in session_metadata.list().await {
            if let Some(enabled) = entry.sandbox_enabled {
                sandbox_router.set_override(&entry.key, enabled).await;
            }
            if let Some(ref image) = entry.sandbox_image {
                sandbox_router
                    .set_image_override(&entry.key, image.clone())
                    .await;
            }
        }
    }

    // Session service is wired after hook registry is built (below).

    // Wire channel store and Telegram channel service.
    {
        use moltis_channels::store::ChannelStore;

        let channel_store: Arc<dyn ChannelStore> = Arc::new(
            crate::channel_store::SqliteChannelStore::new(db_pool.clone()),
        );

        let channel_sink = Arc::new(crate::channel_events::GatewayChannelEventSink::new(
            Arc::clone(&deferred_state),
        ));
        let mut tg_plugin = moltis_telegram::TelegramPlugin::new()
            .with_message_log(Arc::clone(&message_log))
            .with_event_sink(channel_sink);

        // Start channels from config file (these take precedence).
        let tg_accounts = &config.channels.telegram;
        let mut started: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (account_id, account_config) in tg_accounts {
            if let Err(e) = tg_plugin
                .start_account(account_id, account_config.clone())
                .await
            {
                tracing::warn!(account_id, "failed to start telegram account: {e}");
            } else {
                started.insert(account_id.clone());
            }
        }

        // Load persisted channels that weren't in the config file.
        match channel_store.list().await {
            Ok(stored) => {
                info!("{} stored channel(s) found in database", stored.len());
                for ch in stored {
                    if started.contains(&ch.account_id) {
                        info!(
                            account_id = ch.account_id,
                            "skipping stored channel (already started from config)"
                        );
                        continue;
                    }
                    info!(
                        account_id = ch.account_id,
                        channel_type = ch.channel_type,
                        "starting stored channel"
                    );
                    if let Err(e) = tg_plugin.start_account(&ch.account_id, ch.config).await {
                        tracing::warn!(
                            account_id = ch.account_id,
                            "failed to start stored telegram account: {e}"
                        );
                    } else {
                        started.insert(ch.account_id);
                    }
                }
            },
            Err(e) => {
                tracing::warn!("failed to load stored channels: {e}");
            },
        }

        if !started.is_empty() {
            info!("{} telegram account(s) started", started.len());
        }

        // Grab shared outbound before moving tg_plugin into the channel service.
        let tg_outbound = tg_plugin.shared_outbound();
        services = services.with_channel_outbound(tg_outbound);

        services.channel = Arc::new(crate::channel::LiveChannelService::new(
            tg_plugin,
            channel_store,
            Arc::clone(&message_log),
            Arc::clone(&session_metadata),
        ));
    }

    services = services.with_session_metadata(Arc::clone(&session_metadata));
    services = services.with_session_store(Arc::clone(&session_store));

    // ── Hook discovery & registration ─────────────────────────────────────
    seed_default_workspace_markdown_files();
    seed_example_skill();
    seed_example_hook();
    let persisted_disabled = crate::methods::load_disabled_hooks();
    let (hook_registry, discovered_hooks_info) =
        discover_and_build_hooks(&persisted_disabled, Some(&session_store)).await;

    // Wire live session service with sandbox router, project store, hooks, and browser.
    {
        let mut session_svc =
            LiveSessionService::new(Arc::clone(&session_store), Arc::clone(&session_metadata))
                .with_sandbox_router(Arc::clone(&sandbox_router))
                .with_project_store(Arc::clone(&project_store))
                .with_state_store(Arc::clone(&session_state_store))
                .with_browser_service(Arc::clone(&services.browser));
        if let Some(ref hooks) = hook_registry {
            session_svc = session_svc.with_hooks(Arc::clone(hooks));
        }
        services.session = Arc::new(session_svc);
    }

    // ── Memory system initialization ─────────────────────────────────────
    let memory_manager: Option<Arc<moltis_memory::manager::MemoryManager>> = {
        // Build embedding provider(s) for the fallback chain.
        let mut embedding_providers: Vec<(
            String,
            Box<dyn moltis_memory::embeddings::EmbeddingProvider>,
        )> = Vec::new();

        let mem_cfg = &config.memory;

        // 1. If user explicitly configured an embedding provider, use it.
        if let Some(ref provider_name) = mem_cfg.provider {
            match provider_name.as_str() {
                "local" => {
                    // Local GGUF embeddings require the `local-embeddings` feature on moltis-memory.
                    #[cfg(feature = "local-embeddings")]
                    {
                        let cache_dir = mem_cfg
                            .base_url
                            .as_ref()
                            .map(std::path::PathBuf::from)
                            .unwrap_or_else(
                                moltis_memory::embeddings_local::LocalGgufEmbeddingProvider::default_cache_dir,
                            );
                        match moltis_memory::embeddings_local::LocalGgufEmbeddingProvider::ensure_model(
                            cache_dir,
                        )
                        .await
                        {
                            Ok(path) => {
                                match moltis_memory::embeddings_local::LocalGgufEmbeddingProvider::new(
                                    path,
                                ) {
                                    Ok(p) => embedding_providers.push(("local-gguf".into(), Box::new(p))),
                                    Err(e) => warn!("memory: failed to load local GGUF model: {e}"),
                                }
                            },
                            Err(e) => warn!("memory: failed to ensure local model: {e}"),
                        }
                    }
                    #[cfg(not(feature = "local-embeddings"))]
                    warn!(
                        "memory: 'local' embedding provider requires the 'local-embeddings' feature"
                    );
                },
                "ollama" | "custom" | "openai" => {
                    let base_url = mem_cfg
                        .base_url
                        .clone()
                        .unwrap_or_else(|| match provider_name.as_str() {
                            "ollama" => "http://localhost:11434".into(),
                            _ => "https://api.openai.com".into(),
                        });
                    if provider_name == "ollama" {
                        let model = mem_cfg.model.as_deref().unwrap_or("nomic-embed-text");
                        ensure_ollama_model(&base_url, model).await;
                    }
                    let api_key = mem_cfg
                        .api_key
                        .as_ref()
                        .map(|k| k.expose_secret().clone())
                        .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                        .unwrap_or_default();
                    let mut e =
                        moltis_memory::embeddings_openai::OpenAiEmbeddingProvider::new(api_key);
                    if base_url != "https://api.openai.com" {
                        e = e.with_base_url(base_url);
                    }
                    if let Some(ref model) = mem_cfg.model {
                        // Use a sensible default dims; the API returns the actual dims.
                        e = e.with_model(model.clone(), 1536);
                    }
                    embedding_providers.push((provider_name.clone(), Box::new(e)));
                },
                other => warn!("memory: unknown embedding provider '{other}'"),
            }
        }

        // 2. Auto-detect: try Ollama health check.
        if embedding_providers.is_empty() {
            let ollama_ok = reqwest::Client::new()
                .get("http://localhost:11434/api/tags")
                .timeout(std::time::Duration::from_secs(2))
                .send()
                .await
                .is_ok();
            if ollama_ok {
                ensure_ollama_model("http://localhost:11434", "nomic-embed-text").await;
                let e =
                    moltis_memory::embeddings_openai::OpenAiEmbeddingProvider::new(String::new())
                        .with_base_url("http://localhost:11434".into())
                        .with_model("nomic-embed-text".into(), 768);
                embedding_providers.push(("ollama".into(), Box::new(e)));
                info!("memory: detected Ollama at localhost:11434");
            }
        }

        // 3. Auto-detect: try remote API-key providers.
        const EMBEDDING_CANDIDATES: &[(&str, &str, &str)] = &[
            ("openai", "OPENAI_API_KEY", "https://api.openai.com"),
            ("mistral", "MISTRAL_API_KEY", "https://api.mistral.ai/v1"),
            (
                "openrouter",
                "OPENROUTER_API_KEY",
                "https://openrouter.ai/api/v1",
            ),
            ("groq", "GROQ_API_KEY", "https://api.groq.com/openai"),
            ("xai", "XAI_API_KEY", "https://api.x.ai"),
            ("deepseek", "DEEPSEEK_API_KEY", "https://api.deepseek.com"),
            ("cerebras", "CEREBRAS_API_KEY", "https://api.cerebras.ai/v1"),
            ("minimax", "MINIMAX_API_KEY", "https://api.minimax.chat/v1"),
            ("moonshot", "MOONSHOT_API_KEY", "https://api.moonshot.ai/v1"),
            ("venice", "VENICE_API_KEY", "https://api.venice.ai/api/v1"),
        ];

        for (config_name, env_key, default_base) in EMBEDDING_CANDIDATES {
            let key = effective_providers
                .get(config_name)
                .and_then(|e| e.api_key.as_ref().map(|k| k.expose_secret().clone()))
                .or_else(|| std::env::var(env_key).ok())
                .filter(|k| !k.is_empty());
            if let Some(api_key) = key {
                let base = effective_providers
                    .get(config_name)
                    .and_then(|e| e.base_url.clone())
                    .unwrap_or_else(|| default_base.to_string());
                let mut e = moltis_memory::embeddings_openai::OpenAiEmbeddingProvider::new(api_key);
                if base != "https://api.openai.com" {
                    e = e.with_base_url(base);
                }
                embedding_providers.push((config_name.to_string(), Box::new(e)));
            }
        }

        // Build the final embedder: fallback chain, single provider, or keyword-only.
        let embedder: Option<Box<dyn moltis_memory::embeddings::EmbeddingProvider>> =
            if embedding_providers.is_empty() {
                info!("memory: no embedding provider found, using keyword-only search");
                None
            } else {
                let names: Vec<&str> = embedding_providers
                    .iter()
                    .map(|(n, _)| n.as_str())
                    .collect();
                if embedding_providers.len() == 1 {
                    if let Some((name, provider)) = embedding_providers.into_iter().next() {
                        info!(provider = %name, "memory: using single embedding provider");
                        Some(provider)
                    } else {
                        None
                    }
                } else {
                    info!(providers = ?names, active = names[0], "memory: fallback chain configured");
                    Some(Box::new(
                        moltis_memory::embeddings_fallback::FallbackEmbeddingProvider::new(
                            embedding_providers,
                        ),
                    ))
                }
            };

        let memory_db_path = data_dir.join("memory.db");
        let memory_db_url = format!("sqlite:{}?mode=rwc", memory_db_path.display());
        match sqlx::SqlitePool::connect(&memory_db_url).await {
            Ok(memory_pool) => {
                if let Err(e) = moltis_memory::schema::run_migrations(&memory_pool).await {
                    tracing::warn!("memory migration failed: {e}");
                    None
                } else {
                    // Scan the data directory for memory files written by the
                    // silent memory turn (MEMORY.md, memory/*.md).
                    let data_memory_file = data_dir.join("MEMORY.md");
                    let data_memory_file_lower = data_dir.join("memory.md");
                    let data_memory_sub = data_dir.join("memory");

                    let config = moltis_memory::config::MemoryConfig {
                        db_path: memory_db_path.to_string_lossy().into(),
                        memory_dirs: vec![
                            data_memory_file,
                            data_memory_file_lower,
                            data_memory_sub,
                        ],
                        ..Default::default()
                    };

                    let store = Box::new(moltis_memory::store_sqlite::SqliteMemoryStore::new(
                        memory_pool,
                    ));
                    let watch_dirs: Vec<_> = config
                        .memory_dirs
                        .iter()
                        .filter(|p| p.is_dir())
                        .cloned()
                        .collect();
                    let manager = Arc::new(if let Some(embedder) = embedder {
                        moltis_memory::manager::MemoryManager::new(config, store, embedder)
                    } else {
                        moltis_memory::manager::MemoryManager::keyword_only(config, store)
                    });

                    // Initial sync + periodic re-sync (15min with watcher, 5min without).
                    let sync_manager = Arc::clone(&manager);
                    tokio::spawn(async move {
                        match sync_manager.sync().await {
                            Ok(report) => {
                                info!(
                                    updated = report.files_updated,
                                    unchanged = report.files_unchanged,
                                    removed = report.files_removed,
                                    errors = report.errors,
                                    cache_hits = report.cache_hits,
                                    cache_misses = report.cache_misses,
                                    "memory: initial sync complete"
                                );
                                match sync_manager.status().await {
                                    Ok(status) => info!(
                                        files = status.total_files,
                                        chunks = status.total_chunks,
                                        db_size = %status.db_size_display(),
                                        model = %status.embedding_model,
                                        "memory: status"
                                    ),
                                    Err(e) => tracing::warn!("memory: failed to get status: {e}"),
                                }
                            },
                            Err(e) => tracing::warn!("memory: initial sync failed: {e}"),
                        }

                        // Start file watcher for real-time sync (if feature enabled).
                        #[cfg(feature = "file-watcher")]
                        {
                            let watcher_manager = Arc::clone(&sync_manager);
                            match moltis_memory::watcher::MemoryFileWatcher::start(watch_dirs) {
                                Ok((_watcher, mut rx)) => {
                                    info!("memory: file watcher started");
                                    tokio::spawn(async move {
                                        while let Some(event) = rx.recv().await {
                                            let path = match &event {
                                                moltis_memory::watcher::WatchEvent::Created(p)
                                                | moltis_memory::watcher::WatchEvent::Modified(p) => {
                                                    Some(p.clone())
                                                },
                                                moltis_memory::watcher::WatchEvent::Removed(p) => {
                                                    // For removed files, trigger a full sync
                                                    if let Err(e) = watcher_manager.sync().await {
                                                        tracing::warn!(
                                                            path = %p.display(),
                                                            error = %e,
                                                            "memory: watcher sync (removal) failed"
                                                        );
                                                    }
                                                    None
                                                },
                                            };
                                            if let Some(path) = path
                                                && let Err(e) =
                                                    watcher_manager.sync_path(&path).await
                                            {
                                                tracing::warn!(
                                                    path = %path.display(),
                                                    error = %e,
                                                    "memory: watcher sync_path failed"
                                                );
                                            }
                                        }
                                    });
                                },
                                Err(e) => {
                                    tracing::warn!("memory: failed to start file watcher: {e}");
                                },
                            }
                        }

                        // Periodic full sync as safety net (longer interval with watcher).
                        #[cfg(feature = "file-watcher")]
                        let interval_secs = 900; // 15 minutes
                        #[cfg(not(feature = "file-watcher"))]
                        let interval_secs = 300; // 5 minutes

                        let mut interval =
                            tokio::time::interval(std::time::Duration::from_secs(interval_secs));
                        interval.tick().await; // skip first immediate tick
                        loop {
                            interval.tick().await;
                            if let Err(e) = sync_manager.sync().await {
                                tracing::warn!("memory: periodic sync failed: {e}");
                            }
                        }
                    });

                    info!(
                        embeddings = manager.has_embeddings(),
                        "memory system initialized"
                    );
                    Some(manager)
                }
            },
            Err(e) => {
                tracing::warn!("memory: failed to open memory.db: {e}");
                None
            },
        }
    };

    let is_localhost =
        matches!(bind, "127.0.0.1" | "::1" | "localhost") || bind.ends_with(".localhost");
    #[cfg(feature = "tls")]
    let tls_active_for_state = config.tls.enabled;
    #[cfg(not(feature = "tls"))]
    let tls_active_for_state = false;

    // Initialize metrics system.
    #[cfg(feature = "metrics")]
    let metrics_handle = {
        let metrics_config = moltis_metrics::MetricsRecorderConfig {
            enabled: config.metrics.enabled,
            prefix: None,
            global_labels: vec![
                ("service".to_string(), "moltis-gateway".to_string()),
                ("version".to_string(), env!("CARGO_PKG_VERSION").to_string()),
            ],
        };
        match moltis_metrics::init_metrics(metrics_config) {
            Ok(handle) => {
                if config.metrics.enabled {
                    info!("Metrics collection enabled");
                }
                Some(handle)
            },
            Err(e) => {
                warn!("Failed to initialize metrics: {e}");
                None
            },
        }
    };

    // Initialize metrics store for persistence.
    #[cfg(feature = "metrics")]
    let metrics_store: Option<Arc<dyn crate::state::MetricsStore>> = {
        let metrics_db_path = data_dir.join("metrics.db");
        match moltis_metrics::SqliteMetricsStore::new(&metrics_db_path).await {
            Ok(store) => {
                info!(
                    "Metrics history store initialized at {}",
                    metrics_db_path.display()
                );
                Some(Arc::new(store))
            },
            Err(e) => {
                warn!("Failed to initialize metrics store: {e}");
                None
            },
        }
    };

    let behind_proxy = std::env::var("MOLTIS_BEHIND_PROXY")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    // Keep a reference to the browser service for periodic cleanup and shutdown.
    let browser_for_lifecycle = Arc::clone(&services.browser);

    let state = GatewayState::with_options(
        resolved_auth,
        services,
        Some(Arc::clone(&sandbox_router)),
        Some(Arc::clone(&credential_store)),
        is_localhost,
        behind_proxy,
        tls_active_for_state,
        hook_registry.clone(),
        memory_manager.clone(),
        port,
        config.server.ws_request_logs,
        deploy_platform.clone(),
        #[cfg(feature = "metrics")]
        metrics_handle,
        #[cfg(feature = "metrics")]
        metrics_store.clone(),
    );

    // Store discovered hook info and disabled set in state for the web UI.
    {
        let mut inner = state.inner.write().await;
        inner.discovered_hooks = discovered_hooks_info;
        inner.disabled_hooks = persisted_disabled;
    }

    // Note: LLM provider registry is available through the ChatService,
    // not stored separately in GatewayState.

    // Generate a one-time setup code if setup is pending and auth is not disabled.
    let setup_code_display =
        if !credential_store.is_setup_complete() && !credential_store.is_auth_disabled() {
            let code = std::env::var("MOLTIS_E2E_SETUP_CODE")
                .unwrap_or_else(|_| crate::auth_routes::generate_setup_code());
            state.inner.write().await.setup_code = Some(secrecy::Secret::new(code.clone()));
            Some(code)
        } else {
            None
        };

    // ── Tailscale Serve/Funnel ─────────────────────────────────────────
    #[cfg(feature = "tailscale")]
    let tailscale_mode: TailscaleMode = {
        // CLI flag overrides config file.
        let mode_str = tailscale_opts
            .as_ref()
            .map(|o| o.mode.clone())
            .unwrap_or_else(|| config.tailscale.mode.clone());
        mode_str.parse().unwrap_or(TailscaleMode::Off)
    };
    #[cfg(feature = "tailscale")]
    let tailscale_reset_on_exit = tailscale_opts
        .as_ref()
        .map(|o| o.reset_on_exit)
        .unwrap_or(config.tailscale.reset_on_exit);

    #[cfg(feature = "tailscale")]
    if tailscale_mode != TailscaleMode::Off {
        validate_tailscale_config(tailscale_mode, bind, credential_store.is_setup_complete())?;
    }

    // Populate the deferred reference so cron callbacks can reach the gateway.
    let _ = deferred_state.set(Arc::clone(&state));

    // Set the state on local-llm service for broadcasting download progress.
    #[cfg(feature = "local-llm")]
    if let Some(svc) = &local_llm_service {
        svc.set_state(Arc::clone(&state));
    }

    // Set the state on model service for broadcasting model update events.
    live_model_service.set_state(Arc::clone(&state));

    // Model support probing is triggered on-demand by the web UI when the
    // user opens the model selector (via the `models.detect_supported` RPC).
    // With dynamic model discovery, automatic probing at startup is too
    // expensive and noisy — non-chat models (image, audio, video) would
    // generate spurious warnings.

    // Store heartbeat config on state for gon data and RPC methods.
    state.inner.write().await.heartbeat_config = config.heartbeat.clone();

    // Wire live chat service (needs state reference, so done after state creation).
    {
        let broadcaster = Arc::new(GatewayApprovalBroadcaster::new(Arc::clone(&state)));
        let env_provider: Arc<dyn EnvVarProvider> = credential_store.clone();
        let exec_tool = moltis_tools::exec::ExecTool::default()
            .with_approval(Arc::clone(&approval_manager), broadcaster)
            .with_sandbox_router(Arc::clone(&sandbox_router))
            .with_env_provider(env_provider);

        let cron_tool = moltis_tools::cron_tool::CronTool::new(Arc::clone(&cron_service));

        let mut tool_registry = moltis_agents::tool_registry::ToolRegistry::new();
        let process_tool = moltis_tools::process::ProcessTool::new()
            .with_sandbox_router(Arc::clone(&sandbox_router));

        let sandbox_packages_tool = moltis_tools::sandbox_packages::SandboxPackagesTool::new()
            .with_sandbox_router(Arc::clone(&sandbox_router));

        tool_registry.register(Box::new(exec_tool));
        tool_registry.register(Box::new(process_tool));
        tool_registry.register(Box::new(sandbox_packages_tool));
        tool_registry.register(Box::new(cron_tool));
        if let Some(t) =
            moltis_tools::web_search::WebSearchTool::from_config(&config.tools.web.search)
        {
            tool_registry.register(Box::new(t));
        }
        if let Some(t) = moltis_tools::web_fetch::WebFetchTool::from_config(&config.tools.web.fetch)
        {
            tool_registry.register(Box::new(t));
        }
        if let Some(t) = moltis_tools::browser::BrowserTool::from_config(&config.tools.browser) {
            tool_registry.register(Box::new(t));
        }

        // Register memory tools if the memory system is available.
        if let Some(ref mm) = memory_manager {
            tool_registry.register(Box::new(moltis_memory::tools::MemorySearchTool::new(
                Arc::clone(mm),
            )));
            tool_registry.register(Box::new(moltis_memory::tools::MemoryGetTool::new(
                Arc::clone(mm),
            )));
        }

        // Register session state tool for per-session persistent KV store.
        tool_registry.register(Box::new(
            moltis_tools::session_state::SessionStateTool::new(Arc::clone(&session_state_store)),
        ));

        // Register built-in voice tools for explicit TTS/STT calls in agents.
        tool_registry.register(Box::new(crate::voice_agent_tools::SpeakTool::new(
            Arc::clone(&state.services.tts),
        )));
        tool_registry.register(Box::new(crate::voice_agent_tools::TranscribeTool::new(
            Arc::clone(&state.services.stt),
        )));

        // Register skill management tools for agent self-extension.
        // Use data_dir so created skills land in the configured workspace root.
        {
            tool_registry.register(Box::new(moltis_tools::skill_tools::CreateSkillTool::new(
                data_dir.clone(),
            )));
            tool_registry.register(Box::new(moltis_tools::skill_tools::UpdateSkillTool::new(
                data_dir.clone(),
            )));
            tool_registry.register(Box::new(moltis_tools::skill_tools::DeleteSkillTool::new(
                data_dir.clone(),
            )));
        }

        // Register branch session tool for session forking.
        tool_registry.register(Box::new(
            moltis_tools::branch_session::BranchSessionTool::new(
                Arc::clone(&session_store),
                Arc::clone(&session_metadata),
            ),
        ));

        // Register location tool for browser geolocation requests.
        let location_requester = Arc::new(GatewayLocationRequester {
            state: Arc::clone(&state),
        });
        tool_registry.register(Box::new(moltis_tools::location::LocationTool::new(
            location_requester,
        )));

        // Register map tool for showing static map images with links.
        tool_registry.register(Box::new(moltis_tools::map::ShowMapTool::new()));

        // Register spawn_agent tool for sub-agent support.
        // The tool gets a snapshot of the current registry (without itself)
        // so sub-agents have access to all other tools.
        if let Some(default_provider) = registry.read().await.first_with_tools() {
            let base_tools = Arc::new(tool_registry.clone_without(&[]));
            let state_for_spawn = Arc::clone(&state);
            let on_spawn_event: moltis_tools::spawn_agent::OnSpawnEvent = Arc::new(move |event| {
                use moltis_agents::runner::RunnerEvent;
                let state = Arc::clone(&state_for_spawn);
                let payload = match &event {
                    RunnerEvent::SubAgentStart { task, model, depth } => {
                        serde_json::json!({
                            "state": "sub_agent_start",
                            "task": task,
                            "model": model,
                            "depth": depth,
                        })
                    },
                    RunnerEvent::SubAgentEnd {
                        task,
                        model,
                        depth,
                        iterations,
                        tool_calls_made,
                    } => serde_json::json!({
                        "state": "sub_agent_end",
                        "task": task,
                        "model": model,
                        "depth": depth,
                        "iterations": iterations,
                        "toolCallsMade": tool_calls_made,
                    }),
                    _ => return, // Only broadcast sub-agent lifecycle events.
                };
                tokio::spawn(async move {
                    broadcast(&state, "chat", payload, BroadcastOpts::default()).await;
                });
            });
            let spawn_tool = moltis_tools::spawn_agent::SpawnAgentTool::new(
                Arc::clone(&registry),
                default_provider,
                base_tools,
            )
            .with_on_event(on_spawn_event);
            tool_registry.register(Box::new(spawn_tool));
        }

        let shared_tool_registry = Arc::new(tokio::sync::RwLock::new(tool_registry));
        let mut chat_service = LiveChatService::new(
            Arc::clone(&registry),
            Arc::clone(&model_store),
            Arc::clone(&state),
            Arc::clone(&session_store),
            Arc::clone(&session_metadata),
        )
        .with_tools(Arc::clone(&shared_tool_registry))
        .with_failover(config.failover.clone());

        if let Some(ref hooks) = state.inner.read().await.hook_registry {
            chat_service = chat_service.with_hooks_arc(Arc::clone(hooks));
        }

        let live_chat = Arc::new(chat_service);
        state.set_chat(live_chat).await;

        // Store registry in the MCP service so runtime mutations auto-sync,
        // and do an initial sync for any servers that already started.
        live_mcp
            .set_tool_registry(Arc::clone(&shared_tool_registry))
            .await;
        crate::mcp_service::sync_mcp_tools(live_mcp.manager(), &shared_tool_registry).await;

        // Log registered tools for debugging.
        let schemas = shared_tool_registry.read().await.list_schemas();
        let tool_names: Vec<&str> = schemas.iter().filter_map(|s| s["name"].as_str()).collect();
        info!(tools = ?tool_names, "agent tools registered");
    }

    // Spawn skill file watcher for hot-reload.
    #[cfg(feature = "file-watcher")]
    {
        let search_paths = moltis_skills::discover::FsSkillDiscoverer::default_paths();
        let watch_dirs: Vec<std::path::PathBuf> =
            search_paths.into_iter().map(|(p, _)| p).collect();
        if let Ok((_watcher, mut rx)) = moltis_skills::watcher::SkillWatcher::start(watch_dirs) {
            let watcher_state = Arc::clone(&state);
            tokio::spawn(async move {
                let _watcher = _watcher; // keep alive
                while let Some(_event) = rx.recv().await {
                    broadcast(
                        &watcher_state,
                        "skills.changed",
                        serde_json::json!({}),
                        BroadcastOpts::default(),
                    )
                    .await;
                }
            });
        }
    }

    // Spawn MCP health polling + auto-restart background task.
    {
        let health_state = Arc::clone(&state);
        let health_mcp = Arc::clone(&live_mcp);
        tokio::spawn(async move {
            crate::mcp_health::run_health_monitor(health_state, health_mcp).await;
        });
    }

    let methods = Arc::new(MethodRegistry::new());

    // Initialize push notification service if the feature is enabled.
    #[cfg(feature = "push-notifications")]
    let push_service: Option<Arc<crate::push::PushService>> = {
        match crate::push::PushService::new(&data_dir).await {
            Ok(svc) => {
                info!("push notification service initialized");
                // Store in GatewayState for use by chat service
                state.set_push_service(Arc::clone(&svc)).await;
                Some(svc)
            },
            Err(e) => {
                tracing::warn!("failed to initialize push notification service: {e}");
                None
            },
        }
    };

    #[cfg_attr(not(feature = "tls"), allow(unused_mut))]
    #[cfg(feature = "push-notifications")]
    let mut app = build_gateway_app(
        Arc::clone(&state),
        Arc::clone(&methods),
        push_service,
        config.server.http_request_logs,
        webauthn_state.clone(),
    );
    #[cfg_attr(not(feature = "tls"), allow(unused_mut))]
    #[cfg(not(feature = "push-notifications"))]
    let mut app = build_gateway_app(
        Arc::clone(&state),
        Arc::clone(&methods),
        config.server.http_request_logs,
        webauthn_state.clone(),
    );

    let addr: SocketAddr = format!("{bind}:{port}").parse()?;

    // Resolve TLS configuration (only when compiled with the `tls` feature).
    #[cfg(feature = "tls")]
    let tls_active = config.tls.enabled;
    #[cfg(not(feature = "tls"))]
    let tls_active = false;

    #[cfg(feature = "tls")]
    let mut ca_cert_path: Option<PathBuf> = None;
    #[cfg(feature = "tls")]
    let mut rustls_config: Option<rustls::ServerConfig> = None;

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
            let mgr = crate::tls::FsCertManager::new()?;
            let (ca, cert, key) = mgr.ensure_certs()?;
            (Some(ca), cert, key)
        } else {
            anyhow::bail!(
                "TLS is enabled but no certificates configured and auto_generate is false"
            );
        };

        ca_cert_path = ca_path.clone();

        let mgr = crate::tls::FsCertManager::new()?;
        rustls_config = Some(mgr.build_rustls_config(&cert_path, &key_path)?);

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
            .map(|ip| std::net::SocketAddr::new(ip, port))
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
        format!("{} methods registered", methods.method_names().len()),
        format!("llm: {}", provider_summary),
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
            mcp_configured_count,
            if mcp_configured_count > 0 {
                " (starting in background)"
            } else {
                ""
            }
        ),
        format!("sandbox: {} backend", sandbox_router.backend_name()),
        format!(
            "config: {}",
            moltis_config::find_or_default_config_path().display()
        ),
        format!("data: {}", data_dir.display()),
    ];
    // Hint about Apple Container on macOS when using Docker.
    #[cfg(target_os = "macos")]
    if sandbox_router.backend_name() == "docker" {
        lines.push(
            "hint: install Apple Container for VM-isolated sandboxing (see docs/sandbox.md)".into(),
        );
    }
    // Warn when no sandbox backend is available.
    if sandbox_router.backend_name() == "none" {
        if moltis_tools::sandbox::is_debian_host() && !sandbox_router.config().packages.is_empty() {
            lines.push(
                "⚠ no container runtime found; installing packages on host in background".into(),
            );
        } else {
            lines.push("⚠ no container runtime found; commands run on host".into());
        }
    }
    // Display setup code if one was generated.
    if let Some(ref code) = setup_code_display {
        lines.push(format!(
            "setup code: {code} (enter this in the browser to set your password)"
        ));
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

    // Spawn shutdown handler:
    // - reset tailscale state on exit (when configured)
    // - give browser pool 5s to shut down gracefully
    // - force process exit to avoid hanging after ctrl-c
    {
        let browser_for_shutdown = Arc::clone(&browser_for_lifecycle);
        #[cfg(feature = "tailscale")]
        let reset_tailscale_on_exit =
            tailscale_mode != TailscaleMode::Off && tailscale_reset_on_exit;
        #[cfg(feature = "tailscale")]
        let ts_mode = tailscale_mode;
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_err() {
                return;
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

    // Spawn periodic update check against latest GitHub release.
    let update_state = Arc::clone(&state);
    let update_repository_url =
        resolve_repository_url(config.server.update_repository_url.as_deref());
    tokio::spawn(async move {
        let latest_release_api_url = match update_repository_url {
            Some(repository_url) => match github_latest_release_api_url(&repository_url) {
                Ok(url) => url,
                Err(e) => {
                    warn!("update checker disabled: {e}");
                    return;
                },
            },
            None => {
                info!("update checker disabled: server.update_repository_url is not configured");
                return;
            },
        };

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
            match fetch_update_availability(&client, &latest_release_api_url, &update_state.version)
                .await
            {
                Ok(next) => {
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
                },
                Err(e) => {
                    warn!("failed to check latest release: {e}");
                },
            }
        }
    });

    // Spawn metrics history collection and broadcast task (every 10 seconds).
    #[cfg(feature = "metrics")]
    {
        let metrics_state = Arc::clone(&state);
        let server_start = std::time::Instant::now();
        tokio::spawn(async move {
            // Load history from persistent store on startup.
            if let Some(ref store) = metrics_state.metrics_store {
                // Load last 7 days of history (max points for charts).
                let seven_days_ago = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64
                    - (7 * 24 * 60 * 60 * 1000);
                match store.load_history(seven_days_ago, 60480).await {
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

                    let point = crate::state::MetricsHistoryPoint {
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

                    // Persist to store if available.
                    if let Some(ref store) = metrics_state.metrics_store
                        && let Err(e) = store.save_point(&point).await
                    {
                        warn!("Failed to persist metrics point: {e}");
                    }

                    // Broadcast metrics update to all connected clients.
                    let payload = crate::state::MetricsUpdatePayload { snapshot, point };
                    if let Ok(payload_json) = serde_json::to_value(&payload) {
                        crate::broadcast::broadcast(
                            &metrics_state,
                            "metrics.update",
                            payload_json,
                            crate::broadcast::BroadcastOpts {
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
                        if let Some(ref store) = metrics_state.metrics_store {
                            // Keep 7 days of history.
                            let cutoff = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as u64
                                - (7 * 24 * 60 * 60 * 1000);
                            match store.cleanup_before(cutoff).await {
                                Ok(deleted) if deleted > 0 => {
                                    info!("Cleaned up {} old metrics points", deleted);
                                },
                                Err(e) => {
                                    warn!("Failed to cleanup old metrics: {e}");
                                },
                                _ => {},
                            }
                        }
                    }
                }
            }
        });
    }

    // Spawn sandbox event broadcast task: forwards provision events to WS clients.
    {
        let event_state = Arc::clone(&state);
        let mut event_rx = sandbox_router.subscribe_events();
        tokio::spawn(async move {
            loop {
                match event_rx.recv().await {
                    Ok(event) => {
                        let (event_name, payload) = match event {
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
                        crate::broadcast::broadcast(
                            &event_state,
                            event_name,
                            payload,
                            crate::broadcast::BroadcastOpts {
                                drop_if_slow: true,
                                ..Default::default()
                            },
                        )
                        .await;
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
                            crate::broadcast::broadcast(
                                &log_state,
                                "logs.entry",
                                payload,
                                crate::broadcast::BroadcastOpts {
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
                        deliver: false,
                        channel: None,
                        to: None,
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
                        deliver: false,
                        channel: None,
                        to: None,
                    },
                    session_target: SessionTarget::Named("heartbeat".into()),
                    delete_after_run: false,
                    enabled: true,
                    system: true,
                    sandbox: moltis_cron::types::CronSandboxConfig {
                        enabled: hb.sandbox_enabled,
                        image: hb.sandbox_image.clone(),
                    },
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

    #[cfg(feature = "tls")]
    if tls_active {
        // Spawn HTTP redirect server on secondary port.
        if let Some(ref ca) = ca_cert_path {
            let http_port = config.tls.http_redirect_port.unwrap_or(port + 1);
            let bind_clone = bind.to_string();
            let ca_clone = ca.clone();
            tokio::spawn(async move {
                if let Err(e) =
                    crate::tls::start_http_redirect_server(&bind_clone, http_port, port, &ca_clone)
                        .await
                {
                    tracing::error!("HTTP redirect server failed: {e}");
                }
            });
        }

        // Run HTTPS server.
        let tls_cfg = rustls_config.expect("rustls config must be set when TLS is active");
        let rustls_cfg = axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(tls_cfg));
        axum_server::bind_rustls(addr, rustls_cfg)
            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
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
        let host = headers
            .get(axum::http::header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !is_same_origin(origin, host) {
            tracing::warn!(origin, host, remote = %addr, "rejected cross-origin WebSocket upgrade");
            return (
                axum::http::StatusCode::FORBIDDEN,
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

/// Extract the client IP from proxy headers, falling back to the direct connection address.
fn extract_ws_client_ip(
    headers: &axum::http::HeaderMap,
    conn_addr: std::net::SocketAddr,
) -> Option<String> {
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

/// Returns `true` when the request carries headers typically set by reverse proxies.
pub(crate) fn has_proxy_headers(headers: &axum::http::HeaderMap) -> bool {
    headers.contains_key("x-forwarded-for")
        || headers.contains_key("x-real-ip")
        || headers.contains_key("cf-connecting-ip")
        || headers.get("forwarded").is_some()
}

/// Returns `true` when `host` (without port) is a loopback name/address.
fn is_loopback_host(host: &str) -> bool {
    // Strip port (IPv6 bracket form, bare IPv6, or simple host:port).
    let name = if host.starts_with('[') {
        // [::1]:port or [::1]
        host.rsplit_once("]:")
            .map_or(host, |(addr, _)| addr)
            .trim_start_matches('[')
            .trim_end_matches(']')
    } else if host.matches(':').count() > 1 {
        // Bare IPv6 like ::1 (multiple colons, no brackets) — no port stripping.
        host
    } else {
        host.rsplit_once(':').map_or(host, |(addr, _)| addr)
    };
    matches!(name, "localhost" | "127.0.0.1" | "::1") || name.ends_with(".localhost")
}

/// Determine whether a connection is a **direct local** connection (no proxy
/// in between).  This is the per-request check used by the three-tier auth
/// model:
///
/// 1. Password set → always require auth
/// 2. No password + local → full access (dev convenience)
/// 3. No password + remote/proxied → onboarding only
///
/// A connection is considered local when **all** of the following hold:
///
/// - `MOLTIS_BEHIND_PROXY` is **not** set (`behind_proxy == false`)
/// - No proxy headers are present (X-Forwarded-For, X-Real-IP, etc.)
/// - The `Host` header resolves to a loopback address (or is absent)
/// - The TCP source IP is loopback
pub(crate) fn is_local_connection(
    headers: &axum::http::HeaderMap,
    remote_addr: std::net::SocketAddr,
    behind_proxy: bool,
) -> bool {
    // Hard override: env var says we're behind a proxy.
    if behind_proxy {
        return false;
    }

    // Proxy headers present → proxied traffic.
    if has_proxy_headers(headers) {
        return false;
    }

    // Host header points to a non-loopback name → likely proxied.
    if let Some(host) = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        && !is_loopback_host(host)
    {
        return false;
    }

    // TCP source must be loopback.
    remote_addr.ip().is_loopback()
}

async fn websocket_header_authenticated(
    headers: &axum::http::HeaderMap,
    credential_store: Option<&Arc<crate::auth::CredentialStore>>,
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

/// Check whether a WebSocket `Origin` header matches the request `Host`.
///
/// Extracts the host portion of the origin URL and compares it to the Host
/// header.  Accepts `localhost`, `127.0.0.1`, and `[::1]` interchangeably
/// so that `http://localhost:8080` matches a Host of `127.0.0.1:8080`.
fn is_same_origin(origin: &str, host: &str) -> bool {
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

    let origin_port = get_port(origin_host);
    let host_port = get_port(host);

    let oh = strip_port(origin_host);
    let hh = strip_port(host);

    // Normalise loopback variants so 127.0.0.1 == localhost == ::1.
    // Subdomains of .localhost (e.g. moltis.localhost) are also loopback per RFC 6761.
    let is_loopback =
        |h: &str| matches!(h, "localhost" | "127.0.0.1" | "::1") || h.ends_with(".localhost");

    (oh == hh || (is_loopback(oh) && is_loopback(hh))) && origin_port == host_port
}

/// SPA fallback: serve `index.html` for any path not matched by an explicit
/// route (assets, ws, health). This lets client-side routing handle `/crons`,
/// `/logs`, etc.
///
/// Injects a `<script>` tag with pre-fetched bootstrap data (channels,
/// sessions, models, projects) so the UI can render synchronously without
/// waiting for the WebSocket handshake — similar to the gon pattern in Rails.
/// All SPA route paths, defined once in Rust and exposed to both
/// askama templates (HTML `href` attributes) and JavaScript via gon.
#[cfg(feature = "web-ui")]
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SpaRoutes {
    chats: &'static str,
    settings: &'static str,
    providers: &'static str,
    security: &'static str,
    identity: &'static str,
    config: &'static str,
    logs: &'static str,
    onboarding: &'static str,
    projects: &'static str,
    skills: &'static str,
    crons: &'static str,
    monitoring: &'static str,
}

#[cfg(feature = "web-ui")]
static SPA_ROUTES: SpaRoutes = SpaRoutes {
    chats: "/chats",
    settings: "/settings",
    providers: "/settings/providers",
    security: "/settings/security",
    identity: "/settings/identity",
    config: "/settings/config",
    logs: "/settings/logs",
    onboarding: "/onboarding",
    projects: "/projects",
    skills: "/skills",
    crons: "/crons",
    monitoring: "/monitoring",
};

/// Server-side data injected into every page as `window.__MOLTIS__`
/// (gon pattern — see CLAUDE.md § Server-Injected Data).
///
/// Add new fields here when the frontend needs data at page load
/// without an async fetch. Fields must not depend on the request
/// (no cookies, no session — use `/api/auth/status` for that).
#[cfg(feature = "web-ui")]
#[derive(serde::Serialize)]
struct GonData {
    identity: moltis_config::ResolvedIdentity,
    port: u16,
    counts: NavCounts,
    crons: Vec<moltis_cron::types::CronJob>,
    cron_status: moltis_cron::types::CronStatus,
    heartbeat_config: moltis_config::schema::HeartbeatConfig,
    heartbeat_runs: Vec<moltis_cron::types::CronRunRecord>,
    voice_enabled: bool,
    /// Non-main git branch name, if running from a git checkout on a
    /// non-default branch. `None` when on `main`/`master` or outside a repo.
    git_branch: Option<String>,
    /// Memory stats snapshot (process RSS + system available/total).
    mem: MemSnapshot,
    /// Cloud deploy platform (e.g. "flyio"), `None` when running locally.
    #[serde(skip_serializing_if = "Option::is_none")]
    deploy_platform: Option<String>,
    /// Availability of newer GitHub release for this running version.
    update: crate::update_check::UpdateAvailability,
    /// Sandbox runtime info so the UI can render sandbox status without
    /// waiting for the auth-protected `/api/bootstrap` endpoint.
    sandbox: SandboxGonInfo,
    /// Central SPA route definitions so JS can read paths from gon
    /// instead of hardcoding them.
    routes: SpaRoutes,
}

/// Sandbox runtime snapshot included in gon data so the settings page
/// can show the correct backend status before bootstrap completes.
#[cfg(feature = "web-ui")]
#[derive(serde::Serialize)]
struct SandboxGonInfo {
    backend: String,
    os: &'static str,
    default_image: String,
}

/// Memory snapshot included in gon data and tick broadcasts.
#[cfg(feature = "web-ui")]
#[derive(serde::Serialize)]
struct MemSnapshot {
    process: u64,
    available: u64,
    total: u64,
}

/// Collect a point-in-time memory snapshot (process RSS + system memory).
#[cfg(feature = "web-ui")]
fn collect_mem_snapshot() -> MemSnapshot {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let pid = sysinfo::get_current_pid().ok();
    if let Some(pid) = pid {
        sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[pid]),
            false,
            sysinfo::ProcessRefreshKind::nothing().with_memory(),
        );
    }
    let process = pid
        .and_then(|p| sys.process(p))
        .map(|p| p.memory())
        .unwrap_or(0);
    let total = sys.total_memory();
    // available_memory() returns 0 on macOS; fall back to total − used.
    let available = match sys.available_memory() {
        0 => total.saturating_sub(sys.used_memory()),
        v => v,
    };
    MemSnapshot {
        process,
        available,
        total,
    }
}

/// Detect the current git branch, returning `None` for `main`/`master` or
/// when not inside a git repository. The result is cached in a `OnceLock`.
#[cfg(feature = "web-ui")]
fn detect_git_branch() -> Option<String> {
    static BRANCH: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    BRANCH
        .get_or_init(|| {
            let repo = gix::discover(".").ok()?;
            let head = repo.head().ok()?;
            let branch = head.referent_name()?.shorten().to_string();
            parse_git_branch(&branch)
        })
        .clone()
}

/// Parse a branch name, returning
/// `None` for default branches (`main`/`master`) or empty/blank output.
#[cfg(feature = "web-ui")]
fn parse_git_branch(raw: &str) -> Option<String> {
    let branch = raw.trim();
    if branch.is_empty() || branch == "main" || branch == "master" {
        None
    } else {
        Some(branch.to_owned())
    }
}

/// Counts shown as badges in the sidebar navigation.
#[cfg(feature = "web-ui")]
#[derive(Debug, Default, serde::Serialize)]
struct NavCounts {
    projects: usize,
    providers: usize,
    channels: usize,
    skills: usize,
    mcp: usize,
    crons: usize,
    hooks: usize,
}

#[cfg(feature = "web-ui")]
async fn build_gon_data(gw: &GatewayState) -> GonData {
    let port = gw.port;
    let identity = gw
        .services
        .onboarding
        .identity_get()
        .await
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    let counts = build_nav_counts(gw).await;
    let (crons, cron_status) = tokio::join!(gw.services.cron.list(), gw.services.cron.status());
    let crons: Vec<moltis_cron::types::CronJob> = crons
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let cron_status: moltis_cron::types::CronStatus = cron_status
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let heartbeat_config = gw.inner.read().await.heartbeat_config.clone();

    // Get heartbeat runs using the fixed heartbeat job ID.
    // This preserves run history across restarts.
    let heartbeat_runs: Vec<moltis_cron::types::CronRunRecord> = gw
        .services
        .cron
        .runs(serde_json::json!({ "id": "__heartbeat__", "limit": 10 }))
        .await
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    let sandbox = if let Some(ref router) = gw.sandbox_router {
        SandboxGonInfo {
            backend: router.backend_name().to_owned(),
            os: std::env::consts::OS,
            default_image: router.default_image().await,
        }
    } else {
        SandboxGonInfo {
            backend: "none".to_owned(),
            os: std::env::consts::OS,
            default_image: moltis_tools::sandbox::DEFAULT_SANDBOX_IMAGE.to_owned(),
        }
    };

    GonData {
        identity,
        port,
        counts,
        crons,
        cron_status,
        heartbeat_config,
        heartbeat_runs,
        voice_enabled: cfg!(feature = "voice"),
        git_branch: detect_git_branch(),
        mem: collect_mem_snapshot(),
        deploy_platform: gw.deploy_platform.clone(),
        update: gw.inner.read().await.update.clone(),
        sandbox,
        routes: SPA_ROUTES.clone(),
    }
}

#[cfg(feature = "web-ui")]
async fn build_nav_counts(gw: &GatewayState) -> NavCounts {
    let (projects, models, channels, mcp, crons) = tokio::join!(
        gw.services.project.list(),
        gw.services.model.list(),
        gw.services.channel.status(),
        gw.services.mcp.list(),
        gw.services.cron.list(),
    );

    let projects = projects
        .ok()
        .and_then(|v| v.as_array().map(|a| a.len()))
        .unwrap_or(0);

    let providers = models
        .ok()
        .and_then(|v| {
            v.as_array().map(|arr| {
                let mut names: std::collections::HashSet<&str> = std::collections::HashSet::new();
                for m in arr {
                    if let Some(p) = m.get("provider").and_then(|p| p.as_str()) {
                        names.insert(p);
                    }
                }
                names.len()
            })
        })
        .unwrap_or(0);

    let channels = channels
        .ok()
        .and_then(|v| {
            v.get("channels")
                .and_then(|c| c.as_array())
                .map(|a| a.len())
        })
        .unwrap_or(0);

    // Count enabled skills from skills manifest only.
    let mut skills = 0usize;
    if let Ok(path) = moltis_skills::manifest::ManifestStore::default_path() {
        let store = moltis_skills::manifest::ManifestStore::new(path);
        if let Ok(m) = store.load() {
            skills = m
                .repos
                .iter()
                .flat_map(|r| &r.skills)
                .filter(|s| s.enabled)
                .count();
        }
    }

    let mcp = mcp
        .ok()
        .and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter(|s| s.get("state").and_then(|s| s.as_str()) == Some("running"))
                    .count()
            })
        })
        .unwrap_or(0);

    // Count enabled user cron jobs (exclude system jobs like heartbeat).
    let crons = crons
        .ok()
        .and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter(|j| {
                        let enabled = j.get("enabled").and_then(|e| e.as_bool()).unwrap_or(false);
                        let system = j.get("system").and_then(|s| s.as_bool()).unwrap_or(false);
                        enabled && !system
                    })
                    .count()
            })
        })
        .unwrap_or(0);

    let hooks = gw.inner.read().await.discovered_hooks.len();

    NavCounts {
        projects,
        providers,
        channels,
        skills,
        mcp,
        crons,
        hooks,
    }
}

#[cfg(feature = "web-ui")]
async fn api_gon_handler(State(state): State<AppState>) -> impl IntoResponse {
    Json(build_gon_data(&state.gateway).await)
}

#[cfg(feature = "web-ui")]
async fn oauth_callback_handler(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let Some(code) = params.get("code") else {
        return (
            StatusCode::BAD_REQUEST,
            Html("<h1>Authentication failed</h1><p>Missing authorization code.</p>".to_string()),
        )
            .into_response();
    };
    let Some(oauth_state) = params.get("state") else {
        return (
            StatusCode::BAD_REQUEST,
            Html("<h1>Authentication failed</h1><p>Missing OAuth state.</p>".to_string()),
        )
            .into_response();
    };

    match state
        .gateway
        .services
        .provider_setup
        .oauth_complete(serde_json::json!({
            "code": code,
            "state": oauth_state,
        }))
        .await
    {
        Ok(_) => {
            let nonce = uuid::Uuid::new_v4().to_string();
            let html = format!(
                "<h1>Authentication successful!</h1><p>You can close this window.</p>\
                 <script nonce=\"{nonce}\">window.close();</script>"
            );
            let csp = format!(
                "default-src 'none'; script-src 'nonce-{nonce}'; style-src 'unsafe-inline'"
            );
            let mut resp = Html(html).into_response();
            if let Ok(val) = csp.parse() {
                resp.headers_mut()
                    .insert(axum::http::header::CONTENT_SECURITY_POLICY, val);
            }
            resp
        },
        Err(e) => {
            tracing::warn!(error = %e, "OAuth callback completion failed");
            (
                StatusCode::BAD_REQUEST,
                Html(
                    "<h1>Authentication failed</h1><p>Could not complete OAuth flow.</p>"
                        .to_string(),
                ),
            )
                .into_response()
        },
    }
}

#[cfg(feature = "web-ui")]
async fn spa_fallback(State(state): State<AppState>, uri: axum::http::Uri) -> impl IntoResponse {
    let path = uri.path();
    if path.starts_with("/assets/") || path.contains('.') {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }

    // Auth redirects are handled by auth_gate middleware. Here we only
    // check onboarding completion for the redirect-to-onboarding logic.
    let onboarded = onboarding_completed(&state.gateway).await;
    if should_redirect_to_onboarding(path, onboarded) {
        return Redirect::to("/onboarding").into_response();
    }
    render_spa_template(&state.gateway, SpaTemplate::Index).await
}

#[cfg(feature = "web-ui")]
async fn onboarding_handler(State(state): State<AppState>) -> impl IntoResponse {
    let onboarded = onboarding_completed(&state.gateway).await;

    if should_redirect_from_onboarding(onboarded) {
        return Redirect::to("/").into_response();
    }

    render_spa_template(&state.gateway, SpaTemplate::Onboarding).await
}

#[cfg(feature = "web-ui")]
async fn login_handler_page(State(state): State<AppState>) -> impl IntoResponse {
    render_spa_template(&state.gateway, SpaTemplate::Login).await
}

#[cfg(feature = "web-ui")]
const SHARE_IMAGE_URL: &str = "https://www.moltis.org/og-social.jpg?v=4";

#[cfg(feature = "web-ui")]
#[derive(Clone, Copy)]
enum SpaTemplate {
    Index,
    Login,
    Onboarding,
}

#[cfg(feature = "web-ui")]
struct ShareMeta {
    title: String,
    description: String,
    site_name: String,
    image_alt: String,
}

#[cfg(feature = "web-ui")]
#[derive(askama::Template)]
#[template(path = "index.html", escape = "html")]
struct IndexHtmlTemplate<'a> {
    build_ts: &'a str,
    asset_prefix: &'a str,
    nonce: &'a str,
    gon_json: &'a str,
    share_title: &'a str,
    share_description: &'a str,
    share_site_name: &'a str,
    share_image_url: &'a str,
    share_image_alt: &'a str,
    routes: &'a SpaRoutes,
}

#[cfg(feature = "web-ui")]
#[derive(askama::Template)]
#[template(path = "login.html", escape = "html")]
struct LoginHtmlTemplate<'a> {
    build_ts: &'a str,
    asset_prefix: &'a str,
    nonce: &'a str,
    page_title: &'a str,
    gon_json: &'a str,
}

#[cfg(feature = "web-ui")]
#[derive(askama::Template)]
#[template(path = "onboarding.html", escape = "html")]
struct OnboardingHtmlTemplate<'a> {
    build_ts: &'a str,
    asset_prefix: &'a str,
    nonce: &'a str,
    page_title: &'a str,
    gon_json: &'a str,
}

#[cfg(feature = "web-ui")]
fn script_safe_json<T: serde::Serialize>(value: &T) -> String {
    let json = match serde_json::to_string(value) {
        Ok(json) => json,
        Err(e) => {
            warn!(error = %e, "failed to serialize gon data for html template");
            "{}".to_owned()
        },
    };
    json.replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

#[cfg(feature = "web-ui")]
fn build_share_meta(identity: &moltis_config::ResolvedIdentity) -> ShareMeta {
    let agent_name = identity_name(identity);
    let user_name = identity
        .user_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty());

    let title = match user_name {
        Some(user_name) => format!("{agent_name}: {user_name} AI assistant"),
        None => format!("{agent_name}: AI assistant"),
    };
    let description = match user_name {
        Some(user_name) => format!(
            "{agent_name} is {user_name}'s personal AI assistant. Multi-provider models, tools, memory, sandboxed execution, and channel access in one Rust binary."
        ),
        None => format!(
            "{agent_name} is a personal AI assistant. Multi-provider models, tools, memory, sandboxed execution, and channel access in one Rust binary."
        ),
    };
    let image_alt = format!("{agent_name} - personal AI assistant");

    ShareMeta {
        title,
        description,
        site_name: agent_name.to_owned(),
        image_alt,
    }
}

#[cfg(feature = "web-ui")]
fn identity_name(identity: &moltis_config::ResolvedIdentity) -> &str {
    let name = identity.name.trim();
    if name.is_empty() {
        "moltis"
    } else {
        name
    }
}

#[cfg(feature = "web-ui")]
async fn render_spa_template(
    gateway: &GatewayState,
    template: SpaTemplate,
) -> axum::response::Response {
    let (build_ts, asset_prefix) = if is_dev_assets() {
        // Dev: bust browser cache by routing through the versioned path with a
        // timestamp that changes every request.  Safari aggressively caches even
        // with no-cache headers, so a changing URL is the only reliable fix.
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        ("dev".to_owned(), format!("/assets/v/{ts}/"))
    } else {
        // Production: inject content-hash versioned URLs for immutable caching
        static HASH: std::sync::LazyLock<String> = std::sync::LazyLock::new(asset_content_hash);
        (HASH.to_string(), format!("/assets/v/{}/", *HASH))
    };

    // Generate a per-request nonce for CSP script-src.
    let nonce = uuid::Uuid::new_v4().to_string();
    let body = match template {
        SpaTemplate::Index => {
            let gon = build_gon_data(gateway).await;
            let share_meta = build_share_meta(&gon.identity);
            let gon_json = script_safe_json(&gon);
            let template = IndexHtmlTemplate {
                build_ts: &build_ts,
                asset_prefix: &asset_prefix,
                nonce: &nonce,
                gon_json: &gon_json,
                share_title: &share_meta.title,
                share_description: &share_meta.description,
                share_site_name: &share_meta.site_name,
                share_image_url: SHARE_IMAGE_URL,
                share_image_alt: &share_meta.image_alt,
                routes: &SPA_ROUTES,
            };
            match template.render() {
                Ok(html) => html,
                Err(e) => {
                    warn!(error = %e, "failed to render index template");
                    String::new()
                },
            }
        },
        SpaTemplate::Login => {
            let gon = build_gon_data(gateway).await;
            let gon_json = script_safe_json(&gon);
            let page_title = identity_name(&gon.identity).to_owned();
            let template = LoginHtmlTemplate {
                build_ts: &build_ts,
                asset_prefix: &asset_prefix,
                nonce: &nonce,
                page_title: &page_title,
                gon_json: &gon_json,
            };
            match template.render() {
                Ok(html) => html,
                Err(e) => {
                    warn!(error = %e, "failed to render login template");
                    String::new()
                },
            }
        },
        SpaTemplate::Onboarding => {
            let gon = build_gon_data(gateway).await;
            let gon_json = script_safe_json(&gon);
            let page_title = format!("{} onboarding", identity_name(&gon.identity));
            let template = OnboardingHtmlTemplate {
                build_ts: &build_ts,
                asset_prefix: &asset_prefix,
                nonce: &nonce,
                page_title: &page_title,
                gon_json: &gon_json,
            };
            match template.render() {
                Ok(html) => html,
                Err(e) => {
                    warn!(error = %e, "failed to render onboarding template");
                    String::new()
                },
            }
        },
    };

    let csp = format!(
        "default-src 'self'; \
         script-src 'self' 'nonce-{nonce}'; \
         style-src 'self' 'unsafe-inline'; \
         img-src 'self' data: blob:; \
         media-src 'self' blob:; \
         font-src 'self'; \
         connect-src 'self' ws: wss:; \
         frame-ancestors 'none'; \
         form-action 'self'; \
         base-uri 'self'; \
         object-src 'none'"
    );

    let mut response = Html(body).into_response();
    let headers = response.headers_mut();
    if let Ok(val) = "no-cache, no-store".parse() {
        headers.insert(axum::http::header::CACHE_CONTROL, val);
    }
    if let Ok(val) = csp.parse() {
        headers.insert(axum::http::header::CONTENT_SECURITY_POLICY, val);
    }
    response
}

/// Redirect non-onboarding pages to `/onboarding` when the wizard isn't done.
///
/// Auth-level redirects (setup required) are handled by `auth_gate` middleware.
/// This only covers the *onboarding wizard* completion check.
#[cfg(feature = "web-ui")]
fn should_redirect_to_onboarding(path: &str, onboarded: bool) -> bool {
    !is_onboarding_path(path) && !onboarded
}

/// Redirect `/onboarding` back to `/` once the wizard is complete.
#[cfg(feature = "web-ui")]
fn should_redirect_from_onboarding(onboarded: bool) -> bool {
    onboarded
}

#[cfg(feature = "web-ui")]
fn is_onboarding_path(path: &str) -> bool {
    path == "/onboarding" || path == "/onboarding/"
}

#[cfg(feature = "web-ui")]
async fn onboarding_completed(gw: &GatewayState) -> bool {
    gw.services
        .onboarding
        .wizard_status()
        .await
        .ok()
        .and_then(|v| v.get("onboarded").and_then(|v| v.as_bool()))
        .unwrap_or(false)
}

/// Serve a session media file (screenshot, audio, etc.).
#[cfg(feature = "web-ui")]
async fn api_session_media_handler(
    Path((session_key, filename)): Path<(String, String)>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let Some(ref store) = state.gateway.services.session_store else {
        return (StatusCode::NOT_FOUND, "session store not available").into_response();
    };
    match store.read_media(&session_key, &filename).await {
        Ok(data) => {
            let content_type = match filename.rsplit('.').next() {
                Some("png") => "image/png",
                Some("jpg" | "jpeg") => "image/jpeg",
                Some("ogg") => "audio/ogg",
                Some("webm") => "audio/webm",
                Some("mp3") => "audio/mpeg",
                _ => "application/octet-stream",
            };
            ([(axum::http::header::CONTENT_TYPE, content_type)], data).into_response()
        },
        Err(_) => (StatusCode::NOT_FOUND, "media file not found").into_response(),
    }
}

#[cfg(feature = "web-ui")]
async fn api_logs_download_handler(State(state): State<AppState>) -> impl IntoResponse {
    use {axum::http::header, tokio_util::io::ReaderStream};

    let Some(path) = state.gateway.services.logs.log_file_path() else {
        return (StatusCode::NOT_FOUND, "log file not available").into_response();
    };
    let file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(_) => return (StatusCode::NOT_FOUND, "log file not found").into_response(),
    };
    let stream = ReaderStream::new(tokio::io::BufReader::new(file));
    let body = axum::body::Body::from_stream(stream);
    let headers = [
        (header::CONTENT_TYPE, "application/x-ndjson"),
        (
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"moltis-logs.jsonl\"",
        ),
    ];
    (headers, body).into_response()
}

#[cfg(feature = "web-ui")]
async fn api_bootstrap_handler(State(state): State<AppState>) -> impl IntoResponse {
    let gw = &state.gateway;
    let (channels, sessions, models, projects, onboarded) = tokio::join!(
        gw.services.channel.status(),
        gw.services.session.list(),
        gw.services.model.list(),
        gw.services.project.list(),
        onboarding_completed(gw),
    );
    let identity = gw.services.agent.identity_get().await.ok();
    let sandbox = if let Some(ref router) = state.gateway.sandbox_router {
        let default_image = router.default_image().await;
        serde_json::json!({
            "backend": router.backend_name(),
            "os": std::env::consts::OS,
            "default_image": default_image,
        })
    } else {
        serde_json::json!({
            "backend": "none",
            "os": std::env::consts::OS,
            "default_image": moltis_tools::sandbox::DEFAULT_SANDBOX_IMAGE,
        })
    };
    let counts = build_nav_counts(gw).await;
    Json(serde_json::json!({
        "channels": channels.ok(),
        "sessions": sessions.ok(),
        "models": models.ok(),
        "projects": projects.ok(),
        "onboarded": onboarded,
        "identity": identity,
        "sandbox": sandbox,
        "counts": counts,
    }))
}

/// MCP servers list for the UI (HTTP endpoint for initial page load).
#[cfg(feature = "web-ui")]
async fn api_mcp_handler(State(state): State<AppState>) -> impl IntoResponse {
    let servers = state.gateway.services.mcp.list().await;
    match servers {
        Ok(val) => axum::Json(val).into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

/// Hooks list for the UI (HTTP endpoint for initial page load).
#[cfg(feature = "web-ui")]
async fn api_hooks_handler(State(state): State<AppState>) -> impl IntoResponse {
    let hooks = state.gateway.inner.read().await;
    axum::Json(serde_json::json!({ "hooks": hooks.discovered_hooks }))
}

/// Lightweight skills overview: repo summaries + enabled skills only.
/// Full skill lists are loaded on-demand via /api/skills/search.
/// Returns enabled skills from the skills manifest and skill repos.
#[cfg(feature = "web-ui")]
fn enabled_from_manifest(
    path_result: anyhow::Result<std::path::PathBuf>,
) -> Vec<serde_json::Value> {
    let Ok(path) = path_result else {
        return Vec::new();
    };
    let store = moltis_skills::manifest::ManifestStore::new(path);
    store
        .load()
        .map(|m| {
            m.repos
                .iter()
                .flat_map(|repo| {
                    let source = repo.source.clone();
                    repo.skills.iter().filter(|s| s.enabled).map(move |s| {
                        serde_json::json!({
                            "name": s.name,
                            "source": source,
                            "enabled": true,
                        })
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Skills endpoint: repos, enabled registry skills, and discovered personal/project skills.
#[cfg(feature = "web-ui")]
async fn api_skills_handler(State(state): State<AppState>) -> impl IntoResponse {
    let repos = state
        .gateway
        .services
        .skills
        .repos_list()
        .await
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();

    let mut skills = enabled_from_manifest(moltis_skills::manifest::ManifestStore::default_path());

    // Also include discovered Personal and Project skills (not in the manifest).
    {
        use moltis_skills::discover::{FsSkillDiscoverer, SkillDiscoverer};
        let data_dir = moltis_config::data_dir();
        let search_paths = vec![
            (
                data_dir.join("skills"),
                moltis_skills::types::SkillSource::Personal,
            ),
            (
                data_dir.join(".moltis/skills"),
                moltis_skills::types::SkillSource::Project,
            ),
        ];
        let discoverer = FsSkillDiscoverer::new(search_paths);
        if let Ok(discovered) = discoverer.discover().await {
            for s in discovered {
                skills.push(serde_json::json!({
                    "name": s.name,
                    "description": s.description,
                    "source": s.source,
                    "enabled": true,
                }));
            }
        }
    }

    Json(serde_json::json!({ "skills": skills, "repos": repos }))
}

/// Search skills within a specific repo. Query params: source, q (optional).
#[cfg(feature = "web-ui")]
async fn api_search_handler(
    repos: Vec<serde_json::Value>,
    source: &str,
    query: &str,
) -> Json<serde_json::Value> {
    let query = query.to_lowercase();
    let skills: Vec<serde_json::Value> = repos
        .into_iter()
        .find(|repo| {
            repo.get("source")
                .and_then(|s| s.as_str())
                .map(|s| s == source)
                .unwrap_or(false)
        })
        .and_then(|repo| repo.get("skills").and_then(|s| s.as_array()).cloned())
        .unwrap_or_default()
        .into_iter()
        .filter(|skill| {
            if query.is_empty() {
                return true;
            }
            let name = skill
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_lowercase();
            let display = skill
                .get("display_name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_lowercase();
            let desc = skill
                .get("description")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_lowercase();
            name.contains(&query) || display.contains(&query) || desc.contains(&query)
        })
        .take(30)
        .collect();

    Json(serde_json::json!({ "skills": skills }))
}

#[cfg(feature = "web-ui")]
async fn api_skills_search_handler(
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let source = params.get("source").cloned().unwrap_or_default();
    let query = params.get("q").cloned().unwrap_or_default();
    let repos = state
        .gateway
        .services
        .skills
        .repos_list_full()
        .await
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    api_search_handler(repos, &source, &query).await
}

/// List cached tool images.
#[cfg(feature = "web-ui")]
async fn api_cached_images_handler() -> impl IntoResponse {
    let builder = moltis_tools::image_cache::DockerImageBuilder::new();
    match builder.list_cached().await {
        Ok(images) => Json(serde_json::json!({ "images": images })).into_response(),
        Err(e) => {
            let msg = e.to_string();
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response()
        },
    }
}

/// Delete a specific cached tool image.
#[cfg(feature = "web-ui")]
async fn api_delete_cached_image_handler(Path(tag): Path<String>) -> impl IntoResponse {
    let builder = moltis_tools::image_cache::DockerImageBuilder::new();
    // The tag comes URL-encoded; the path captures "moltis-cache/skill:hash" as a single segment.
    let full_tag = if tag.starts_with("moltis-cache/") {
        tag
    } else {
        format!("moltis-cache/{tag}")
    };
    match builder.remove_cached(&full_tag).await {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => {
            let msg = e.to_string();
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response()
        },
    }
}

/// Prune all cached tool images.
#[cfg(feature = "web-ui")]
async fn api_prune_cached_images_handler() -> impl IntoResponse {
    let builder = moltis_tools::image_cache::DockerImageBuilder::new();
    match builder.prune_all().await {
        Ok(count) => Json(serde_json::json!({ "pruned": count })).into_response(),
        Err(e) => {
            let msg = e.to_string();
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response()
        },
    }
}

/// Check which packages already exist in a base image.
///
/// Runs `dpkg -s <pkg>` and `which <pkg>` inside the base image to detect
/// packages that are already installed. Returns a map of package name to
/// boolean (true = already present).
#[cfg(feature = "web-ui")]
async fn api_check_packages_handler(Json(body): Json<serde_json::Value>) -> impl IntoResponse {
    let base = body
        .get("base")
        .and_then(|v| v.as_str())
        .unwrap_or("ubuntu:25.10")
        .trim()
        .to_string();
    let packages: Vec<String> = body
        .get("packages")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();

    if packages.is_empty() {
        return Json(serde_json::json!({ "found": {} })).into_response();
    }

    // Build a shell command that checks each package via dpkg -s or which.
    let checks: Vec<String> = packages
        .iter()
        .map(|pkg| {
            format!(
                r#"if dpkg -s '{pkg}' >/dev/null 2>&1 || command -v '{pkg}' >/dev/null 2>&1; then echo "FOUND:{pkg}"; fi"#
            )
        })
        .collect();
    let script = checks.join("\n");

    let output = tokio::process::Command::new("docker")
        .args(["run", "--rm", "--entrypoint", "sh", &base, "-c", &script])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await;

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let mut found = serde_json::Map::new();
            for pkg in &packages {
                let present = stdout.lines().any(|l| l.trim() == format!("FOUND:{pkg}"));
                found.insert(pkg.clone(), serde_json::Value::Bool(present));
            }
            Json(serde_json::json!({ "found": found })).into_response()
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Get the current default sandbox image.
#[cfg(feature = "web-ui")]
async fn api_get_default_image_handler(State(state): State<AppState>) -> impl IntoResponse {
    let image = if let Some(ref router) = state.gateway.sandbox_router {
        router.default_image().await
    } else {
        moltis_tools::sandbox::DEFAULT_SANDBOX_IMAGE.to_string()
    };
    Json(serde_json::json!({ "image": image }))
}

/// Set the default sandbox image.
#[cfg(feature = "web-ui")]
async fn api_set_default_image_handler(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let image = body.get("image").and_then(|v| v.as_str()).map(|s| s.trim());

    if let Some(ref router) = state.gateway.sandbox_router {
        let value = image.filter(|s| !s.is_empty()).map(String::from);
        router.set_global_image(value.clone()).await;
        let effective = router.default_image().await;
        Json(serde_json::json!({ "image": effective })).into_response()
    } else {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "no sandbox backend available" })),
        )
            .into_response()
    }
}

/// Build a custom image from a base + apt packages.
#[cfg(feature = "web-ui")]
async fn api_build_image_handler(Json(body): Json<serde_json::Value>) -> impl IntoResponse {
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let base = body
        .get("base")
        .and_then(|v| v.as_str())
        .unwrap_or("ubuntu:25.10")
        .trim();
    let packages: Vec<&str> = body
        .get("packages")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    if name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "name is required" })),
        )
            .into_response();
    }
    if packages.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "packages list is empty" })),
        )
            .into_response();
    }

    // Validate name: only allow alphanumeric, dash, underscore
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "name must be alphanumeric, dash, or underscore" })),
        )
            .into_response();
    }

    let pkg_list = packages.join(" ");
    let dockerfile_contents = format!(
        "FROM {base}\n\
RUN apt-get update && apt-get install -y {pkg_list}\n\
RUN mkdir -p /home/sandbox\n\
ENV HOME=/home/sandbox\n\
WORKDIR /home/sandbox\n"
    );

    let tmp_dir = std::env::temp_dir().join(format!("moltis-build-{}", uuid::Uuid::new_v4()));
    if let Err(e) = std::fs::create_dir_all(&tmp_dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    let dockerfile_path = tmp_dir.join("Dockerfile");
    if let Err(e) = std::fs::write(&dockerfile_path, &dockerfile_contents) {
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    let builder = moltis_tools::image_cache::DockerImageBuilder::new();
    let result = builder.ensure_image(name, &dockerfile_path, &tmp_dir).await;
    let _ = std::fs::remove_dir_all(&tmp_dir);
    match result {
        Ok(tag) => Json(serde_json::json!({ "tag": tag })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[cfg(feature = "web-ui")]
static ASSETS: include_dir::Dir = include_dir::include_dir!("$CARGO_MANIFEST_DIR/src/assets");

// ── Asset serving: filesystem (dev) or embedded (release) ───────────────────

/// Filesystem path to serve assets from, if available. Checked once at startup.
/// Set via `MOLTIS_ASSETS_DIR` env var, or auto-detected from the crate source
/// tree when running via `cargo run`.
#[cfg(feature = "web-ui")]
static FS_ASSETS_DIR: std::sync::LazyLock<Option<std::path::PathBuf>> =
    std::sync::LazyLock::new(|| {
        use std::path::PathBuf;

        // Explicit env var takes precedence
        if let Ok(dir) = std::env::var("MOLTIS_ASSETS_DIR") {
            let p = PathBuf::from(dir);
            if p.is_dir() {
                info!("Serving assets from filesystem: {}", p.display());
                return Some(p);
            }
        }

        // Auto-detect: works when running from the repo via `cargo run`
        let cargo_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/assets");
        if cargo_dir.is_dir() {
            info!("Serving assets from filesystem: {}", cargo_dir.display());
            return Some(cargo_dir);
        }

        info!("Serving assets from embedded binary");
        None
    });

/// Whether we're serving from the filesystem (dev mode) or embedded (release).
#[cfg(feature = "web-ui")]
fn is_dev_assets() -> bool {
    FS_ASSETS_DIR.is_some()
}

/// Compute a short content hash of all embedded assets. Only used in release
/// mode (embedded assets) for cache-busting versioned URLs.
#[cfg(feature = "web-ui")]
fn asset_content_hash() -> String {
    use std::{collections::BTreeMap, hash::Hasher};

    let mut files = BTreeMap::new();
    let mut stack: Vec<&include_dir::Dir<'_>> = vec![&ASSETS];
    while let Some(dir) = stack.pop() {
        for file in dir.files() {
            files.insert(file.path().display().to_string(), file.contents());
        }
        for sub in dir.dirs() {
            stack.push(sub);
        }
    }

    let mut h = std::hash::DefaultHasher::new();
    for (path, contents) in &files {
        h.write(path.as_bytes());
        h.write(contents);
    }
    format!("{:016x}", h.finish())
}

#[cfg(feature = "web-ui")]
fn mime_for_path(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "css" => "text/css; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        "mjs" => "application/javascript; charset=utf-8",
        "html" => "text/html; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "json" => "application/json",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        _ => "application/octet-stream",
    }
}

/// Read an asset file, preferring filesystem over embedded.
#[cfg(feature = "web-ui")]
fn read_asset(path: &str) -> Option<Vec<u8>> {
    if let Some(dir) = FS_ASSETS_DIR.as_ref() {
        let file_path = dir.join(path);
        // Prevent path traversal
        if file_path.starts_with(dir)
            && let Ok(bytes) = std::fs::read(&file_path)
        {
            return Some(bytes);
        }
    }
    ASSETS.get_file(path).map(|f| f.contents().to_vec())
}

/// Versioned assets: `/assets/v/<hash>/path` — immutable, cached forever.
#[cfg(feature = "web-ui")]
async fn versioned_asset_handler(
    Path((_version, path)): Path<(String, String)>,
) -> impl IntoResponse {
    let cache = if is_dev_assets() {
        "no-cache, no-store"
    } else {
        "public, max-age=31536000, immutable"
    };
    serve_asset(&path, cache)
}

/// Unversioned assets: `/assets/path` — always revalidate.
#[cfg(feature = "web-ui")]
async fn asset_handler(Path(path): Path<String>) -> impl IntoResponse {
    let cache = if is_dev_assets() {
        "no-cache, no-store"
    } else {
        "no-cache"
    };
    serve_asset(&path, cache)
}

/// PWA manifest: `/manifest.json` — served from assets root.
#[cfg(feature = "web-ui")]
async fn manifest_handler() -> impl IntoResponse {
    serve_asset("manifest.json", "no-cache")
}

/// Service worker: `/sw.js` — served from assets root, no-cache for updates.
#[cfg(feature = "web-ui")]
async fn service_worker_handler() -> impl IntoResponse {
    serve_asset("sw.js", "no-cache")
}

#[cfg(feature = "web-ui")]
fn serve_asset(path: &str, cache_control: &'static str) -> axum::response::Response {
    match read_asset(path) {
        Some(body) => {
            let mut response = (
                StatusCode::OK,
                [
                    ("content-type", mime_for_path(path)),
                    ("cache-control", cache_control),
                    ("x-content-type-options", "nosniff"),
                ],
                body,
            )
                .into_response();

            // Harden SVG delivery against script execution when user-controlled
            // SVGs are ever introduced. Static first-party SVGs continue to render.
            if path.rsplit('.').next().unwrap_or("") == "svg" {
                response.headers_mut().insert(
                    axum::http::header::CONTENT_SECURITY_POLICY,
                    axum::http::HeaderValue::from_static(
                        "default-src 'none'; img-src 'self' data:; style-src 'none'; script-src 'none'; object-src 'none'; frame-ancestors 'none'",
                    ),
                );
            }

            response
        },
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

// ── Hook discovery helper ────────────────────────────────────────────────────

/// Metadata for built-in hooks (compiled Rust, always active).
/// Returns `(name, description, events, source_file)` tuples.
fn builtin_hook_metadata() -> Vec<(
    &'static str,
    &'static str,
    Vec<moltis_common::hooks::HookEvent>,
    &'static str,
)> {
    use moltis_common::hooks::HookEvent;
    vec![
        (
            "boot-md",
            "Reads BOOT.md from the workspace on startup and injects its content as the initial user message to the agent.",
            vec![HookEvent::GatewayStart],
            "crates/plugins/src/bundled/boot_md.rs",
        ),
        (
            "command-logger",
            "Logs all slash-command invocations to a JSONL audit file at ~/.moltis/logs/commands.log.",
            vec![HookEvent::Command],
            "crates/plugins/src/bundled/command_logger.rs",
        ),
        (
            "session-memory",
            "Saves the conversation history to a markdown file in the memory directory when a session is reset or a new session is created, making it searchable for future sessions.",
            vec![HookEvent::Command],
            "crates/plugins/src/bundled/session_memory.rs",
        ),
    ]
}

/// Seed a skeleton example hook into `~/.moltis/hooks/example/` on first run.
///
/// The hook has no command, so it won't execute — it's a template showing
/// users what's possible. If the directory already exists it's a no-op.
fn seed_example_hook() {
    let hook_dir = moltis_config::data_dir().join("hooks/example");
    let hook_md = hook_dir.join("HOOK.md");
    if hook_md.exists() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&hook_dir) {
        tracing::debug!("could not create example hook dir: {e}");
        return;
    }
    if let Err(e) = std::fs::write(&hook_md, EXAMPLE_HOOK_MD) {
        tracing::debug!("could not write example HOOK.md: {e}");
    }
}

/// Seed built-in personal skills into `~/.moltis/skills/`.
///
/// These are safe defaults shipped with the binary. Existing user content
/// is never overwritten.
fn seed_example_skill() {
    seed_skill_if_missing("template-skill", EXAMPLE_SKILL_MD);
    seed_skill_if_missing("tmux", TMUX_SKILL_MD);
}

/// Write a skill's `SKILL.md` into `<data_dir>/skills/<name>/` if it doesn't
/// already exist.
fn seed_skill_if_missing(name: &str, content: &str) {
    let skill_dir = moltis_config::data_dir().join(format!("skills/{name}"));
    let skill_md = skill_dir.join("SKILL.md");
    if skill_md.exists() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&skill_dir) {
        tracing::debug!("could not create {name} skill dir: {e}");
        return;
    }
    if let Err(e) = std::fs::write(&skill_md, content) {
        tracing::debug!("could not write {name} SKILL.md: {e}");
    }
}

/// Seed default workspace markdown files in workspace root on first run.
fn seed_default_workspace_markdown_files() {
    let data_dir = moltis_config::data_dir();
    seed_file_if_missing(data_dir.join("BOOT.md"), DEFAULT_BOOT_MD);
    seed_file_if_missing(data_dir.join("AGENTS.md"), DEFAULT_WORKSPACE_AGENTS_MD);
    seed_file_if_missing(data_dir.join("TOOLS.md"), DEFAULT_TOOLS_MD);
    seed_file_if_missing(data_dir.join("HEARTBEAT.md"), DEFAULT_HEARTBEAT_MD);
}

fn seed_file_if_missing(path: std::path::PathBuf, content: &str) {
    if path.exists() {
        return;
    }
    if let Err(e) = std::fs::write(&path, content) {
        tracing::debug!(path = %path.display(), "could not write default markdown file: {e}");
    }
}

/// Content for the skeleton example hook.
const EXAMPLE_HOOK_MD: &str = r#"+++
name = "example"
description = "Skeleton hook — edit this to build your own"
emoji = "🪝"
events = ["BeforeToolCall"]
# command = "./handler.sh"
# timeout = 10
# priority = 0

# [requires]
# os = ["darwin", "linux"]
# bins = ["jq", "curl"]
# env = ["SLACK_WEBHOOK_URL"]
+++

# Example Hook

This is a skeleton hook to help you get started. It subscribes to
`BeforeToolCall` but has no `command`, so it won't execute anything.

## Quick start

1. Uncomment the `command` line above and point it at your script
2. Create `handler.sh` (or any executable) in this directory
3. Click **Reload** in the Hooks UI (or restart moltis)

## How hooks work

Your script receives the event payload as **JSON on stdin** and communicates
its decision via **exit code** and **stdout**:

| Exit code | Stdout | Action |
|-----------|--------|--------|
| 0 | *(empty)* | **Continue** — let the action proceed |
| 0 | `{"action":"modify","data":{...}}` | **Modify** — alter the payload |
| 1 | *(stderr used as reason)* | **Block** — prevent the action |

## Example handler (bash)

```bash
#!/usr/bin/env bash
# handler.sh — log every tool call to a file
payload=$(cat)
tool=$(echo "$payload" | jq -r '.tool_name // "unknown"')
echo "$(date -Iseconds) tool=$tool" >> /tmp/moltis-hook.log
# Exit 0 with no stdout = Continue
```

## Available events

**Can modify or block (sequential dispatch):**
- `BeforeAgentStart` — before a new agent run begins
- `BeforeToolCall` — before executing a tool (inspect/modify arguments)
- `BeforeCompaction` — before compacting chat history
- `MessageSending` — before sending a message to the LLM
- `ToolResultPersist` — before persisting a tool result

**Read-only (parallel dispatch, Block/Modify ignored):**
- `AgentEnd` — after an agent run completes
- `AfterToolCall` — after a tool finishes (observe result)
- `AfterCompaction` — after compaction completes
- `MessageReceived` — after receiving an LLM response
- `MessageSent` — after a message is sent
- `SessionStart` / `SessionEnd` — session lifecycle
- `GatewayStart` / `GatewayStop` — server lifecycle

## Frontmatter reference

```toml
name = "my-hook"           # unique identifier
description = "What it does"
emoji = "🔧"               # optional, shown in UI
events = ["BeforeToolCall"] # which events to subscribe to
command = "./handler.sh"    # script to run (relative to this dir)
timeout = 10                # seconds before kill (default: 10)
priority = 0                # higher runs first (default: 0)

[requires]
os = ["darwin", "linux"]    # skip on other OSes
bins = ["jq"]               # required binaries in PATH
env = ["MY_API_KEY"]        # required environment variables
```
"#;

/// Content for the starter example personal skill.
const EXAMPLE_SKILL_MD: &str = r#"---
name: template-skill
description: Starter skill template (safe to copy and edit)
---

# Template Skill

Use this as a starting point for your own skills.

## How to use

1. Copy this folder to a new skill name (or edit in place)
2. Update `name` and `description` in frontmatter
3. Replace this body with clear, specific instructions

## Tips

- Keep instructions explicit and task-focused
- Avoid broad permissions unless required
- Document required tools and expected inputs
"#;

/// Content for the built-in tmux skill (interactive terminal processes).
const TMUX_SKILL_MD: &str = r#"---
name: tmux
description: Run and interact with terminal applications (htop, vim, etc.) using tmux sessions in the sandbox
allowed-tools:
  - process
---

# tmux — Interactive Terminal Sessions

Use the `process` tool to run and interact with interactive or long-running
programs inside the sandbox. Every command runs in a named **tmux session**,
giving you full control over TUI apps, REPLs, and background processes.

## When to use this skill

- **TUI / ncurses apps**: htop, vim, nano, less, top, iftop
- **Interactive REPLs**: python3, node, irb, psql, sqlite3
- **Long-running commands**: tail -f, watch, servers, builds
- **Programs that need keyboard input**: anything that waits for keypresses

For simple one-shot commands (ls, cat, echo), use `exec` instead.

## Workflow

1. **Start** a session with a command
2. **Poll** to see the current terminal output
3. **Send keys** or **paste text** to interact
4. **Poll** again to see the result
5. **Kill** when done

Always poll after sending keys — the terminal updates asynchronously.

## Actions

### start — Launch a program

```json
{"action": "start", "command": "htop", "session_name": "my-htop"}
```

- `session_name` is optional (auto-generated if omitted)
- The command runs in a 200x50 terminal

### poll — Read terminal output

```json
{"action": "poll", "session_name": "my-htop"}
```

Returns the visible pane content (what a user would see on screen).

### send_keys — Send keystrokes

```json
{"action": "send_keys", "session_name": "my-htop", "keys": "q"}
```

Common key names:
- `Enter`, `Escape`, `Tab`, `Space`
- `Up`, `Down`, `Left`, `Right`
- `C-c` (Ctrl+C), `C-d` (Ctrl+D), `C-z` (Ctrl+Z)
- `C-l` (clear screen), `C-a` / `C-e` (line start/end)
- Single characters: `q`, `y`, `n`, `/`

### paste — Insert text

```json
{"action": "paste", "session_name": "repl", "text": "print('hello world')\n"}
```

Use paste for multi-character input (code, file content). For single
keystrokes, prefer `send_keys`.

### kill — End a session

```json
{"action": "kill", "session_name": "my-htop"}
```

### list — Show active sessions

```json
{"action": "list"}
```

## Examples

### Run htop and report system load

1. `start` with `"command": "htop"`
2. `poll` to capture the htop display
3. Summarize CPU/memory usage from the output
4. `send_keys` with `"keys": "q"` to quit
5. `kill` the session

### Interactive Python REPL

1. `start` with `"command": "python3"`
2. `paste` with `"text": "2 + 2\n"`
3. `poll` to see the result
4. `send_keys` with `"keys": "C-d"` to exit

### Watch a log file

1. `start` with `"command": "tail -f /var/log/syslog"`, `"session_name": "logs"`
2. `poll` periodically to read new lines
3. `send_keys` with `"keys": "C-c"` when done
4. `kill` the session

## Tips

- Session names must be `[a-zA-Z0-9_-]` only (no spaces or special chars)
- Always `kill` sessions when done to free resources
- If a program is unresponsive, `send_keys` with `C-c` or `C-\` first
- Poll output is a snapshot; poll again for updates after sending input
"#;

/// Default BOOT.md content seeded into workspace root.
const DEFAULT_BOOT_MD: &str = r#"<!--
BOOT.md is optional startup context.

How Moltis uses this file:
- Read on every GatewayStart by the built-in boot-md hook.
- Missing/empty/comment-only file = no startup injection.
- Non-empty content = injected as startup user message context.

Recommended usage:
- Keep it short and explicit.
- Use for startup checks/reminders, not onboarding identity setup.
-->"#;

/// Default workspace AGENTS.md content seeded into workspace root.
const DEFAULT_WORKSPACE_AGENTS_MD: &str = r#"<!--
Workspace AGENTS.md contains global instructions for this workspace.

How Moltis uses this file:
- Loaded from data_dir/AGENTS.md when present.
- Injected as workspace context in the system prompt.
- Separate from project AGENTS.md/CLAUDE.md discovery.

Use this for cross-project rules that should apply everywhere in this workspace.
-->"#;

/// Default TOOLS.md content seeded into workspace root.
const DEFAULT_TOOLS_MD: &str = r#"<!--
TOOLS.md contains workspace-specific tool notes and constraints.

How Moltis uses this file:
- Loaded from data_dir/TOOLS.md when present.
- Injected as workspace context in the system prompt.

Use this for local setup details (hosts, aliases, device names) and
tool behavior constraints (safe defaults, forbidden actions, etc.).
-->"#;

/// Default HEARTBEAT.md content seeded into workspace root.
const DEFAULT_HEARTBEAT_MD: &str = r#"<!--
HEARTBEAT.md is an optional heartbeat prompt source.

Prompt precedence:
1) heartbeat.prompt from config
2) HEARTBEAT.md
3) built-in default prompt

Cost guard:
- If HEARTBEAT.md exists but is empty/comment-only and there is no explicit
  heartbeat.prompt override, Moltis skips heartbeat LLM turns to avoid token use.
-->"#;

/// Discover hooks from the filesystem, check eligibility, and build a
/// [`HookRegistry`] plus a `Vec<DiscoveredHookInfo>` for the web UI.
///
/// Hooks whose names appear in `disabled` are still returned in the info list
/// (with `enabled: false`) but are not registered in the registry.
pub(crate) async fn discover_and_build_hooks(
    disabled: &HashSet<String>,
    session_store: Option<&Arc<moltis_sessions::store::SessionStore>>,
) -> (
    Option<Arc<moltis_common::hooks::HookRegistry>>,
    Vec<crate::state::DiscoveredHookInfo>,
) {
    use moltis_plugins::{
        bundled::{
            boot_md::BootMdHook, command_logger::CommandLoggerHook,
            session_memory::SessionMemoryHook,
        },
        hook_discovery::{FsHookDiscoverer, HookDiscoverer, HookSource},
        hook_eligibility::check_hook_eligibility,
        shell_hook::ShellHookHandler,
    };

    let discoverer = FsHookDiscoverer::new(FsHookDiscoverer::default_paths());
    let discovered = discoverer.discover().await.unwrap_or_default();

    let mut registry = moltis_common::hooks::HookRegistry::new();
    let mut info_list = Vec::with_capacity(discovered.len());

    for (parsed, source) in &discovered {
        let meta = &parsed.metadata;
        let elig = check_hook_eligibility(meta);
        let is_disabled = disabled.contains(&meta.name);
        let is_enabled = elig.eligible && !is_disabled;

        if !elig.eligible {
            info!(
                hook = %meta.name,
                source = ?source,
                missing_os = elig.missing_os,
                missing_bins = ?elig.missing_bins,
                missing_env = ?elig.missing_env,
                "hook ineligible, skipping"
            );
        }

        // Read the raw HOOK.md content for the UI editor.
        let raw_content =
            std::fs::read_to_string(parsed.source_path.join("HOOK.md")).unwrap_or_default();

        let source_str = match source {
            HookSource::Project => "project",
            HookSource::User => "user",
            HookSource::Bundled => "bundled",
        };

        info_list.push(crate::state::DiscoveredHookInfo {
            name: meta.name.clone(),
            description: meta.description.clone(),
            emoji: meta.emoji.clone(),
            events: meta.events.iter().map(|e| e.to_string()).collect(),
            command: meta.command.clone(),
            timeout: meta.timeout,
            priority: meta.priority,
            source: source_str.to_string(),
            source_path: parsed.source_path.display().to_string(),
            eligible: elig.eligible,
            missing_os: elig.missing_os,
            missing_bins: elig.missing_bins.clone(),
            missing_env: elig.missing_env.clone(),
            enabled: is_enabled,
            body: raw_content,
            body_html: crate::services::markdown_to_html(&parsed.body),
            call_count: 0,
            failure_count: 0,
            avg_latency_ms: 0,
        });

        // Only register eligible, non-disabled hooks.
        if is_enabled && let Some(ref command) = meta.command {
            let handler = ShellHookHandler::new(
                meta.name.clone(),
                command.clone(),
                meta.events.clone(),
                std::time::Duration::from_secs(meta.timeout),
                meta.env.clone(),
            );
            registry.register(Arc::new(handler));
        }
    }

    // ── Built-in hooks (compiled Rust, always active) ──────────────────
    {
        let data = moltis_config::data_dir();

        // boot-md: inject BOOT.md content on GatewayStart.
        let boot = BootMdHook::new(data.clone());
        registry.register(Arc::new(boot));

        // command-logger: append JSONL entries for every slash command.
        let log_path =
            CommandLoggerHook::default_path().unwrap_or_else(|| data.join("logs/commands.log"));
        let logger = CommandLoggerHook::new(log_path);
        registry.register(Arc::new(logger));

        // session-memory: save conversation to memory on /new or /reset.
        if let Some(store) = session_store {
            let memory_hook = SessionMemoryHook::new(data.clone(), Arc::clone(store));
            registry.register(Arc::new(memory_hook));
        }
    }

    for (name, description, events, source_file) in builtin_hook_metadata() {
        info_list.push(crate::state::DiscoveredHookInfo {
            name: name.to_string(),
            description: description.to_string(),
            emoji: Some("\u{2699}\u{fe0f}".to_string()), // ⚙️
            events: events.iter().map(|e| e.to_string()).collect(),
            command: None,
            timeout: 0,
            priority: 0,
            source: "builtin".to_string(),
            source_path: source_file.to_string(),
            eligible: true,
            missing_os: false,
            missing_bins: vec![],
            missing_env: vec![],
            enabled: true,
            body: String::new(),
            body_html: format!(
                "<p><em>Built-in hook implemented in Rust.</em></p><p>{}</p>",
                description
            ),
            call_count: 0,
            failure_count: 0,
            avg_latency_ms: 0,
        });
    }

    if !info_list.is_empty() {
        info!(
            "{} hook(s) discovered ({} shell, {} built-in), {} registered",
            info_list.len(),
            discovered.len(),
            info_list.len() - discovered.len(),
            registry.handler_names().len()
        );
    }

    (Some(Arc::new(registry)), info_list)
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, std::collections::HashSet};

    #[test]
    fn approval_manager_uses_config_values() {
        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.tools.exec.approval_mode = "always".into();
        cfg.tools.exec.security_level = "strict".into();
        cfg.tools.exec.allowlist = vec!["git*".into()];

        let manager = approval_manager_from_config(&cfg);
        assert_eq!(manager.mode, ApprovalMode::Always);
        assert_eq!(manager.security_level, SecurityLevel::Deny);
        assert_eq!(manager.allowlist, vec!["git*".to_string()]);
    }

    #[test]
    fn approval_manager_falls_back_for_invalid_values() {
        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.tools.exec.approval_mode = "bogus".into();
        cfg.tools.exec.security_level = "bogus".into();

        let manager = approval_manager_from_config(&cfg);
        assert_eq!(manager.mode, ApprovalMode::OnMiss);
        assert_eq!(manager.security_level, SecurityLevel::Allowlist);
    }

    #[tokio::test]
    async fn discover_hooks_registers_builtin_handlers() {
        let tmp = tempfile::tempdir().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let session_store = Arc::new(moltis_sessions::store::SessionStore::new(sessions_dir));

        let (registry, info) =
            discover_and_build_hooks(&HashSet::new(), Some(&session_store)).await;
        let registry = registry.expect("expected hook registry to be created");
        let handler_names = registry.handler_names();

        assert!(handler_names.iter().any(|n| n == "boot-md"));
        assert!(handler_names.iter().any(|n| n == "command-logger"));
        assert!(handler_names.iter().any(|n| n == "session-memory"));

        assert!(
            info.iter()
                .any(|h| h.name == "boot-md" && h.source == "builtin")
        );
        assert!(
            info.iter()
                .any(|h| h.name == "command-logger" && h.source == "builtin")
        );
        assert!(
            info.iter()
                .any(|h| h.name == "session-memory" && h.source == "builtin")
        );
    }

    #[tokio::test]
    async fn command_hook_dispatch_saves_session_memory_file() {
        let tmp = tempfile::tempdir().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let session_store = Arc::new(moltis_sessions::store::SessionStore::new(sessions_dir));

        session_store
            .append(
                "smoke-session",
                &serde_json::json!({"role": "user", "content": "Hello from smoke test"}),
            )
            .await
            .unwrap();
        session_store
            .append(
                "smoke-session",
                &serde_json::json!({"role": "assistant", "content": "Hi there"}),
            )
            .await
            .unwrap();

        let mut registry = moltis_common::hooks::HookRegistry::new();
        registry.register(Arc::new(
            moltis_plugins::bundled::session_memory::SessionMemoryHook::new(
                tmp.path().to_path_buf(),
                Arc::clone(&session_store),
            ),
        ));

        let payload = moltis_common::hooks::HookPayload::Command {
            session_key: "smoke-session".into(),
            action: "new".into(),
            sender_id: None,
        };
        let result = registry.dispatch(&payload).await.unwrap();
        assert!(matches!(result, moltis_common::hooks::HookAction::Continue));

        let memory_dir = tmp.path().join("memory");
        assert!(memory_dir.is_dir());

        let files: Vec<_> = std::fs::read_dir(&memory_dir).unwrap().flatten().collect();
        assert_eq!(files.len(), 1);

        let content = std::fs::read_to_string(files[0].path()).unwrap();
        assert!(content.contains("smoke-session"));
        assert!(content.contains("Hello from smoke test"));
        assert!(content.contains("Hi there"));
    }

    #[tokio::test]
    async fn websocket_header_auth_accepts_valid_session_cookie() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = Arc::new(
            crate::auth::CredentialStore::with_config(pool, &moltis_config::AuthConfig::default())
                .await
                .unwrap(),
        );
        store.set_initial_password("supersecret").await.unwrap();
        let token = store.create_session().await.unwrap();

        let mut headers = axum::http::HeaderMap::new();
        let cookie = format!("{}={token}", crate::auth_middleware::SESSION_COOKIE);
        headers.insert(axum::http::header::COOKIE, cookie.parse().unwrap());

        assert!(websocket_header_authenticated(&headers, Some(&store), false).await);
    }

    #[tokio::test]
    async fn websocket_header_auth_accepts_valid_bearer_api_key() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = Arc::new(
            crate::auth::CredentialStore::with_config(pool, &moltis_config::AuthConfig::default())
                .await
                .unwrap(),
        );
        store.set_initial_password("supersecret").await.unwrap();
        let (_id, raw_key) = store.create_api_key("ws", None).await.unwrap();

        let mut headers = axum::http::HeaderMap::new();
        let auth_value = format!("Bearer {raw_key}");
        headers.insert(
            axum::http::header::AUTHORIZATION,
            auth_value.parse().unwrap(),
        );

        assert!(websocket_header_authenticated(&headers, Some(&store), false).await);
    }

    #[tokio::test]
    async fn websocket_header_auth_rejects_missing_credentials_when_setup_complete() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = Arc::new(
            crate::auth::CredentialStore::with_config(pool, &moltis_config::AuthConfig::default())
                .await
                .unwrap(),
        );
        store.set_initial_password("supersecret").await.unwrap();
        let headers = axum::http::HeaderMap::new();

        assert!(!websocket_header_authenticated(&headers, Some(&store), false).await);
    }

    /// Regression test for proxy auth bypass: when a password is set, the
    /// local-no-password shortcut must NOT grant access — even when the
    /// connection is local (is_local = true).  Behind a reverse proxy on
    /// the same machine every request appears to come from 127.0.0.1,
    /// so trusting loopback alone would bypass authentication for all
    /// internet traffic.  See CVE-2026-25253 for the analogous OpenClaw
    /// vulnerability.
    #[tokio::test]
    async fn websocket_header_auth_rejects_local_when_password_set() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = Arc::new(
            crate::auth::CredentialStore::with_config(pool, &moltis_config::AuthConfig::default())
                .await
                .unwrap(),
        );
        store.set_initial_password("supersecret").await.unwrap();
        let headers = axum::http::HeaderMap::new();

        // is_local = true but password is set → must reject.
        assert!(
            !websocket_header_authenticated(&headers, Some(&store), true).await,
            "local connection must not bypass auth when a password is configured"
        );
    }

    #[test]
    fn onboarding_redirect_rules() {
        // Onboarding incomplete forces redirect.
        assert!(should_redirect_to_onboarding("/", false));
        assert!(should_redirect_to_onboarding("/chats", false));
        // /onboarding itself is never redirected.
        assert!(!should_redirect_to_onboarding("/onboarding", false));

        // Once onboarded, no redirect is needed.
        assert!(!should_redirect_to_onboarding("/", true));

        // Once onboarding is complete, /onboarding should bounce back to /.
        assert!(should_redirect_from_onboarding(true));
        // Not yet onboarded — stay on /onboarding.
        assert!(!should_redirect_from_onboarding(false));
    }

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

    #[cfg(feature = "web-ui")]
    #[test]
    fn asset_serving_sets_nosniff_header() {
        let response = serve_asset("style.css", "no-cache");
        assert_eq!(response.status(), StatusCode::OK);
        let nosniff = response
            .headers()
            .get("x-content-type-options")
            .and_then(|v| v.to_str().ok());
        assert_eq!(nosniff, Some("nosniff"));
    }

    #[cfg(feature = "web-ui")]
    #[test]
    fn svg_assets_get_restrictive_csp_header() {
        let response = serve_asset("icons/icon-base.svg", "no-cache");
        assert_eq!(response.status(), StatusCode::OK);
        let csp = response
            .headers()
            .get(axum::http::header::CONTENT_SECURITY_POLICY)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(csp.contains("script-src 'none'"));
        assert!(csp.contains("object-src 'none'"));
    }

    #[cfg(feature = "web-ui")]
    #[test]
    fn onboarding_template_uses_dedicated_entrypoint() {
        let template = OnboardingHtmlTemplate {
            build_ts: "dev",
            asset_prefix: "/assets/v/test/",
            nonce: "nonce-123",
            page_title: "sparky onboarding",
            gon_json: "{\"identity\":{\"name\":\"moltis\"},\"voice_enabled\":true}",
        };
        let html = match template.render() {
            Ok(html) => html,
            Err(e) => panic!("failed to render onboarding template: {e}"),
        };
        assert!(html.contains("<title>sparky onboarding</title>"));
        assert!(html.contains(
            "<link rel=\"icon\" type=\"image/png\" sizes=\"96x96\" href=\"/assets/v/test/icons/icon-96.png\">"
        ));
        assert!(html.contains(
            "<link rel=\"icon\" type=\"image/png\" sizes=\"32x32\" href=\"/assets/v/test/icons/icon-72.png\">"
        ));
        assert!(html.contains("/assets/v/test/js/onboarding-app.js"));
        assert!(!html.contains("/assets/v/test/js/app.js"));
        assert!(!html.contains("/manifest.json"));
        assert!(html.contains("<script nonce=\"nonce-123\">"));
        assert!(html.contains(
            "<script nonce=\"nonce-123\">window.__MOLTIS__={\"identity\":{\"name\":\"moltis\"},\"voice_enabled\":true};</script>"
        ));
        assert!(html.contains("<script nonce=\"nonce-123\" type=\"importmap\">"));
        assert!(html.contains(
            "<script nonce=\"nonce-123\" type=\"module\" src=\"/assets/v/test/js/onboarding-app.js\">"
        ));
    }

    #[cfg(feature = "web-ui")]
    #[test]
    fn share_meta_uses_agent_and_user_name() {
        let identity = moltis_config::ResolvedIdentity {
            name: "sparky".to_owned(),
            user_name: Some("penso".to_owned()),
            ..Default::default()
        };

        let meta = build_share_meta(&identity);
        assert_eq!(meta.title, "sparky: penso AI assistant");
        assert!(
            meta.description
                .contains("sparky is penso's personal AI assistant.")
        );
        assert_eq!(meta.site_name, "sparky");
        assert_eq!(meta.image_alt, "sparky - personal AI assistant");
    }

    #[cfg(feature = "web-ui")]
    #[test]
    fn share_meta_falls_back_when_user_name_missing() {
        let identity = moltis_config::ResolvedIdentity {
            name: "moltis".to_owned(),
            user_name: Some("   ".to_owned()),
            ..Default::default()
        };

        let meta = build_share_meta(&identity);
        assert_eq!(meta.title, "moltis: AI assistant");
        assert!(
            meta.description
                .starts_with("moltis is a personal AI assistant.")
        );
        assert_eq!(meta.site_name, "moltis");
    }

    #[cfg(feature = "web-ui")]
    #[test]
    fn share_meta_omits_emoji_in_title() {
        let identity = moltis_config::ResolvedIdentity {
            name: "sparky".to_owned(),
            emoji: Some("\u{1f525}".to_owned()),
            user_name: Some("penso".to_owned()),
            ..Default::default()
        };

        let meta = build_share_meta(&identity);
        assert_eq!(meta.title, "sparky: penso AI assistant");
        assert_eq!(meta.site_name, "sparky");
    }

    #[cfg(feature = "web-ui")]
    #[test]
    fn askama_template_escapes_share_meta_values() {
        let template = IndexHtmlTemplate {
            build_ts: "dev",
            asset_prefix: "/assets/v/test/",
            nonce: "nonce-123",
            gon_json: "{}",
            share_title: "A&B <tag>",
            share_description: "desc <b>safe</b>",
            share_site_name: "moltis",
            share_image_url: SHARE_IMAGE_URL,
            share_image_alt: "preview <image>",
            routes: &SPA_ROUTES,
        };
        let html = match template.render() {
            Ok(html) => html,
            Err(e) => panic!("failed to render askama template: {e}"),
        };
        assert!(html.contains("A&amp;B") || html.contains("A&#38;B"));
        assert!(!html.contains("A&B <tag>"));
        assert!(
            html.contains("desc &#60;b&#62;safe&#60;/b&#62;")
                || html.contains("desc &lt;b&gt;safe&lt;/b&gt;")
        );
        assert!(!html.contains("desc <b>safe</b>"));
        assert!(html.contains("preview &#60;image&#62;") || html.contains("preview &lt;image&gt;"));
        assert!(!html.contains("preview <image>"));
    }

    #[cfg(feature = "web-ui")]
    #[test]
    fn login_template_includes_gon_script_with_nonce() {
        let template = LoginHtmlTemplate {
            build_ts: "dev",
            asset_prefix: "/assets/v/test/",
            nonce: "nonce-abc",
            page_title: "sparky",
            gon_json: "{\"identity\":{\"name\":\"moltis\"}}",
        };
        let html = match template.render() {
            Ok(html) => html,
            Err(e) => panic!("failed to render login template: {e}"),
        };
        assert!(html.contains("<title>sparky</title>"));
        assert!(html.contains(
            "<link rel=\"icon\" type=\"image/png\" sizes=\"96x96\" href=\"/assets/v/test/icons/icon-96.png\">"
        ));
        assert!(html.contains(
            "<link rel=\"icon\" type=\"image/png\" sizes=\"32x32\" href=\"/assets/v/test/icons/icon-72.png\">"
        ));
        assert!(html.contains("<script nonce=\"nonce-abc\">window.__MOLTIS__={\"identity\":{\"name\":\"moltis\"}};</script>"));
        assert!(html.contains(
            "<script nonce=\"nonce-abc\" type=\"module\" src=\"/assets/v/test/js/login-app.js\"></script>"
        ));
    }

    #[cfg(feature = "web-ui")]
    #[test]
    fn script_safe_json_escapes_html_sensitive_chars() {
        let input = serde_json::json!({
            "value": "</script><b>&\u{2028}\u{2029}",
        });
        let json = script_safe_json(&input);
        assert!(json.contains("\\u003c/script\\u003e\\u003cb\\u003e\\u0026\\u2028\\u2029"));
        assert!(!json.contains("</script>"));
        assert!(!json.contains("<b>"));
        assert!(!json.contains("&"));
    }

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

    #[cfg(feature = "web-ui")]
    mod git_branch_tests {
        use super::super::parse_git_branch;

        #[test]
        fn feature_branch_returned() {
            assert_eq!(
                parse_git_branch("top-banner-branch\n"),
                Some("top-banner-branch".to_owned())
            );
        }

        #[test]
        fn main_returns_none() {
            assert_eq!(parse_git_branch("main\n"), None);
        }

        #[test]
        fn master_returns_none() {
            assert_eq!(parse_git_branch("master\n"), None);
        }

        #[test]
        fn empty_returns_none() {
            assert_eq!(parse_git_branch(""), None);
            assert_eq!(parse_git_branch("  \n"), None);
        }

        #[test]
        fn trims_whitespace() {
            assert_eq!(
                parse_git_branch("  feat/my-feature  \n"),
                Some("feat/my-feature".to_owned())
            );
        }
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
        let addr: std::net::SocketAddr = "0.0.0.0:9999".parse().unwrap();
        assert!(addr.ip().is_unspecified());

        if let Some(ip) = resolve_outbound_ip(false) {
            let display = std::net::SocketAddr::new(ip, addr.port());
            assert!(!display.ip().is_unspecified());
            assert_eq!(display.port(), 9999);
        }
    }

    // ── is_local_connection / proxy header detection tests ───────────────

    #[test]
    fn has_proxy_headers_detects_xff() {
        let mut h = axum::http::HeaderMap::new();
        h.insert("x-forwarded-for", "203.0.113.50".parse().unwrap());
        assert!(has_proxy_headers(&h));
    }

    #[test]
    fn has_proxy_headers_detects_x_real_ip() {
        let mut h = axum::http::HeaderMap::new();
        h.insert("x-real-ip", "203.0.113.50".parse().unwrap());
        assert!(has_proxy_headers(&h));
    }

    #[test]
    fn has_proxy_headers_detects_cf_connecting_ip() {
        let mut h = axum::http::HeaderMap::new();
        h.insert("cf-connecting-ip", "203.0.113.50".parse().unwrap());
        assert!(has_proxy_headers(&h));
    }

    #[test]
    fn has_proxy_headers_detects_forwarded() {
        let mut h = axum::http::HeaderMap::new();
        h.insert("forwarded", "for=203.0.113.50".parse().unwrap());
        assert!(has_proxy_headers(&h));
    }

    #[test]
    fn has_proxy_headers_empty() {
        assert!(!has_proxy_headers(&axum::http::HeaderMap::new()));
    }

    #[test]
    fn is_loopback_host_variants() {
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("localhost:18789"));
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("127.0.0.1:18789"));
        assert!(is_loopback_host("::1"));
        assert!(is_loopback_host("[::1]:18789"));
        assert!(is_loopback_host("moltis.localhost"));
        assert!(is_loopback_host("moltis.localhost:8080"));

        assert!(!is_loopback_host("example.com"));
        assert!(!is_loopback_host("example.com:18789"));
        assert!(!is_loopback_host("192.168.1.1:18789"));
        assert!(!is_loopback_host("moltis.example.com"));
    }

    #[test]
    fn local_connection_direct_loopback() {
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::HOST, "localhost:18789".parse().unwrap());
        assert!(is_local_connection(&headers, addr, false));
    }

    #[test]
    fn local_connection_ipv6_loopback() {
        let addr: SocketAddr = "[::1]:12345".parse().unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::HOST, "[::1]:18789".parse().unwrap());
        assert!(is_local_connection(&headers, addr, false));
    }

    #[test]
    fn local_connection_no_host_header() {
        // CLI/SDK clients may not send a Host header.
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let headers = axum::http::HeaderMap::new();
        assert!(is_local_connection(&headers, addr, false));
    }

    #[test]
    fn not_local_when_behind_proxy_env() {
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::HOST, "localhost:18789".parse().unwrap());
        // behind_proxy = true overrides everything.
        assert!(!is_local_connection(&headers, addr, true));
    }

    #[test]
    fn not_local_when_proxy_headers_present() {
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::HOST, "localhost:18789".parse().unwrap());
        headers.insert("x-forwarded-for", "203.0.113.50".parse().unwrap());
        assert!(!is_local_connection(&headers, addr, false));
    }

    #[test]
    fn not_local_when_xff_spoofs_loopback_value() {
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::HOST, "localhost:18789".parse().unwrap());
        // Header presence alone marks the request as proxied, even when value
        // is spoofed to look loopback.
        headers.insert("x-forwarded-for", "127.0.0.1".parse().unwrap());
        assert!(!is_local_connection(&headers, addr, false));
    }

    #[test]
    fn not_local_when_forwarded_spoofs_loopback_value() {
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::HOST, "localhost:18789".parse().unwrap());
        // RFC 7239 Forwarded header should never allow localhost bypass.
        headers.insert("forwarded", "for=127.0.0.1;proto=https".parse().unwrap());
        assert!(!is_local_connection(&headers, addr, false));
    }

    #[test]
    fn not_local_when_host_is_external() {
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::HOST,
            "moltis.example.com".parse().unwrap(),
        );
        assert!(!is_local_connection(&headers, addr, false));
    }

    #[test]
    fn not_local_when_remote_ip_not_loopback() {
        let addr: SocketAddr = "203.0.113.50:12345".parse().unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::HOST, "localhost:18789".parse().unwrap());
        assert!(!is_local_connection(&headers, addr, false));
    }

    /// Simulates a reverse proxy (Caddy/nginx) on the same machine that
    /// does NOT add proxy headers (bare nginx `proxy_pass`). The Host header
    /// is rewritten to the upstream (loopback) address and the TCP source is
    /// loopback. Without `MOLTIS_BEHIND_PROXY`, this would appear local.
    #[test]
    fn bare_proxy_without_env_var_appears_local() {
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::HOST, "127.0.0.1:18789".parse().unwrap());
        // This is the known limitation: bare proxy looks like local.
        assert!(is_local_connection(&headers, addr, false));
        // Setting MOLTIS_BEHIND_PROXY fixes it.
        assert!(!is_local_connection(&headers, addr, true));
    }

    /// Typical Caddy/nginx with proper headers: loopback TCP but
    /// X-Forwarded-For reveals the real client IP.
    #[test]
    fn proxy_with_xff_detected() {
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::HOST,
            "moltis.example.com".parse().unwrap(),
        );
        headers.insert("x-forwarded-for", "203.0.113.50".parse().unwrap());
        assert!(!is_local_connection(&headers, addr, false));
    }

    /// Proxy that rewrites Host to a public domain (but no XFF).
    #[test]
    fn proxy_detected_via_host_header() {
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::HOST,
            "moltis.example.com".parse().unwrap(),
        );
        assert!(!is_local_connection(&headers, addr, false));
    }

    #[cfg(feature = "web-ui")]
    #[test]
    fn askama_templates_emit_nonce_on_scripts() {
        let nonce = "test-nonce-abc";
        let index_template = IndexHtmlTemplate {
            build_ts: "dev",
            asset_prefix: "/assets/v/test/",
            nonce,
            gon_json: "{}",
            share_title: "moltis: AI assistant",
            share_description: "desc",
            share_site_name: "moltis",
            share_image_url: SHARE_IMAGE_URL,
            share_image_alt: "preview",
            routes: &SPA_ROUTES,
        };
        let index_html = match index_template.render() {
            Ok(html) => html,
            Err(e) => panic!("failed to render index template: {e}"),
        };
        assert!(index_html.contains(&format!("<script nonce=\"{nonce}\">!function()")));
        assert!(index_html.contains(&format!(
            "<script nonce=\"{nonce}\">window.__MOLTIS__={{}};</script>"
        )));
        assert!(index_html.contains(&format!("<script nonce=\"{nonce}\" type=\"importmap\">")));
        assert!(index_html.contains(&format!(
            "<script nonce=\"{nonce}\" type=\"module\" src=\"/assets/v/test/js/app.js\"></script>"
        )));

        let onboarding_template = OnboardingHtmlTemplate {
            build_ts: "dev",
            asset_prefix: "/assets/v/test/",
            nonce,
            page_title: "moltis onboarding",
            gon_json: "{}",
        };
        let onboarding_html = match onboarding_template.render() {
            Ok(html) => html,
            Err(e) => panic!("failed to render onboarding template: {e}"),
        };
        assert!(onboarding_html.contains(&format!(
            "<script nonce=\"{nonce}\">window.__MOLTIS__={{}};</script>"
        )));
        assert!(onboarding_html.contains(&format!(
            "<script nonce=\"{nonce}\" type=\"module\" src=\"/assets/v/test/js/onboarding-app.js\"></script>"
        )));
    }

    #[cfg(feature = "web-ui")]
    #[test]
    fn csp_header_contains_nonce() {
        let nonce = "abc-123";
        let csp = format!(
            "default-src 'self'; \
             script-src 'self' 'nonce-{nonce}'; \
             style-src 'self' 'unsafe-inline'; \
             img-src 'self' data: blob:; \
             media-src 'self' blob:; \
             font-src 'self'; \
             connect-src 'self' ws: wss:; \
             frame-ancestors 'none'; \
             form-action 'self'; \
             base-uri 'self'; \
             object-src 'none'"
        );

        assert!(csp.contains("'nonce-abc-123'"));
        assert!(csp.contains("frame-ancestors 'none'"));
        assert!(csp.contains("object-src 'none'"));
        assert!(csp.contains("connect-src 'self' ws: wss:"));
    }
}
