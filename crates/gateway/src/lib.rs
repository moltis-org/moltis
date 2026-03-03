//! Gateway: core business logic, protocol dispatch, session/node registry.
//!
//! Lifecycle:
//! 1. Load + validate config
//! 2. Resolve auth, bind address
//! 3. Build core gateway state (sessions, services, methods)
//! 4. Spawn background tasks (cron, update checks, MCP health)
//!
//! HTTP transport (routes, middleware, WebSocket upgrade) lives in `moltis-httpd`.
//! All domain logic (agents, channels, etc.) lives in other crates and is
//! invoked through method handlers registered in `methods.rs`.

pub mod agent_persona;
pub mod approval;
pub mod auth;
pub mod auth_webauthn;
pub mod broadcast;
pub mod channel;
pub mod channel_agent_tools;
pub mod channel_events;
pub mod channel_store;
pub mod channel_webhook_dedup;
pub mod channel_webhook_middleware;
pub mod channel_webhook_rate_limit;
pub mod chat;
pub mod chat_error;
pub mod cron;
#[cfg(feature = "local-llm")]
pub mod local_llm_setup;
pub mod logs;
pub mod mcp_health;
pub mod mcp_service;
#[cfg(feature = "mdns")]
pub mod mdns;
pub mod message_log_store;
pub mod methods;
pub mod network_audit;
pub mod nodes;
pub mod onboarding;
pub mod pairing;
pub mod project;
pub mod provider_setup;
#[cfg(feature = "push-notifications")]
pub mod push;
pub mod server;
pub mod services;
pub mod session;
pub mod session_types;
pub mod share_store;
pub mod state;
#[cfg(feature = "tailscale")]
pub mod tailscale;
#[cfg(feature = "tls")]
pub mod tls;
pub mod tts_phrases;
pub mod update_check;
pub mod voice;
pub mod voice_agent_tools;

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
