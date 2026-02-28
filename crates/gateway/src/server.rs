use std::{
    collections::{HashMap, HashSet},
    fs::OpenOptions,
    io::Write,
    net::SocketAddr,
    path::{Path as FsPath, PathBuf},
    sync::Arc,
};

use secrecy::ExposeSecret;

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
    tracing::{Level, debug, info, warn},
};

use {moltis_channels::ChannelPlugin, moltis_protocol::TICK_INTERVAL_MS};

use moltis_providers::ProviderRegistry;

use moltis_tools::{
    approval::{ApprovalManager, ApprovalMode, SecurityLevel},
    exec::EnvVarProvider,
};

use {
    moltis_projects::ProjectStore,
    moltis_sessions::{
        metadata::{SessionMetadata, SqliteSessionMetadata},
        session_events::{SessionEvent, SessionEventBus},
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
                "Please share your location in this chat.",
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

fn env_value_with_overrides(env_overrides: &HashMap<String, String>, key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            env_overrides
                .get(key)
                .cloned()
                .filter(|value| !value.trim().is_empty())
        })
}

fn merge_env_overrides(
    base_overrides: &HashMap<String, String>,
    additional: Vec<(String, String)>,
) -> HashMap<String, String> {
    let mut merged = base_overrides.clone();
    for (key, value) in additional {
        if key.trim().is_empty() || value.trim().is_empty() {
            continue;
        }
        merged.entry(key).or_insert(value);
    }
    merged
}

fn summarize_model_ids_for_logs(sorted_model_ids: &[String], max_items: usize) -> Vec<String> {
    if max_items == 0 {
        return Vec::new();
    }

    if sorted_model_ids.len() <= max_items || max_items < 3 {
        return sorted_model_ids.iter().take(max_items).cloned().collect();
    }

    let head_count = max_items / 2;
    let tail_count = max_items - head_count - 1;
    let mut sample = Vec::with_capacity(max_items);
    sample.extend(sorted_model_ids.iter().take(head_count).cloned());
    sample.push("...".to_string());
    sample.extend(
        sorted_model_ids
            .iter()
            .skip(sorted_model_ids.len().saturating_sub(tail_count))
            .cloned(),
    );
    sample
}

fn log_startup_model_inventory(reg: &ProviderRegistry) {
    const STARTUP_MODEL_SAMPLE_SIZE: usize = 8;
    const STARTUP_PROVIDER_MODEL_SAMPLE_SIZE: usize = 4;

    let mut by_provider: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    let mut model_ids: Vec<String> = Vec::with_capacity(reg.list_models().len());
    for model in reg.list_models() {
        model_ids.push(model.id.clone());
        by_provider
            .entry(model.provider.clone())
            .or_default()
            .push(model.id.clone());
    }
    model_ids.sort();

    let provider_model_counts: Vec<(String, usize)> = by_provider
        .iter()
        .map(|(provider, provider_models)| (provider.clone(), provider_models.len()))
        .collect();

    info!(
        model_count = model_ids.len(),
        provider_count = by_provider.len(),
        provider_model_counts = ?provider_model_counts,
        sample_model_ids = ?summarize_model_ids_for_logs(&model_ids, STARTUP_MODEL_SAMPLE_SIZE),
        "startup model inventory"
    );

    for (provider, provider_models) in &mut by_provider {
        provider_models.sort();
        debug!(
            provider = %provider,
            model_count = provider_models.len(),
            sample_model_ids = ?summarize_model_ids_for_logs(
                provider_models,
                STARTUP_PROVIDER_MODEL_SAMPLE_SIZE
            ),
            "startup provider model inventory"
        );
    }
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
    push_service: Option<Arc<crate::push::PushService>>,
    webauthn_registry: Option<Arc<crate::auth_webauthn::WebAuthnRegistry>>,
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
    webauthn_registry: Option<Arc<crate::auth_webauthn::WebAuthnRegistry>>,
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
    push_service: Option<Arc<crate::push::PushService>>,
    http_request_logs: bool,
    webauthn_registry: Option<Arc<crate::auth_webauthn::WebAuthnRegistry>>,
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
    webauthn_registry: Option<Arc<crate::auth_webauthn::WebAuthnRegistry>>,
) -> Router {
    let (router, app_state) = build_gateway_base(state, methods, webauthn_registry);
    finalize_gateway_app(router, app_state, http_request_logs)
}

fn env_var_or_unset(name: &str) -> String {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "<unset>".to_string())
}

fn env_flag_enabled(name: &str) -> bool {
    std::env::var(name)
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn validate_proxy_tls_configuration(
    behind_proxy: bool,
    tls_enabled: bool,
    allow_tls_behind_proxy: bool,
) -> anyhow::Result<()> {
    if behind_proxy && tls_enabled && !allow_tls_behind_proxy {
        anyhow::bail!(
            "MOLTIS_BEHIND_PROXY=true with Moltis TLS enabled is usually a proxy misconfiguration. Run with --no-tls (or MOLTIS_NO_TLS=true). If your proxy upstream is HTTPS/TCP passthrough by design, set MOLTIS_ALLOW_TLS_BEHIND_PROXY=true."
        );
    }
    Ok(())
}

fn log_path_diagnostics(kind: &str, path: &FsPath) {
    match std::fs::metadata(path) {
        Ok(metadata) => {
            info!(
                kind,
                path = %path.display(),
                exists = true,
                is_dir = metadata.is_dir(),
                readonly = metadata.permissions().readonly(),
                size_bytes = metadata.len(),
                "startup path diagnostics"
            );
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            info!(kind, path = %path.display(), exists = false, "startup path missing");
        },
        Err(error) => {
            warn!(
                kind,
                path = %path.display(),
                error = %error,
                "failed to inspect startup path"
            );
        },
    }
}

fn log_directory_write_probe(dir: &FsPath) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let probe_path = dir.join(format!(
        ".moltis-write-check-{}-{nanos}.tmp",
        std::process::id()
    ));

    match OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&probe_path)
    {
        Ok(mut file) => {
            if let Err(error) = file.write_all(b"probe") {
                warn!(
                    path = %probe_path.display(),
                    error = %error,
                    "startup write probe could not write to config directory"
                );
            } else {
                info!(
                    path = %probe_path.display(),
                    "startup write probe succeeded for config directory"
                );
            }
            if let Err(error) = std::fs::remove_file(&probe_path) {
                warn!(
                    path = %probe_path.display(),
                    error = %error,
                    "failed to clean up startup write probe file"
                );
            }
        },
        Err(error) => {
            warn!(
                path = %probe_path.display(),
                error = %error,
                "startup write probe failed for config directory"
            );
        },
    }
}

#[cfg(feature = "openclaw-import")]
fn log_startup_openclaw_detection() -> String {
    match moltis_openclaw_import::detect() {
        Some(detection) => {
            info!(
                openclaw_home = %detection.home_dir.display(),
                openclaw_workspace = %detection.workspace_dir.display(),
                has_config = detection.has_config,
                has_credentials = detection.has_credentials,
                has_memory = detection.has_memory,
                has_skills = detection.has_skills,
                has_mcp_servers = detection.has_mcp_servers,
                sessions = detection.session_count,
                agents = detection.agent_ids.len(),
                agent_ids = ?detection.agent_ids,
                unsupported_channels = ?detection.unsupported_channels,
                "startup OpenClaw installation detected"
            );
            format!("detected ({})", detection.home_dir.display())
        },
        None => {
            info!(
                openclaw_home_env = %env_var_or_unset("OPENCLAW_HOME"),
                openclaw_profile_env = %env_var_or_unset("OPENCLAW_PROFILE"),
                "startup OpenClaw installation not detected (checked OPENCLAW_HOME and ~/.openclaw)"
            );
            "not detected".to_string()
        },
    }
}

#[cfg(not(feature = "openclaw-import"))]
fn log_startup_openclaw_detection() -> String {
    info!("startup OpenClaw import feature disabled; detection skipped");
    "feature disabled".to_string()
}

#[cfg(feature = "openclaw-import")]
pub fn openclaw_detected_for_ui() -> bool {
    moltis_openclaw_import::detect().is_some()
}

#[cfg(not(feature = "openclaw-import"))]
pub fn openclaw_detected_for_ui() -> bool {
    false
}

fn log_startup_config_storage_diagnostics() {
    let config_dir = moltis_config::config_dir().unwrap_or_else(|| PathBuf::from(".moltis"));
    let discovered_config = moltis_config::loader::find_config_file();
    let expected_config = moltis_config::find_or_default_config_path();
    let provider_keys_path = config_dir.join("provider_keys.json");

    let discovered_display = discovered_config
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<none>".to_string());
    info!(
        user = %env_var_or_unset("USER"),
        home = %env_var_or_unset("HOME"),
        config_dir = %config_dir.display(),
        discovered_config = %discovered_display,
        expected_config = %expected_config.display(),
        provider_keys_path = %provider_keys_path.display(),
        "startup configuration storage diagnostics"
    );

    log_path_diagnostics("config-dir", &config_dir);
    log_directory_write_probe(&config_dir);

    if let Some(path) = discovered_config {
        log_path_diagnostics("config-file", &path);
    } else if expected_config.exists() {
        info!(
            path = %expected_config.display(),
            "default config file exists even though discovery did not report a named config"
        );
        log_path_diagnostics("config-file", &expected_config);
    } else {
        warn!(
            path = %expected_config.display(),
            "no config file detected on startup; Moltis is running with in-memory defaults until config is persisted"
        );
    }

    if provider_keys_path.exists() {
        log_path_diagnostics("provider-keys", &provider_keys_path);
        match std::fs::read_to_string(&provider_keys_path) {
            Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                Ok(_) => {
                    info!(
                        path = %provider_keys_path.display(),
                        bytes = content.len(),
                        "provider key store file is readable JSON"
                    );
                },
                Err(error) => {
                    warn!(
                        path = %provider_keys_path.display(),
                        error = %error,
                        "provider key store file contains invalid JSON"
                    );
                },
            },
            Err(error) => {
                warn!(
                    path = %provider_keys_path.display(),
                    error = %error,
                    "provider key store file exists but is not readable"
                );
            },
        }
    } else {
        info!(
            path = %provider_keys_path.display(),
            "provider key store file not found yet; it will be created after the first providers.save_key"
        );
    }
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
}

/// Internal metadata for the startup banner printed by [`start_gateway`].
pub(crate) struct BannerMeta {
    pub provider_summary: String,
    pub mcp_configured_count: usize,
    pub method_count: usize,
    pub sandbox_backend_name: String,
    pub data_dir: PathBuf,
    pub openclaw_status: String,
    pub setup_code_display: Option<String>,
    pub webauthn_registry: Option<Arc<crate::auth_webauthn::WebAuthnRegistry>>,
    pub browser_for_lifecycle: Arc<dyn crate::services::BrowserService>,
    pub config: moltis_config::schema::MoltisConfig,
    #[cfg(feature = "tailscale")]
    pub tailscale_mode: TailscaleMode,
    #[cfg(feature = "tailscale")]
    pub tailscale_reset_on_exit: bool,
}

/// Prepare the full gateway: load config, run migrations, wire services,
/// spawn background tasks, and return the composed axum application.
///
/// This is the core setup extracted from [`start_gateway`]. The swift-bridge
/// calls this directly and manages its own TCP listener + graceful shutdown.
///
/// `extra_routes` is an optional callback that returns additional routes
/// (e.g. the web-UI) to merge before finalization.
#[allow(clippy::expect_used)] // Startup fail-fast: DB, migrations, credential store must succeed.
pub async fn prepare_gateway(
    bind: &str,
    port: u16,
    no_tls: bool,
    log_buffer: Option<crate::logs::LogBuffer>,
    config_dir: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    #[cfg(feature = "tailscale")] tailscale_opts: Option<TailscaleOpts>,
    extra_routes: Option<RouteEnhancer>,
    session_event_bus: Option<SessionEventBus>,
) -> anyhow::Result<PreparedGateway> {
    let session_event_bus = session_event_bus.unwrap_or_default();

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
    let config_env_overrides = config.env.clone();
    let instance_slug_value = instance_slug(&config);
    let browser_container_prefix = browser_container_prefix(&instance_slug_value);
    let sandbox_container_prefix = sandbox_container_prefix(&instance_slug_value);

    // CLI --no-tls / MOLTIS_NO_TLS overrides config file TLS setting.
    if no_tls {
        config.tls.enabled = false;
    }
    let behind_proxy = env_flag_enabled("MOLTIS_BEHIND_PROXY");
    let allow_tls_behind_proxy = env_flag_enabled("MOLTIS_ALLOW_TLS_BEHIND_PROXY");
    #[cfg(feature = "tls")]
    let tls_enabled_for_gateway = config.tls.enabled;
    #[cfg(not(feature = "tls"))]
    let tls_enabled_for_gateway = false;
    validate_proxy_tls_configuration(
        behind_proxy,
        tls_enabled_for_gateway,
        allow_tls_behind_proxy,
    )?;
    if behind_proxy && tls_enabled_for_gateway && allow_tls_behind_proxy {
        warn!(
            "MOLTIS_ALLOW_TLS_BEHIND_PROXY=true is set; ensure your proxy uses HTTPS upstream or TLS passthrough to avoid redirect loops"
        );
    }

    let base_provider_config = config.providers.clone();

    // Merge any previously saved API keys into the provider config so they
    // survive gateway restarts without requiring env vars.
    let key_store = crate::provider_setup::KeyStore::new();
    // Collect local-llm model IDs (if the feature is enabled and models are configured).
    #[cfg(feature = "local-llm")]
    let local_model_ids: Vec<String> = crate::local_llm_setup::LocalLlmConfig::load()
        .map(|c| c.models.iter().map(|m| m.model_id.clone()).collect())
        .unwrap_or_default();
    #[cfg(not(feature = "local-llm"))]
    let local_model_ids: Vec<String> = Vec::new();

    let effective_providers = crate::provider_setup::config_with_saved_keys(
        &base_provider_config,
        &key_store,
        &local_model_ids,
    );

    let has_explicit_provider_settings =
        crate::provider_setup::has_explicit_provider_settings(&config.providers);
    let auto_detected_provider_sources = if has_explicit_provider_settings {
        Vec::new()
    } else {
        crate::provider_setup::detect_auto_provider_sources_with_overrides(
            &config.providers,
            deploy_platform.as_deref(),
            &config_env_overrides,
        )
    };

    // Discover LLM providers from env + config + saved keys.
    let registry = Arc::new(tokio::sync::RwLock::new(
        ProviderRegistry::from_env_with_config_and_overrides(
            &effective_providers,
            &config_env_overrides,
        ),
    ));
    let (provider_summary, providers_available_at_startup) = {
        let reg = registry.read().await;
        log_startup_model_inventory(&reg);
        (reg.provider_summary(), !reg.is_empty())
    };
    if !providers_available_at_startup {
        let config_path = moltis_config::find_or_default_config_path();
        let provider_keys_path = moltis_config::config_dir()
            .unwrap_or_else(|| PathBuf::from(".moltis"))
            .join("provider_keys.json");
        warn!(
            provider_summary = %provider_summary,
            config_path = %config_path.display(),
            provider_keys_path = %provider_keys_path.display(),
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
    )
    .with_env_overrides(config_env_overrides.clone())
    .with_error_parser(crate::chat_error::parse_chat_error);
    provider_setup.set_priority_models(live_model_service.priority_models_handle());
    let provider_setup_service = Arc::new(provider_setup);
    services.provider_setup =
        Arc::clone(&provider_setup_service) as Arc<dyn crate::services::ProviderSetupService>;

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
                let oauth = entry
                    .oauth
                    .as_ref()
                    .map(|o| moltis_mcp::registry::McpOAuthConfig {
                        client_id: o.client_id.clone(),
                        auth_url: o.auth_url.clone(),
                        token_url: o.token_url.clone(),
                        scopes: o.scopes.clone(),
                    });
                merged
                    .servers
                    .insert(name.clone(), moltis_mcp::McpServerConfig {
                        command: entry.command.clone(),
                        args: entry.args.clone(),
                        env: entry.env.clone(),
                        enabled: entry.enabled,
                        transport,
                        url: entry.url.clone(),
                        oauth,
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

    let config_dir = moltis_config::config_dir().unwrap_or_else(|| PathBuf::from(".moltis"));
    std::fs::create_dir_all(&config_dir).unwrap_or_else(|e| {
        panic!(
            "failed to create config directory {}: {e}",
            config_dir.display()
        )
    });
    log_startup_config_storage_diagnostics();

    let openclaw_startup_status = log_startup_openclaw_detection();

    // Enable log persistence so entries survive restarts.
    if let Some(ref buf) = log_buffer {
        buf.enable_persistence(data_dir.join("logs.jsonl"));
    }
    let db_path = data_dir.join("moltis.db");
    let db_pool = {
        use {
            sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
            std::str::FromStr,
        };
        let options = SqliteConnectOptions::from_str(&format!("sqlite:{}", db_path.display()))
            .expect("invalid database path")
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(std::time::Duration::from_secs(5));
        sqlx::SqlitePool::connect_with(options)
            .await
            .expect("failed to open moltis.db")
    };

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

    // Vault migrations (vault_metadata table).
    #[cfg(feature = "vault")]
    moltis_vault::run_migrations(&db_pool)
        .await
        .expect("failed to run vault migrations");

    // Migrate plugins data into unified skills system (idempotent, non-fatal).
    moltis_skills::migration::migrate_plugins_to_skills(&data_dir).await;

    // Initialize vault for encryption-at-rest.
    #[cfg(feature = "vault")]
    let vault: Option<Arc<moltis_vault::Vault>> = {
        match moltis_vault::Vault::new(db_pool.clone()).await {
            Ok(v) => {
                info!(status = ?v.status().await, "vault ready");
                Some(Arc::new(v))
            },
            Err(e) => {
                warn!(error = %e, "vault init failed, encryption disabled");
                None
            },
        }
    };

    // Initialize credential store (auth tables).
    #[cfg(feature = "vault")]
    let credential_store = Arc::new(
        auth::CredentialStore::with_vault(db_pool.clone(), &config.auth, vault.clone())
            .await
            .expect("failed to init credential store"),
    );
    #[cfg(not(feature = "vault"))]
    let credential_store = Arc::new(
        auth::CredentialStore::new(db_pool.clone())
            .await
            .expect("failed to init credential store"),
    );

    // Runtime env overrides from the settings UI (`/api/env`) layered after
    // config `[env]`. Process env remains highest precedence.
    let runtime_env_overrides = match credential_store.get_all_env_values().await {
        Ok(db_env_vars) => merge_env_overrides(&config_env_overrides, db_env_vars),
        Err(error) => {
            warn!(%error, "failed to load persisted env overrides from credential store");
            config_env_overrides.clone()
        },
    };

    // Initialize WebAuthn registry for passkey support.
    // Each hostname the user may access from gets its own RP ID + origins entry
    // so passkeys work from localhost, mDNS hostname, and .local alike.
    let default_scheme = if config.tls.enabled {
        "https"
    } else {
        "http"
    };

    // Explicit RP ID from env (PaaS platforms).
    let explicit_rp_id = std::env::var("MOLTIS_WEBAUTHN_RP_ID")
        .or_else(|_| std::env::var("APP_DOMAIN"))
        .or_else(|_| std::env::var("RENDER_EXTERNAL_HOSTNAME"))
        .or_else(|_| std::env::var("FLY_APP_NAME").map(|name| format!("{name}.fly.dev")))
        .or_else(|_| std::env::var("RAILWAY_PUBLIC_DOMAIN"))
        .ok();

    let explicit_origin = std::env::var("MOLTIS_WEBAUTHN_ORIGIN")
        .or_else(|_| std::env::var("APP_URL"))
        .or_else(|_| std::env::var("RENDER_EXTERNAL_URL"))
        .ok();

    let webauthn_registry = {
        let mut registry = crate::auth_webauthn::WebAuthnRegistry::new();
        let mut any_ok = false;

        // Helper: try to add one RP ID with its origin + extras to the registry.
        let mut try_add = |rp_id: &str, origin_str: &str, extras: &[webauthn_rs::prelude::Url]| {
            let Ok(origin_url) = webauthn_rs::prelude::Url::parse(origin_str) else {
                tracing::warn!("invalid WebAuthn origin URL '{origin_str}'");
                return;
            };
            match crate::auth_webauthn::WebAuthnState::new(rp_id, &origin_url, extras) {
                Ok(wa) => {
                    info!(rp_id = %rp_id, origins = ?wa.get_allowed_origins(), "WebAuthn RP registered");
                    registry.add(rp_id.to_owned(), wa);
                    any_ok = true;
                },
                Err(e) => tracing::warn!(rp_id = %rp_id, "failed to init WebAuthn: {e}"),
            }
        };

        if let Some(ref rp_id) = explicit_rp_id {
            // PaaS: single explicit RP ID.
            let origin = explicit_origin
                .clone()
                .unwrap_or_else(|| format!("https://{rp_id}"));
            try_add(rp_id, &origin, &[]);
        } else {
            // Local: register localhost + moltis.localhost as extras.
            let localhost_origin = format!("{default_scheme}://localhost:{port}");
            let moltis_localhost: Vec<webauthn_rs::prelude::Url> =
                webauthn_rs::prelude::Url::parse(&format!(
                    "{default_scheme}://moltis.localhost:{port}"
                ))
                .into_iter()
                .collect();
            try_add("localhost", &localhost_origin, &moltis_localhost);

            // Register system hostname and hostname.local for LAN/mDNS access.
            if let Ok(hn) = hostname::get() {
                let hn_str = hn.to_string_lossy();
                if hn_str != "localhost" {
                    // hostname.local as RP ID (mDNS access)
                    let local_name = if hn_str.ends_with(".local") {
                        hn_str.to_string()
                    } else {
                        format!("{hn_str}.local")
                    };
                    let local_origin = format!("{default_scheme}://{local_name}:{port}");
                    try_add(&local_name, &local_origin, &[]);

                    // bare hostname as RP ID (direct LAN access)
                    let bare = hn_str.strip_suffix(".local").unwrap_or(&hn_str);
                    if bare != local_name {
                        let bare_origin = format!("{default_scheme}://{bare}:{port}");
                        try_add(bare, &bare_origin, &[]);
                    }
                }
            }
        }

        if any_ok {
            info!(origins = ?registry.get_all_origins(), "WebAuthn passkeys enabled");
            Some(Arc::new(registry))
        } else {
            None
        }
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
    let config_dir = moltis_config::config_dir().unwrap_or_else(|| PathBuf::from(".moltis"));
    let projects_toml_path = config_dir.join("projects.toml");
    if projects_toml_path.exists() {
        info!("migrating projects.toml to SQLite");
        let old_store = moltis_projects::TomlProjectStore::new(projects_toml_path.clone());
        let sqlite_store = moltis_projects::SqliteProjectStore::new(db_pool.clone());
        if let Ok(projects) =
            <moltis_projects::TomlProjectStore as ProjectStore>::list(&old_store).await
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
    let project_store: Arc<dyn ProjectStore> =
        Arc::new(moltis_projects::SqliteProjectStore::new(db_pool.clone()));
    let session_store = Arc::new(SessionStore::new(sessions_dir));
    let event_bus_for_metadata = session_event_bus.clone();
    let session_metadata = Arc::new(SqliteSessionMetadata::with_event_bus(
        db_pool.clone(),
        event_bus_for_metadata,
    ));
    let session_share_store = Arc::new(crate::share_store::ShareStore::new(db_pool.clone()));
    let session_state_store = Arc::new(moltis_sessions::state_store::SessionStateStore::new(
        db_pool.clone(),
    ));

    // Wire agent persona store for multi-agent support (created early so onboarding can use it).
    let agent_persona_store = Arc::new(crate::agent_persona::AgentPersonaStore::new(
        db_pool.clone(),
    ));
    if let Err(e) = agent_persona_store.ensure_main_workspace_seeded() {
        tracing::warn!(error = %e, "failed to seed main agent workspace");
    }

    services =
        services.with_onboarding(Arc::new(crate::onboarding::GatewayOnboardingService::new(
            live_onboarding,
            Arc::clone(&session_metadata),
            Arc::clone(&agent_persona_store),
        )));

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

    // Create the system events queue before the callbacks so it can be shared.
    let events_queue = moltis_cron::system_events::SystemEventsQueue::new();

    // Agent turn: run an LLM turn in a session determined by the job's session_target.
    let agent_state = Arc::clone(&deferred_state);
    let agent_events_queue = Arc::clone(&events_queue);
    let on_agent_turn: moltis_cron::service::AgentTurnFn = Arc::new(move |req| {
        let st = Arc::clone(&agent_state);
        let eq = Arc::clone(&agent_events_queue);
        Box::pin(async move {
            let state = st
                .get()
                .ok_or_else(|| moltis_cron::Error::message("gateway not ready"))?;

            // OpenClaw-style cost guard: if HEARTBEAT.md exists but is effectively
            // empty (comments/blank scaffold) and there's no explicit
            // heartbeat.prompt override, skip the LLM turn entirely.
            let is_heartbeat_turn = matches!(
                &req.session_target,
                moltis_cron::types::SessionTarget::Named(name) if name == "heartbeat"
            );
            // Check for pending system events (used to bypass the empty-content guard).
            let has_pending_events = is_heartbeat_turn && !eq.is_empty().await;
            if is_heartbeat_turn && !has_pending_events {
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
                } else {
                    router.remove_image_override(&session_key).await;
                }
            }

            let prompt_text = if is_heartbeat_turn {
                let events = eq.drain().await;
                if events.is_empty() {
                    req.message.clone()
                } else {
                    tracing::info!(
                        event_count = events.len(),
                        "enriching heartbeat prompt with system events"
                    );
                    moltis_cron::heartbeat::build_event_enriched_prompt(&events, &req.message)
                }
            } else {
                req.message.clone()
            };

            // When the output will be delivered to a channel, prepend a
            // formatting hint so the LLM produces channel-friendly content.
            let prompt_text = if req.deliver && !is_heartbeat_turn {
                format!(
                    "Your response will be delivered to an external chat channel. \
                     Keep it concise and prefer plain text with minimal formatting.\n\n\
                     {prompt_text}"
                )
            } else {
                prompt_text
            };

            let mut params = serde_json::json!({
                "text": prompt_text,
                "_session_key": session_key,
            });
            if let Some(ref model) = req.model {
                params["model"] = serde_json::Value::String(model.clone());
            }
            let result = chat
                .send_sync(params)
                .await
                .map_err(|e| moltis_cron::Error::message(e.to_string()));

            // Clean up sandbox overrides.
            if let Some(ref router) = state.sandbox_router {
                router.remove_override(&session_key).await;
                router.remove_image_override(&session_key).await;
            }

            let val = result?;
            let input_tokens = val.get("inputTokens").and_then(|v| v.as_u64());
            let output_tokens = val.get("outputTokens").and_then(|v| v.as_u64());
            let text = val
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // Deliver output to a channel if requested.
            if req.deliver
                && !is_heartbeat_turn
                && !text.is_empty()
                && let (Some(channel_account), Some(chat_id)) = (&req.channel, &req.to)
            {
                if let Some(outbound) = state.services.channel_outbound_arc() {
                    if let Err(e) = outbound
                        .send_text(channel_account, chat_id, &text, None)
                        .await
                    {
                        tracing::warn!(
                            channel = %channel_account,
                            to = %chat_id,
                            error = %e,
                            "cron job channel delivery failed"
                        );
                    }
                } else {
                    tracing::debug!(
                        "cron job delivery requested but no channel outbound configured"
                    );
                }
            }

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
                broadcast(&state, event, payload, BroadcastOpts {
                    drop_if_slow: true,
                    ..Default::default()
                })
                .await;
            });
        });

    // Build rate limit config from moltis config.
    let rate_limit_config = moltis_cron::service::RateLimitConfig {
        max_per_window: config.cron.rate_limit_max,
        window_ms: config.cron.rate_limit_window_secs * 1000,
    };

    let cron_service = moltis_cron::service::CronService::with_events_queue(
        cron_store,
        on_system_event,
        on_agent_turn,
        Some(on_cron_notify),
        rate_limit_config,
        events_queue,
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
            // Mark the build as in-progress so the UI can show a banner
            // even if the WebSocket broadcast fires before the client connects.
            sandbox_router
                .building_flag
                .store(true, std::sync::atomic::Ordering::Relaxed);
            let build_router = Arc::clone(&sandbox_router);
            tokio::spawn(async move {
                // Broadcast build start event.
                if let Some(state) = deferred_for_build.get() {
                    broadcast(
                        state,
                        "sandbox.image.build",
                        serde_json::json!({ "phase": "start", "packages": packages }),
                        BroadcastOpts {
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
                        build_router
                            .building_flag
                            .store(false, std::sync::atomic::Ordering::Relaxed);

                        if let Some(state) = deferred_for_build.get() {
                            broadcast(
                                state,
                                "sandbox.image.build",
                                serde_json::json!({
                                    "phase": "done",
                                    "tag": result.tag,
                                    "built": result.built,
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
                        debug!(
                            "sandbox image pre-build: no-op (no packages or unsupported backend)"
                        );
                        build_router
                            .building_flag
                            .store(false, std::sync::atomic::Ordering::Relaxed);
                    },
                    Err(e) => {
                        tracing::warn!("sandbox image pre-build failed: {e}");
                        build_router
                            .building_flag
                            .store(false, std::sync::atomic::Ordering::Relaxed);
                        if let Some(state) = deferred_for_build.get() {
                            broadcast(
                                state,
                                "sandbox.image.build",
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

    // Startup GC: remove orphaned session containers from previous runs.
    // At startup no legitimate sessions exist, so any prefixed containers are stale.
    if sandbox_router.backend_name() != "none" {
        let prefix = sandbox_router.config().container_prefix.clone();
        tokio::spawn(async move {
            if let Some(prefix) = prefix {
                match moltis_tools::sandbox::clean_all_containers(&prefix).await {
                    Ok(0) => {},
                    Ok(n) => info!(
                        removed = n,
                        "startup GC: cleaned orphaned session containers"
                    ),
                    Err(e) => debug!("startup GC: container cleanup skipped: {e}"),
                }
            }
        });
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
                broadcast(
                    state,
                    "browser.image.pull",
                    serde_json::json!({
                        "phase": "start",
                        "image": sandbox_image,
                    }),
                    BroadcastOpts {
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
                        broadcast(
                            state,
                            "browser.image.pull",
                            serde_json::json!({
                                "phase": "done",
                                "image": sandbox_image,
                            }),
                            BroadcastOpts {
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
                        broadcast(
                            state,
                            "browser.image.pull",
                            serde_json::json!({
                                "phase": "error",
                                "image": sandbox_image,
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

    let msteams_webhook_plugin: Arc<tokio::sync::RwLock<moltis_msteams::MsTeamsPlugin>>;

    // Wire channel store and channel plugins.
    {
        use moltis_channels::store::ChannelStore;

        let channel_store: Arc<dyn ChannelStore> = Arc::new(
            crate::channel_store::SqliteChannelStore::new(db_pool.clone()),
        );

        let channel_sink: Arc<dyn moltis_channels::ChannelEventSink> = Arc::new(
            crate::channel_events::GatewayChannelEventSink::new(Arc::clone(&deferred_state)),
        );
        let tg_plugin = Arc::new(tokio::sync::RwLock::new(
            moltis_telegram::TelegramPlugin::new()
                .with_message_log(Arc::clone(&message_log))
                .with_event_sink(Arc::clone(&channel_sink)),
        ));
        let msteams_plugin = Arc::new(tokio::sync::RwLock::new(
            moltis_msteams::MsTeamsPlugin::new()
                .with_message_log(Arc::clone(&message_log))
                .with_event_sink(Arc::clone(&channel_sink)),
        ));
        msteams_webhook_plugin = Arc::clone(&msteams_plugin);
        let discord_plugin = Arc::new(tokio::sync::RwLock::new(
            moltis_discord::DiscordPlugin::new()
                .with_message_log(Arc::clone(&message_log))
                .with_event_sink(Arc::clone(&channel_sink)),
        ));

        #[cfg(feature = "whatsapp")]
        let whatsapp_plugin = {
            let wa_data_dir = data_dir.join("whatsapp");
            if let Err(e) = std::fs::create_dir_all(&wa_data_dir) {
                tracing::warn!("failed to create whatsapp data dir: {e}");
            }
            Arc::new(tokio::sync::RwLock::new(
                moltis_whatsapp::WhatsAppPlugin::new(wa_data_dir)
                    .with_message_log(Arc::clone(&message_log))
                    .with_event_sink(channel_sink),
            ))
        };
        #[cfg(not(feature = "whatsapp"))]
        let _ = channel_sink; // consume unused channel_sink

        // Start channels from config file (these take precedence over DB rows).
        let mut started: HashSet<(String, String)> = HashSet::new();

        {
            let mut tg = tg_plugin.write().await;
            for (account_id, account_config) in &config.channels.telegram {
                if let Err(e) = tg.start_account(account_id, account_config.clone()).await {
                    tracing::warn!(account_id, "failed to start telegram account: {e}");
                } else {
                    started.insert(("telegram".into(), account_id.clone()));
                }
            }
        }

        {
            let mut ms = msteams_plugin.write().await;
            for (account_id, account_config) in &config.channels.msteams {
                if let Err(e) = ms.start_account(account_id, account_config.clone()).await {
                    tracing::warn!(account_id, "failed to start microsoft teams account: {e}");
                } else {
                    started.insert(("msteams".into(), account_id.clone()));
                }
            }
        }

        {
            let mut dc = discord_plugin.write().await;
            for (account_id, account_config) in &config.channels.discord {
                if let Err(e) = dc.start_account(account_id, account_config.clone()).await {
                    tracing::warn!(account_id, "failed to start discord account: {e}");
                } else {
                    started.insert(("discord".into(), account_id.clone()));
                }
            }
        }

        #[cfg(feature = "whatsapp")]
        {
            let mut wa = whatsapp_plugin.write().await;
            for (account_id, account_config) in &config.channels.whatsapp {
                if let Err(e) = wa.start_account(account_id, account_config.clone()).await {
                    tracing::warn!(account_id, "failed to start whatsapp account: {e}");
                } else {
                    started.insert(("whatsapp".into(), account_id.clone()));
                }
            }
        }

        // Load persisted channels that were not started from config.
        match channel_store.list().await {
            Ok(stored) => {
                info!("{} stored channel(s) found in database", stored.len());
                for ch in stored {
                    let key = (ch.channel_type.clone(), ch.account_id.clone());
                    if started.contains(&key) {
                        info!(
                            account_id = ch.account_id,
                            channel_type = ch.channel_type,
                            "skipping stored channel (already started from config)"
                        );
                        continue;
                    }

                    info!(
                        account_id = ch.account_id,
                        channel_type = ch.channel_type,
                        "starting stored channel"
                    );
                    let start_result = match ch.channel_type.parse::<moltis_channels::ChannelType>()
                    {
                        Ok(moltis_channels::ChannelType::Telegram) => {
                            let mut tg = tg_plugin.write().await;
                            tg.start_account(&ch.account_id, ch.config).await
                        },
                        Ok(moltis_channels::ChannelType::MsTeams) => {
                            let mut ms = msteams_plugin.write().await;
                            ms.start_account(&ch.account_id, ch.config).await
                        },
                        Ok(moltis_channels::ChannelType::Discord) => {
                            let mut dc = discord_plugin.write().await;
                            dc.start_account(&ch.account_id, ch.config).await
                        },
                        #[cfg(feature = "whatsapp")]
                        Ok(moltis_channels::ChannelType::Whatsapp) => {
                            let mut wa = whatsapp_plugin.write().await;
                            wa.start_account(&ch.account_id, ch.config).await
                        },
                        #[cfg(not(feature = "whatsapp"))]
                        Ok(moltis_channels::ChannelType::Whatsapp) => {
                            tracing::warn!(
                                account_id = ch.account_id,
                                "whatsapp feature not enabled, skipping stored account"
                            );
                            continue;
                        },
                        Err(e) => Err(moltis_channels::Error::invalid_input(e)),
                    };

                    if let Err(e) = start_result {
                        tracing::warn!(
                            account_id = ch.account_id,
                            channel_type = ch.channel_type,
                            "failed to start stored channel account: {e}"
                        );
                    } else {
                        started.insert(key);
                    }
                }
            },
            Err(e) => tracing::warn!("failed to load stored channels: {e}"),
        }

        if !started.is_empty() {
            info!("{} channel account(s) started", started.len());
        }

        let (tg_outbound, tg_stream_outbound) = {
            let tg = tg_plugin.read().await;
            (tg.shared_outbound(), tg.shared_stream_outbound())
        };
        let (ms_outbound, ms_stream_outbound) = {
            let ms = msteams_plugin.read().await;
            (ms.shared_outbound(), ms.shared_stream_outbound())
        };
        let (dc_outbound, dc_stream_outbound) = {
            let dc = discord_plugin.read().await;
            (dc.shared_outbound(), dc.shared_stream_outbound())
        };
        #[cfg(feature = "whatsapp")]
        let (wa_outbound, wa_stream_outbound) = {
            let wa = whatsapp_plugin.read().await;
            (wa.shared_outbound(), wa.shared_stream_outbound())
        };

        let multi_router = Arc::new(crate::channel_outbound::MultiChannelOutbound::new(
            Arc::clone(&tg_plugin),
            Arc::clone(&msteams_plugin),
            Arc::clone(&discord_plugin),
            #[cfg(feature = "whatsapp")]
            Arc::clone(&whatsapp_plugin),
            tg_outbound,
            ms_outbound,
            dc_outbound,
            #[cfg(feature = "whatsapp")]
            wa_outbound,
            tg_stream_outbound,
            ms_stream_outbound,
            dc_stream_outbound,
            #[cfg(feature = "whatsapp")]
            wa_stream_outbound,
        ));

        services = services.with_channel_outbound(
            Arc::clone(&multi_router) as Arc<dyn moltis_channels::ChannelOutbound>
        );
        services = services.with_channel_stream_outbound(
            multi_router as Arc<dyn moltis_channels::ChannelStreamOutbound>,
        );

        services.channel = Arc::new(crate::channel::LiveChannelService::new(
            tg_plugin,
            msteams_plugin,
            discord_plugin,
            #[cfg(feature = "whatsapp")]
            whatsapp_plugin,
            channel_store,
            Arc::clone(&message_log),
            Arc::clone(&session_metadata),
        ));
    }

    services = services.with_session_metadata(Arc::clone(&session_metadata));
    services = services.with_session_store(Arc::clone(&session_store));
    services = services.with_session_share_store(Arc::clone(&session_share_store));

    services = services.with_agent_persona_store(Arc::clone(&agent_persona_store));

    // ── Hook discovery & registration ─────────────────────────────────────
    seed_default_workspace_markdown_files();
    seed_example_skill();
    seed_example_hook();
    seed_dcg_guard_hook();
    let persisted_disabled = crate::methods::load_disabled_hooks();
    let (hook_registry, discovered_hooks_info) =
        discover_and_build_hooks(&persisted_disabled, Some(&session_store)).await;

    // Wire live session service with sandbox router, project store, hooks, and browser.
    {
        let mut session_svc =
            LiveSessionService::new(Arc::clone(&session_store), Arc::clone(&session_metadata))
                .with_tts_service(Arc::clone(&services.tts))
                .with_share_store(Arc::clone(&session_share_store))
                .with_sandbox_router(Arc::clone(&sandbox_router))
                .with_agent_persona_store(Arc::clone(&agent_persona_store))
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

        if mem_cfg.disable_rag {
            info!("memory: RAG disabled via memory.disable_rag=true, using keyword-only search");
        } else {
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
                                .map(PathBuf::from)
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
                        let base_url = mem_cfg.base_url.clone().unwrap_or_else(|| {
                            match provider_name.as_str() {
                                "ollama" => "http://localhost:11434".into(),
                                _ => "https://api.openai.com".into(),
                            }
                        });
                        if provider_name == "ollama" {
                            let model = mem_cfg.model.as_deref().unwrap_or("nomic-embed-text");
                            ensure_ollama_model(&base_url, model).await;
                        }
                        let api_key = mem_cfg
                            .api_key
                            .as_ref()
                            .map(|k| k.expose_secret().clone())
                            .or_else(|| {
                                env_value_with_overrides(&runtime_env_overrides, "OPENAI_API_KEY")
                            })
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
                    let e = moltis_memory::embeddings_openai::OpenAiEmbeddingProvider::new(
                        String::new(),
                    )
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
                ("minimax", "MINIMAX_API_KEY", "https://api.minimax.io/v1"),
                ("moonshot", "MOONSHOT_API_KEY", "https://api.moonshot.ai/v1"),
                ("venice", "VENICE_API_KEY", "https://api.venice.ai/api/v1"),
            ];

            for (config_name, env_key, default_base) in EMBEDDING_CANDIDATES {
                let key = effective_providers
                    .get(config_name)
                    .and_then(|e| e.api_key.as_ref().map(|k| k.expose_secret().clone()))
                    .or_else(|| env_value_with_overrides(&runtime_env_overrides, env_key))
                    .filter(|k| !k.is_empty());
                if let Some(api_key) = key {
                    let base = effective_providers
                        .get(config_name)
                        .and_then(|e| e.base_url.clone())
                        .unwrap_or_else(|| default_base.to_string());
                    let mut e =
                        moltis_memory::embeddings_openai::OpenAiEmbeddingProvider::new(api_key);
                    if base != "https://api.openai.com" {
                        e = e.with_base_url(base);
                    }
                    embedding_providers.push((config_name.to_string(), Box::new(e)));
                }
            }
        }

        // Build the final embedder: fallback chain, single provider, or keyword-only.
        let embedder: Option<Box<dyn moltis_memory::embeddings::EmbeddingProvider>> = if mem_cfg
            .disable_rag
        {
            None
        } else if embedding_providers.is_empty() {
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
        let memory_pool_result = {
            use {
                sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
                std::str::FromStr,
            };
            let options =
                SqliteConnectOptions::from_str(&format!("sqlite:{}", memory_db_path.display()))
                    .expect("invalid memory database path")
                    .create_if_missing(true)
                    .journal_mode(SqliteJournalMode::Wal)
                    .synchronous(SqliteSynchronous::Normal)
                    .busy_timeout(std::time::Duration::from_secs(5));
            sqlx::SqlitePool::connect_with(options).await
        };
        match memory_pool_result {
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
                    let agents_root = data_dir.join("agents");

                    let config = moltis_memory::config::MemoryConfig {
                        db_path: memory_db_path.to_string_lossy().into(),
                        data_dir: Some(data_dir.clone()),
                        memory_dirs: vec![
                            data_memory_file,
                            data_memory_file_lower,
                            data_memory_sub,
                            // Include all agent workspaces so per-agent memory writes
                            // remain indexed across periodic full syncs.
                            agents_root,
                        ],
                        ..Default::default()
                    };

                    let store = Box::new(moltis_memory::store_sqlite::SqliteMemoryStore::new(
                        memory_pool,
                    ));
                    // Map file entries to their parent directory so that
                    // root-level files like MEMORY.md are covered by the
                    // watcher. Deduplicate via BTreeSet to avoid watching
                    // the same directory twice.
                    let watch_dirs: Vec<_> = config
                        .memory_dirs
                        .iter()
                        .map(|p| {
                            if p.is_dir() {
                                p.clone()
                            } else {
                                p.parent().unwrap_or(p.as_path()).to_path_buf()
                            }
                        })
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
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

    // Keep a reference to the browser service for periodic cleanup and shutdown.
    let browser_for_lifecycle = Arc::clone(&services.browser);

    let state = GatewayState::with_options(
        resolved_auth,
        services,
        Some(Arc::clone(&sandbox_router)),
        Some(Arc::clone(&credential_store)),
        is_localhost,
        behind_proxy,
        tls_enabled_for_gateway,
        hook_registry.clone(),
        memory_manager.clone(),
        port,
        config.server.ws_request_logs,
        deploy_platform.clone(),
        Some(session_event_bus),
        #[cfg(feature = "metrics")]
        metrics_handle,
        #[cfg(feature = "metrics")]
        metrics_store.clone(),
        #[cfg(feature = "vault")]
        vault.clone(),
    );

    // Store discovered hook info and disabled set in state for the web UI.
    {
        let mut inner = state.inner.write().await;
        inner.discovered_hooks = discovered_hooks_info;
        inner.disabled_hooks = persisted_disabled;
        #[cfg(feature = "metrics")]
        {
            inner.metrics_history =
                crate::state::MetricsHistory::new(config.metrics.history_points);
        }
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

    // Set the broadcaster on provider setup service for validation progress updates.
    provider_setup_service.set_broadcaster(Arc::new(crate::provider_setup::GatewayBroadcaster {
        state: Arc::clone(&state),
    }));

    // Set the state on model service for broadcasting model update events.
    live_model_service.set_state(crate::chat::GatewayChatRuntime::from_state(Arc::clone(
        &state,
    )));

    // Model support probing is triggered on-demand by the web UI when the
    // user opens the model selector (via the `models.detect_supported` RPC).
    // With dynamic model discovery, automatic probing at startup is too
    // expensive and noisy — non-chat models (image, audio, video) would
    // generate spurious warnings.

    // Store heartbeat config and channels offered on state for gon data and RPC methods.
    {
        let mut inner = state.inner.write().await;
        inner.heartbeat_config = config.heartbeat.clone();
        inner.channels_offered = config.channels.offered.clone();
    }
    #[cfg(feature = "graphql")]
    state.set_graphql_enabled(config.graphql.enabled);

    // Wire live chat service (needs state reference, so done after state creation).
    {
        let broadcaster = Arc::new(GatewayApprovalBroadcaster::new(Arc::clone(&state)));
        let env_provider: Arc<dyn EnvVarProvider> = credential_store.clone();
        let eq = cron_service.events_queue().clone();
        let cs = Arc::clone(&cron_service);
        let exec_cb: moltis_tools::exec::ExecCompletionFn = Arc::new(move |event| {
            let summary = format!("Command `{}` exited {}", event.command, event.exit_code);
            let eq = Arc::clone(&eq);
            let cs = Arc::clone(&cs);
            tokio::spawn(async move {
                eq.enqueue(summary, "exec-event".into()).await;
                cs.wake("exec-event").await;
            });
        });
        let exec_tool = moltis_tools::exec::ExecTool::default()
            .with_approval(Arc::clone(&approval_manager), broadcaster)
            .with_sandbox_router(Arc::clone(&sandbox_router))
            .with_env_provider(Arc::clone(&env_provider))
            .with_completion_callback(exec_cb);

        let cron_tool = moltis_tools::cron_tool::CronTool::new(Arc::clone(&cron_service));

        let mut tool_registry = moltis_agents::tool_registry::ToolRegistry::new();
        let process_tool = moltis_tools::process::ProcessTool::new()
            .with_sandbox_router(Arc::clone(&sandbox_router));

        let sandbox_packages_tool = moltis_tools::sandbox_packages::SandboxPackagesTool::new()
            .with_sandbox_router(Arc::clone(&sandbox_router));

        tool_registry.register(Box::new(exec_tool));
        tool_registry.register(Box::new(moltis_tools::calc::CalcTool::new()));
        #[cfg(feature = "wasm")]
        {
            let wasm_limits = sandbox_router
                .config()
                .wasm_tool_limits
                .clone()
                .unwrap_or_default();
            let epoch_interval_ms = sandbox_router
                .config()
                .wasm_epoch_interval_ms
                .unwrap_or(100);
            let brave_api_key = config
                .tools
                .web
                .search
                .api_key
                .as_ref()
                .map(|s| s.expose_secret().clone())
                .or_else(|| env_value_with_overrides(&runtime_env_overrides, "BRAVE_API_KEY"))
                .filter(|k| !k.trim().is_empty());
            if let Err(e) = moltis_tools::wasm_tool_runner::register_wasm_tools(
                &mut tool_registry,
                &wasm_limits,
                epoch_interval_ms,
                config.tools.web.fetch.timeout_seconds,
                config.tools.web.fetch.cache_ttl_minutes,
                config.tools.web.search.timeout_seconds,
                config.tools.web.search.cache_ttl_minutes,
                brave_api_key.as_deref(),
            ) {
                warn!(%e, "wasm tool registration failed");
            }
        }
        tool_registry.register(Box::new(process_tool));
        tool_registry.register(Box::new(sandbox_packages_tool));
        tool_registry.register(Box::new(cron_tool));
        tool_registry.register(Box::new(
            moltis_tools::send_image::SendImageTool::new()
                .with_sandbox_router(Arc::clone(&sandbox_router)),
        ));
        if let Some(t) = moltis_tools::web_search::WebSearchTool::from_config_with_env_overrides(
            &config.tools.web.search,
            &runtime_env_overrides,
        ) {
            tool_registry.register(Box::new(t.with_env_provider(Arc::clone(&env_provider))));
        }
        if let Some(t) = moltis_tools::web_fetch::WebFetchTool::from_config(&config.tools.web.fetch)
        {
            tool_registry.register(Box::new(t));
        }
        if let Some(t) = moltis_tools::browser::BrowserTool::from_config(&config.tools.browser) {
            let t = if sandbox_router.backend_name() != "none" {
                t.with_sandbox_router(Arc::clone(&sandbox_router))
            } else {
                t
            };
            tool_registry.register(Box::new(t));
        }

        #[cfg(feature = "caldav")]
        {
            if let Some(t) = moltis_caldav::tool::CalDavTool::from_config(&config.caldav) {
                tool_registry.register(Box::new(t));
            }
        }

        // Register memory tools if the memory system is available.
        if let Some(ref mm) = memory_manager {
            tool_registry.register(Box::new(moltis_memory::tools::MemorySearchTool::new(
                Arc::clone(mm),
            )));
            tool_registry.register(Box::new(moltis_memory::tools::MemoryGetTool::new(
                Arc::clone(mm),
            )));
            tool_registry.register(Box::new(moltis_memory::tools::MemorySaveTool::new(
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
        let map_provider = match config.tools.maps.provider {
            moltis_config::schema::MapProvider::GoogleMaps => {
                moltis_tools::map::MapProvider::GoogleMaps
            },
            moltis_config::schema::MapProvider::AppleMaps => {
                moltis_tools::map::MapProvider::AppleMaps
            },
            moltis_config::schema::MapProvider::OpenStreetMap => {
                moltis_tools::map::MapProvider::OpenStreetMap
            },
        };
        tool_registry.register(Box::new(moltis_tools::map::ShowMapTool::with_provider(
            map_provider,
        )));

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
            crate::chat::GatewayChatRuntime::from_state(Arc::clone(&state)),
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
        let watch_dirs: Vec<PathBuf> = search_paths.into_iter().map(|(p, _)| p).collect();
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

    // Spawn OpenClaw session watcher for automatic background syncing.
    #[cfg(all(feature = "file-watcher", feature = "openclaw-import"))]
    {
        if let Some(detection) = moltis_openclaw_import::detect() {
            let import_agent = if detection.agent_ids.contains(&"main".to_string()) {
                "main"
            } else {
                detection
                    .agent_ids
                    .first()
                    .map(|s| s.as_str())
                    .unwrap_or("main")
            };
            let sessions_dir = detection
                .home_dir
                .join("agents")
                .join(import_agent)
                .join("agent")
                .join("sessions");
            if sessions_dir.is_dir() {
                match moltis_openclaw_import::watcher::ImportWatcher::start(sessions_dir) {
                    Ok((_watcher, mut rx)) => {
                        info!("openclaw: session watcher started");
                        let watcher_data_dir = data_dir.clone();
                        tokio::spawn(async move {
                            let _watcher = _watcher; // keep alive
                            let mut interval =
                                tokio::time::interval(std::time::Duration::from_secs(60));
                            interval.tick().await; // skip first immediate tick
                            loop {
                                tokio::select! {
                                    Some(_event) = rx.recv() => {
                                        debug!("openclaw: session change detected, running incremental import");
                                        let report = moltis_openclaw_import::import_sessions_only(
                                            &detection, &watcher_data_dir,
                                        );
                                        if report.items_imported > 0 || report.items_updated > 0 {
                                            info!(
                                                imported = report.items_imported,
                                                updated = report.items_updated,
                                                skipped = report.items_skipped,
                                                "openclaw: incremental session sync complete"
                                            );
                                        }
                                    }
                                    _ = interval.tick() => {
                                        debug!("openclaw: periodic session sync");
                                        let report = moltis_openclaw_import::import_sessions_only(
                                            &detection, &watcher_data_dir,
                                        );
                                        if report.items_imported > 0 || report.items_updated > 0 {
                                            info!(
                                                imported = report.items_imported,
                                                updated = report.items_updated,
                                                skipped = report.items_skipped,
                                                "openclaw: periodic session sync complete"
                                            );
                                        }
                                    }
                                }
                            }
                        });
                    },
                    Err(e) => {
                        warn!("openclaw: failed to start session watcher: {e}");
                    },
                }
            }
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

    app = app.route(
        "/api/channels/msteams/{account_id}/webhook",
        axum::routing::post(
            move |axum::extract::Path(account_id): axum::extract::Path<String>,
                  axum::extract::Query(query): axum::extract::Query<HashMap<String, String>>,
                  Json(payload): Json<serde_json::Value>| {
                let teams_plugin = Arc::clone(&msteams_webhook_plugin);
                async move {
                    let secret = query.get("secret").map(String::as_str);
                    let result = {
                        let plugin = teams_plugin.read().await;
                        plugin.ingest_activity(&account_id, payload, secret).await
                    };
                    match result {
                        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))),
                        Err(e) => {
                            let msg = e.to_string();
                            if msg.contains("invalid Teams webhook secret") {
                                (
                                    StatusCode::UNAUTHORIZED,
                                    Json(serde_json::json!({ "ok": false, "error": msg })),
                                )
                            } else if msg.contains("unknown Teams account") {
                                (
                                    StatusCode::NOT_FOUND,
                                    Json(serde_json::json!({ "ok": false, "error": msg })),
                                )
                            } else {
                                (
                                    StatusCode::BAD_REQUEST,
                                    Json(serde_json::json!({ "ok": false, "error": msg })),
                                )
                            }
                        },
                    }
                }
            },
        ),
    );

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
            let mgr = crate::tls::FsCertManager::new()?;
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
            enum MetricsPersistJob {
                Save(crate::state::MetricsHistoryPoint),
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

                    // Persist via the dedicated writer, without stalling collection.
                    if let Some(tx) = metrics_persist_tx.as_ref()
                        && tx.send(MetricsPersistJob::Save(point.clone())).is_err()
                    {
                        warn!("metrics persistence writer task is unavailable");
                    }

                    // Broadcast metrics update to all connected clients.
                    let payload = crate::state::MetricsUpdatePayload { snapshot, point };
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
    log_buffer: Option<crate::logs::LogBuffer>,
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

/// Start the gateway HTTP + WebSocket server.
///
/// Thin wrapper around [`prepare_gateway`] that adds the startup banner,
/// ctrl-c handler, TLS termination, and blocks on `axum::serve`.
#[allow(clippy::expect_used)]
pub async fn start_gateway(
    bind: &str,
    port: u16,
    no_tls: bool,
    log_buffer: Option<crate::logs::LogBuffer>,
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
    let passkey_origins = banner
        .webauthn_registry
        .as_ref()
        .map(|registry| registry.get_all_origins())
        .unwrap_or_default();
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
    // Hint about Apple Container on macOS when using Docker.
    #[cfg(target_os = "macos")]
    if banner.sandbox_backend_name == "docker" {
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
                if let Err(e) =
                    crate::tls::start_http_redirect_server(&bind_clone, http_port, port, &ca_clone)
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
        crate::tls::serve_tls_with_http_redirect(tcp_listener, Arc::new(tls_cfg), app, port, bind)
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
fn is_same_origin(origin: &str, host: &str) -> bool {
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

/// Seed the `dcg-guard` hook into `~/.moltis/hooks/dcg-guard/` on first run.
///
/// Writes both `HOOK.md` and `handler.sh`. The handler gracefully no-ops when
/// `dcg` is not installed, so the hook is always eligible.
fn seed_dcg_guard_hook() {
    let hook_dir = moltis_config::data_dir().join("hooks/dcg-guard");
    let hook_md = hook_dir.join("HOOK.md");
    if hook_md.exists() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&hook_dir) {
        tracing::debug!("could not create dcg-guard hook dir: {e}");
        return;
    }
    if let Err(e) = std::fs::write(&hook_md, DCG_GUARD_HOOK_MD) {
        tracing::debug!("could not write dcg-guard HOOK.md: {e}");
    }
    let handler = hook_dir.join("handler.sh");
    if let Err(e) = std::fs::write(&handler, DCG_GUARD_HANDLER_SH) {
        tracing::debug!("could not write dcg-guard handler.sh: {e}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&handler, std::fs::Permissions::from_mode(0o755));
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

fn seed_file_if_missing(path: PathBuf, content: &str) {
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

/// Content for the seeded dcg-guard hook manifest.
const DCG_GUARD_HOOK_MD: &str = r#"+++
name = "dcg-guard"
description = "Blocks destructive commands using Destructive Command Guard (dcg)"
emoji = "🛡️"
events = ["BeforeToolCall"]
command = "./handler.sh"
timeout = 5
+++

# Destructive Command Guard (dcg)

Uses the external [dcg](https://github.com/Dicklesworthstone/destructive_command_guard)
tool to scan shell commands before execution. dcg ships 49+ pattern categories
covering filesystem, git, database, cloud, and infrastructure commands.

This hook is **seeded by default** into `~/.moltis/hooks/dcg-guard/` on first
run. When `dcg` is not installed the hook is a no-op (all commands pass through).

## Install dcg

```bash
cargo install dcg
```

Once installed, the hook will automatically start guarding destructive commands
on the next Moltis restart.
"#;

/// Content for the seeded dcg-guard handler script.
const DCG_GUARD_HANDLER_SH: &str = r#"#!/usr/bin/env bash
# Hook handler: translates Moltis BeforeToolCall payload to dcg format.
# When dcg is not installed the hook is a no-op (all commands pass through).

set -euo pipefail

# Gracefully skip when dcg is not installed.
if ! command -v dcg >/dev/null 2>&1; then
    cat >/dev/null   # drain stdin
    exit 0
fi

INPUT=$(cat)

# Only inspect exec tool calls.
TOOL_NAME=$(printf '%s' "$INPUT" | grep -o '"tool_name":"[^"]*"' | head -1 | cut -d'"' -f4)
if [ "$TOOL_NAME" != "exec" ]; then
    exit 0
fi

# Extract the command string from the arguments object.
COMMAND=$(printf '%s' "$INPUT" | grep -o '"command":"[^"]*"' | head -1 | cut -d'"' -f4)
if [ -z "$COMMAND" ]; then
    exit 0
fi

# Build the payload dcg expects and pipe it in.
DCG_INPUT=$(printf '{"tool_name":"Bash","tool_input":{"command":"%s"}}' "$COMMAND")
DCG_RESULT=$(printf '%s' "$DCG_INPUT" | dcg 2>&1) || {
    # dcg returned non-zero — command is destructive.
    echo "$DCG_RESULT" >&2
    exit 1
}

# dcg returned 0 — command is safe.
exit 0
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
    session_store: Option<&Arc<SessionStore>>,
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
                Some(parsed.source_path.clone()),
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
    use {
        super::*,
        std::collections::{HashMap, HashSet},
    };

    #[test]
    fn summarize_model_ids_for_logs_returns_all_when_within_limit() {
        let model_ids = vec!["a", "b", "c"]
            .into_iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();

        let summary = summarize_model_ids_for_logs(&model_ids, 8);
        assert_eq!(summary, model_ids);
    }

    #[test]
    fn summarize_model_ids_for_logs_truncates_to_head_and_tail() {
        let model_ids = vec!["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]
            .into_iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();

        let summary = summarize_model_ids_for_logs(&model_ids, 7);
        let expected = vec!["a", "b", "c", "...", "h", "i", "j"]
            .into_iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();

        assert_eq!(summary, expected);
    }

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
        let session_store = Arc::new(SessionStore::new(sessions_dir));

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
        let session_store = Arc::new(SessionStore::new(sessions_dir));

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
            auth::CredentialStore::with_config(pool, &moltis_config::AuthConfig::default())
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
            auth::CredentialStore::with_config(pool, &moltis_config::AuthConfig::default())
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
            auth::CredentialStore::with_config(pool, &moltis_config::AuthConfig::default())
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
            auth::CredentialStore::with_config(pool, &moltis_config::AuthConfig::default())
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

    #[test]
    fn proxy_tls_validation_rejects_common_misconfiguration() {
        let err = validate_proxy_tls_configuration(true, true, false)
            .expect_err("behind proxy with TLS should fail without explicit override");
        let message = err.to_string();
        assert!(message.contains("MOLTIS_BEHIND_PROXY=true"));
        assert!(message.contains("--no-tls"));
    }

    #[test]
    fn proxy_tls_validation_allows_proxy_mode_when_tls_is_disabled() {
        assert!(validate_proxy_tls_configuration(true, false, false).is_ok());
    }

    #[test]
    fn proxy_tls_validation_allows_explicit_tls_override() {
        assert!(validate_proxy_tls_configuration(true, true, true).is_ok());
    }

    #[test]
    fn merge_env_overrides_keeps_existing_config_values() {
        let base = HashMap::from([
            ("OPENAI_API_KEY".to_string(), "config-openai".to_string()),
            ("BRAVE_API_KEY".to_string(), "config-brave".to_string()),
        ]);
        let merged = merge_env_overrides(&base, vec![
            ("OPENAI_API_KEY".to_string(), "db-openai".to_string()),
            (
                "PERPLEXITY_API_KEY".to_string(),
                "db-perplexity".to_string(),
            ),
        ]);
        assert_eq!(
            merged.get("OPENAI_API_KEY").map(String::as_str),
            Some("config-openai")
        );
        assert_eq!(
            merged.get("PERPLEXITY_API_KEY").map(String::as_str),
            Some("db-perplexity")
        );
        assert_eq!(
            merged.get("BRAVE_API_KEY").map(String::as_str),
            Some("config-brave")
        );
    }

    #[test]
    fn env_value_with_overrides_uses_override_when_process_env_missing() {
        let unique_key = format!("MOLTIS_TEST_LOOKUP_{}", std::process::id());
        let overrides = HashMap::from([(unique_key.clone(), "override-value".to_string())]);
        assert_eq!(
            env_value_with_overrides(&overrides, &unique_key).as_deref(),
            Some("override-value")
        );
    }
}
