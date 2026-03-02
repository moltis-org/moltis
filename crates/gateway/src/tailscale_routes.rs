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

use crate::{
    server::AppState,
    tailscale::{CliTailscaleManager, TailscaleManager, TailscaleMode, validate_tailscale_config},
};

const TAILSCALE_STATUS_FAILED: &str = "TAILSCALE_STATUS_FAILED";
const TAILSCALE_MODE_INVALID: &str = "TAILSCALE_MODE_INVALID";
const TAILSCALE_CONFIG_INVALID: &str = "TAILSCALE_CONFIG_INVALID";
const TAILSCALE_CONFIGURE_FAILED: &str = "TAILSCALE_CONFIGURE_FAILED";

async fn sync_webauthn_host_and_notice(state: &AppState, hostname: Option<&str>) -> Option<String> {
    let hostname = hostname?;
    let normalized = crate::auth_webauthn::normalize_host(hostname);
    if normalized.is_empty() {
        return None;
    }

    let registry = state.webauthn_registry.as_ref()?;
    if registry.read().await.contains_host(&normalized) {
        return None;
    }

    let scheme = if state.gateway.tls_active {
        "https"
    } else {
        "http"
    };
    let origin = format!("{scheme}://{normalized}:{}", state.gateway.port);
    let origin_url = match webauthn_rs::prelude::Url::parse(&origin) {
        Ok(url) => url,
        Err(e) => {
            warn!(host = %normalized, origin = %origin, "invalid runtime WebAuthn origin: {e}");
            return None;
        },
    };
    let webauthn = match crate::auth_webauthn::WebAuthnState::new(&normalized, &origin_url, &[]) {
        Ok(wa) => wa,
        Err(e) => {
            warn!(host = %normalized, "failed to initialize runtime WebAuthn RP: {e}");
            return None;
        },
    };

    {
        let mut reg = registry.write().await;
        if reg.contains_host(&normalized) {
            return None;
        }
        reg.add(normalized.clone(), webauthn);
        info!(
            host = %normalized,
            origins = ?reg.get_all_origins(),
            "WebAuthn RP registered from tailscale runtime status"
        );
    }

    let has_passkeys = if let Some(store) = state.gateway.credential_store.as_ref() {
        store.has_passkeys().await.unwrap_or(false)
    } else {
        false
    };

    if has_passkeys {
        state
            .gateway
            .add_passkey_host_update_pending(&normalized)
            .await;
        Some(format!(
            "New host detected ({normalized}). Existing passkeys may not work on this host. Sign in with password, then add a new passkey in Settings > Authentication."
        ))
    } else {
        None
    }
}

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
    let manager = CliTailscaleManager::new();
    match manager.status().await {
        Ok(status) => {
            let passkey_warning =
                sync_webauthn_host_and_notice(&state, status.hostname.as_deref()).await;
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

    let manager = CliTailscaleManager::new();

    let tls = state.gateway.tls_active;
    let port = state.gateway.port;
    info!(mode = %mode, port, tls, "applying tailscale mode");
    let result = match mode {
        TailscaleMode::Off => manager.disable().await,
        TailscaleMode::Serve => manager.enable_serve(port, tls).await,
        TailscaleMode::Funnel => manager.enable_funnel(port, tls).await,
    };

    match result {
        Ok(()) => {
            info!(mode = %mode, "tailscale mode applied successfully");
            let status = manager.status().await.ok();
            let passkey_warning = if let Some(ref s) = status {
                sync_webauthn_host_and_notice(&state, s.hostname.as_deref()).await
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
