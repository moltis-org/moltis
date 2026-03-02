//! HTTP/WebSocket transport layer for the moltis gateway.
//!
//! This crate provides the HTTP server, WebSocket upgrade handler,
//! authentication middleware, and all HTTP route handlers. It depends
//! on `moltis-gateway` for core business logic but never the reverse.
//!
//! Non-HTTP consumers (TUI, tests) can depend on `moltis-gateway`
//! directly without pulling in the HTTP stack.

pub mod server;

// Re-export HTTP modules from gateway.
// These modules will migrate here in a future step; for now we expose
// them through moltis-httpd so that consumers can start depending on
// this crate.
pub use moltis_gateway::{
    auth_middleware, auth_routes, env_routes, request_throttle, tools_routes, upload_routes, ws,
};

#[cfg(feature = "graphql")]
pub use moltis_gateway::graphql_routes;
#[cfg(feature = "metrics")]
pub use moltis_gateway::metrics_middleware;
#[cfg(feature = "metrics")]
pub use moltis_gateway::metrics_routes;
#[cfg(feature = "push-notifications")]
pub use moltis_gateway::push_routes;
#[cfg(feature = "tailscale")]
pub use moltis_gateway::tailscale_routes;

// Re-export key types for consumers.
#[cfg(feature = "tailscale")]
pub use server::TailscaleOpts;
pub use server::{
    AppState, PreparedGateway, RouteEnhancer, prepare_httpd_embedded, start_gateway, start_httpd,
};
