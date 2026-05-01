//! HTTP routes for Tailscale Serve/Funnel management.

use {
    axum::{
        Json, Router,
        extract::State,
        http::StatusCode,
        response::IntoResponse,
        routing::{get, post},
    },
    serde::Deserialize,
    tracing::{debug, error, info, warn},
};

use moltis_gateway::tailscale::{TailscaleManager, TailscaleMode, validate_tailscale_config};

use crate::server::AppState;

const TAILSCALE_STATUS_FAILED: &str = "TAILSCALE_STATUS_FAILED";
const TAILSCALE_MODE_INVALID: &str = "TAILSCALE_MODE_INVALID";
const TAILSCALE_CONFIG_INVALID: &str = "TAILSCALE_CONFIG_INVALID";
const TAILSCALE_CONFIGURE_FAILED: &str = "TAILSCALE_CONFIGURE_FAILED";

fn tailscale_error(code: &str, error: impl Into<String>) -> serde_json::Value {
    serde_json::json!({
        "code": code,
        "error": error.into(),
    })
}

#[derive(Deserialize)]
struct ConfigureRequest {
    mode: String,
}

/// Build the tailscale API router.
pub fn tailscale_router() -> Router<AppState> {
    Router::new()
        .route("/status", get(status_handler))
        .route("/configure", post(configure_handler))
}

async fn status_handler(State(state): State<AppState>) -> impl IntoResponse {
    debug!("tailscale status requested");
    let port = state.gateway.port;
    match state.tailscale_manager.status().await {
        Ok(status) => {
            let passkey_warning = moltis_gateway::server::sync_runtime_webauthn_host_and_notice(
                &state.gateway,
                state.webauthn_registry.as_ref(),
                status.hostname.as_deref(),
                None,
                "tailscale runtime status",
            )
            .await;
            debug!(
                mode = %status.mode,
                hostname = ?status.hostname,
                tailscale_up = status.tailscale_up,
                "tailscale status OK"
            );
            Json(serde_json::json!({
                "mode": status.mode,
                "hostname": status.hostname,
                "url": status.url,
                "tailscale_up": status.tailscale_up,
                "installed": status.installed,
                "tailnet": status.tailnet,
                "version": status.version,
                "login_name": status.login_name,
                "tailscale_ip": status.tailscale_ip,
                "port": port,
                "passkey_warning": passkey_warning,
            }))
            .into_response()
        },
        Err(e) => {
            error!("tailscale status failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(tailscale_error(TAILSCALE_STATUS_FAILED, e.to_string())),
            )
                .into_response()
        },
    }
}

async fn configure_handler(
    State(state): State<AppState>,
    Json(body): Json<ConfigureRequest>,
) -> impl IntoResponse {
    info!(requested_mode = %body.mode, "tailscale configure requested");

    let mode: TailscaleMode = match body.mode.parse() {
        Ok(m) => m,
        Err(e) => {
            warn!(mode = %body.mode, "invalid tailscale mode: {e}");
            return (
                StatusCode::BAD_REQUEST,
                Json(tailscale_error(TAILSCALE_MODE_INVALID, e.to_string())),
            )
                .into_response();
        },
    };

    // Check auth requirement for funnel.
    let auth_setup_complete = state
        .gateway
        .credential_store
        .as_ref()
        .is_some_and(|cs| cs.is_setup_complete());

    // Use the gateway's bind address for loopback validation.
    let bind_addr = if state.gateway.localhost_only {
        "127.0.0.1"
    } else {
        "0.0.0.0"
    };

    if let Err(e) = validate_tailscale_config(mode, bind_addr, auth_setup_complete) {
        warn!(mode = %mode, "tailscale config validation failed: {e}");
        return (
            StatusCode::BAD_REQUEST,
            Json(tailscale_error(TAILSCALE_CONFIG_INVALID, e.to_string())),
        )
            .into_response();
    }

    let tls = state.gateway.tls_active;
    let port = state.gateway.port;
    info!(mode = %mode, port, tls, "applying tailscale mode");
    let result = match mode {
        TailscaleMode::Off => state.tailscale_manager.disable().await,
        TailscaleMode::Serve => state.tailscale_manager.enable_serve(port, tls).await,
        TailscaleMode::Funnel => state.tailscale_manager.enable_funnel(port, tls).await,
    };

    match result {
        Ok(()) => {
            info!(mode = %mode, "tailscale mode applied successfully");
            let status = state.tailscale_manager.status().await.ok();
            let passkey_warning = if let Some(ref s) = status {
                moltis_gateway::server::sync_runtime_webauthn_host_and_notice(
                    &state.gateway,
                    state.webauthn_registry.as_ref(),
                    s.hostname.as_deref(),
                    None,
                    "tailscale configure",
                )
                .await
            } else {
                None
            };
            if let Some(ref s) = status {
                info!(
                    active_mode = %s.mode,
                    hostname = ?s.hostname,
                    url = ?s.url,
                    "tailscale status after configure"
                );
            }
            Json(serde_json::json!({
                "ok": true,
                "mode": mode,
                "status": status.map(|s| serde_json::json!({
                    "mode": s.mode,
                    "hostname": s.hostname,
                    "url": s.url,
                    "tailscale_up": s.tailscale_up,
                    "installed": s.installed,
                    "tailnet": s.tailnet,
                    "version": s.version,
                    "login_name": s.login_name,
                })),
                "passkey_warning": passkey_warning,
            }))
            .into_response()
        },
        Err(e) => {
            error!(mode = %mode, "tailscale configure failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(tailscale_error(TAILSCALE_CONFIGURE_FAILED, e.to_string())),
            )
                .into_response()
        },
    }
}
