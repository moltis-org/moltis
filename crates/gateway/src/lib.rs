//! Gateway: central WebSocket/HTTP server, protocol dispatch, session/node registry.
//!
//! Lifecycle:
//! 1. Load + validate config
//! 2. Resolve auth, bind address
//! 3. Start HTTP server (health, control UI, hooks)
//! 4. Attach WebSocket upgrade handler
//! 5. Start channel accounts, cron, maintenance timers
//!
//! All domain logic (agents, channels, etc.) lives in other crates and is
//! invoked through method handlers registered in `methods.rs`.

pub mod approval;
pub mod auth;
pub mod auth_middleware;
pub mod auth_routes;
pub mod auth_webauthn;
pub mod broadcast;
pub mod channel;
pub mod channel_events;
pub mod channel_store;
pub mod chat;
pub mod chat_error;
pub mod cron;
pub mod env_routes;
#[cfg(feature = "local-llm")]
pub mod local_llm_setup;
pub mod logs;
pub mod mcp_health;
pub mod mcp_service;
pub mod message_log_store;
pub mod methods;
#[cfg(feature = "metrics")]
pub mod metrics_middleware;
#[cfg(feature = "metrics")]
pub mod metrics_routes;
pub mod nodes;
pub mod onboarding;
pub mod pairing;
pub mod project;
pub mod provider_setup;
pub mod server;
pub mod services;
pub mod session;
pub mod state;
#[cfg(feature = "tailscale")]
pub mod tailscale;
#[cfg(feature = "tailscale")]
pub mod tailscale_routes;
#[cfg(feature = "tls")]
pub mod tls;
pub mod ws;

/// Run database migrations for the gateway crate.
///
/// This creates the auth tables (auth_password, passkeys, api_keys, auth_sessions),
/// env_variables, message_log, and channels tables. Should be called at application
/// startup after the other crate migrations (projects, sessions, cron).
pub async fn run_migrations(pool: &sqlx::SqlitePool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations")
        .set_ignore_missing(true)
        .run(pool)
        .await?;
    Ok(())
}
