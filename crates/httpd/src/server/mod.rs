//! HTTP server entry points, middleware stack, and router construction.
//!
//! This module contains the HTTP-specific layer of the moltis gateway:
//! `AppState`, router building, middleware, handlers, and server startup.
//! Core business logic lives in `moltis-gateway`; this crate depends on it
//! but never the reverse.

use std::{net::SocketAddr, path::PathBuf, sync::Arc};

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
    state::GatewayState,
    update_check::{UPDATE_CHECK_INTERVAL, fetch_update_availability, resolve_releases_url},
};

#[cfg(feature = "ngrok")]
#[cfg(test)]
use moltis_gateway::methods::MethodRegistry;

use crate::ws::handle_connection;

#[cfg(feature = "tailscale")]
use moltis_gateway::tailscale::{CliTailscaleManager, TailscaleManager, TailscaleMode};

#[cfg(feature = "tls")]
use moltis_tls::CertManager;

// ── Submodules ───────────────────────────────────────────────────────────────

mod builder;
#[cfg(feature = "cloudflare-tunnel")]
mod cloudflare_tunnel;
mod gateway;
mod handlers;
mod middleware;
#[cfg(feature = "netbird")]
mod netbird;
mod ngrok;
mod runtime;
mod types;

// ── Re-exports ───────────────────────────────────────────────────────────────
//
// Keep the public API surface identical to the original single-file module.

pub use {
    builder::{build_gateway_app, build_gateway_base, finalize_gateway_app},
    gateway::prepare_gateway,
    runtime::{prepare_httpd_embedded, start_gateway},
    types::*,
};

#[cfg(feature = "cloudflare-tunnel")]
pub use cloudflare_tunnel::{CloudflareTunnelController, CloudflareTunnelRuntimeStatus};
#[cfg(feature = "netbird")]
pub use netbird::{NetbirdController, NetbirdRuntimeStatus};
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
