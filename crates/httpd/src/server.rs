//! HTTP server entry points, middleware stack, and router construction.
//!
//! This module re-exports the HTTP-specific types from `moltis-gateway` and
//! provides the primary entry points for starting the HTTP server.
//! Future work will move the implementation here; for now this is a facade
//! so that consumers can start depending on `moltis-httpd`.

pub use moltis_gateway::server::{
    AppState,
    BannerMeta,
    PreparedGateway,
    PreparedGatewayCore,
    RouteEnhancer,
    approval_manager_from_config,
    build_gateway_app,
    build_gateway_base,
    finalize_gateway_app,
    is_same_origin,
    openclaw_detected_for_ui,
    prepare_gateway,
    prepare_gateway_core,
    prepare_gateway_embedded,
    // Canonical aliases — the gateway originals will be deprecated once
    // the implementation migrates to this crate.
    prepare_gateway_embedded as prepare_httpd_embedded,
    start_gateway,
    start_gateway as start_httpd,
};

#[cfg(feature = "tailscale")]
pub use moltis_gateway::server::TailscaleOpts;
