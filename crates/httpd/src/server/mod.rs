//! HTTP server entry points, middleware stack, and router construction.
//!
//! This module contains the HTTP-specific layer of the moltis gateway:
//! `AppState`, router building, middleware, handlers, and server startup.
//! Core business logic lives in `moltis-gateway`; this crate depends on it
//! but never the reverse.

use std::{net::SocketAddr, path::PathBuf, sync::Arc};

#[cfg(feature = "ngrok")]
use std::sync::Weak;

use {
    axum::{
        Router,
        extract::{ConnectInfo, State, WebSocketUpgrade},
        http::StatusCode,
        response::{IntoResponse, Json},
    },
    tracing::{info, warn},
};

use moltis_protocol::TICK_INTERVAL_MS;

use moltis_sessions::session_events::{SessionEvent, SessionEventBus};

#[cfg(feature = "ngrok")]
use secrecy::ExposeSecret;

#[cfg(feature = "ngrok")]
use tokio_util::sync::CancellationToken;

use moltis_gateway::{
    auth,
    auth_webauthn::SharedWebAuthnRegistry,
    broadcast::{BroadcastOpts, broadcast, broadcast_tick},
    methods::MethodRegistry,
    state::GatewayState,
    update_check::{UPDATE_CHECK_INTERVAL, fetch_update_availability, resolve_releases_url},
};

use crate::ws::handle_connection;

#[cfg(feature = "tailscale")]
use moltis_gateway::tailscale::{CliTailscaleManager, TailscaleManager, TailscaleMode};

#[cfg(feature = "tls")]
use moltis_tls::CertManager;

// ── Submodules ───────────────────────────────────────────────────────────────

mod builder;
mod gateway;
mod handlers;
mod middleware;
mod ngrok;
mod runtime;

// ── Re-exports ───────────────────────────────────────────────────────────────
//
// Keep the public API surface identical to the original single-file module.

pub use {
    builder::{build_gateway_app, build_gateway_base, finalize_gateway_app},
    gateway::prepare_gateway,
    runtime::{prepare_httpd_embedded, start_gateway},
};

#[cfg(feature = "ngrok")]
use ngrok::NgrokActiveTunnel;
#[cfg(feature = "ngrok")]
pub use ngrok::{NgrokController, NgrokRuntimeStatus};

pub use handlers::is_same_origin;

// Bring submodule internals into scope so that `handlers.rs` and `runtime.rs`
// (which use `use super::*;`) can access peer-module items via glob import.
//
// `handlers::*` — runtime.rs relies on `resolve_outbound_ip`, `tls_runtime_sans`,
//   `startup_bind_line`, `startup_passkey_origin_lines`, `startup_setup_code_lines`.
// `builder::build_gateway_base_internal` — handlers.rs tests (ngrok-only).
// `runtime::{attach_ngrok_controller_owner, ngrok_loopback_has_proxy_headers}` —
//   handlers.rs tests (ngrok-only).
#[cfg(feature = "ngrok")]
#[cfg(test)]
use builder::build_gateway_base_internal;
#[allow(unused_imports)]
use handlers::*;
#[cfg(feature = "ngrok")]
#[cfg(test)]
use runtime::{attach_ngrok_controller_owner, ngrok_loopback_has_proxy_headers};

// ── Shared app state ─────────────────────────────────────────────────────────

/// Options for tailscale serve/funnel passed from CLI flags.
#[cfg(feature = "tailscale")]
pub struct TailscaleOpts {
    pub mode: String,
    pub reset_on_exit: bool,
}

#[derive(Clone)]
pub struct AppState {
    pub gateway: Arc<GatewayState>,
    pub methods: Arc<MethodRegistry>,
    pub request_throttle: Arc<crate::request_throttle::RequestThrottle>,
    pub webauthn_registry: Option<SharedWebAuthnRegistry>,
    #[cfg(feature = "ngrok")]
    pub ngrok_controller_owner: Option<Arc<NgrokController>>,
    #[cfg(feature = "ngrok")]
    pub ngrok_controller: Weak<NgrokController>,
    #[cfg(feature = "ngrok")]
    pub ngrok_runtime: Arc<tokio::sync::RwLock<Option<NgrokRuntimeStatus>>>,
    #[cfg(feature = "push-notifications")]
    pub push_service: Option<Arc<moltis_gateway::push::PushService>>,
    #[cfg(feature = "graphql")]
    pub graphql_schema: moltis_graphql::MoltisSchema,
}

/// Function signature for adding extra routes (e.g. web-UI) to the gateway.
pub type RouteEnhancer = fn() -> Router<AppState>;

#[cfg(feature = "ngrok")]
type GatewayBase = (Router<AppState>, AppState, Arc<NgrokController>);

#[cfg(not(feature = "ngrok"))]
type GatewayBase = (Router<AppState>, AppState);

// ── Prepared gateway types ───────────────────────────────────────────────────

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
    /// Keeps the trusted-network proxy alive for the server's full lifetime.
    /// Dropping this sender closes the watch channel, which is the proxy's
    /// shutdown signal.
    #[cfg(feature = "trusted-network")]
    pub _proxy_shutdown_tx: Option<tokio::sync::watch::Sender<bool>>,
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
    #[cfg(feature = "ngrok")]
    pub ngrok_controller: Arc<NgrokController>,
    pub browser_for_lifecycle: Arc<dyn moltis_gateway::services::BrowserService>,
    pub browser_tool_for_warmup: Option<Arc<dyn moltis_agents::tool_registry::AgentTool>>,
    pub config: moltis_config::schema::MoltisConfig,
    #[cfg(feature = "tailscale")]
    pub tailscale_mode: TailscaleMode,
    #[cfg(feature = "tailscale")]
    pub tailscale_reset_on_exit: bool,
}
