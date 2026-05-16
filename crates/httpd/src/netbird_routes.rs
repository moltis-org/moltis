//! HTTP routes for NetBird private mesh status and configuration.

use {
    axum::{
        Json, Router,
        extract::State,
        http::StatusCode,
        response::IntoResponse,
        routing::{get, post},
    },
    serde::Deserialize,
};

use crate::server::AppState;

#[cfg(feature = "netbird")]
use crate::server::NetbirdRuntimeStatus;

fn netbird_error(code: &str, error: impl Into<String>) -> serde_json::Value {
    serde_json::json!({ "code": code, "error": error.into() })
}

#[derive(Deserialize)]
struct ConfigureNetbirdRequest {
    mode: String,
}

pub fn netbird_router() -> Router<AppState> {
    Router::new()
        .route("/status", get(status_handler))
        .route("/configure", post(configure_handler))
}

async fn status_handler(State(state): State<AppState>) -> impl IntoResponse {
    let config = moltis_config::discover_and_load();
    let mode = config
        .netbird
        .mode
        .parse::<moltis_gateway::netbird::NetbirdMode>()
        .unwrap_or_default();
    let manager = moltis_gateway::netbird::CliNetbirdManager::new(
        mode,
        config.server.port,
        config.tls.enabled,
    );
    match moltis_gateway::netbird::NetbirdManager::status(&manager).await {
        Ok(status) => {
            let runtime = state.netbird_runtime.read().await.clone();
            Json(status_with_runtime_forwarder(status, runtime)).into_response()
        },
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(netbird_error("NETBIRD_STATUS_FAILED", error.to_string())),
        )
            .into_response(),
    }
}

#[cfg(feature = "netbird")]
fn status_with_runtime_forwarder(
    mut status: moltis_gateway::netbird::NetbirdStatus,
    runtime: Option<NetbirdRuntimeStatus>,
) -> moltis_gateway::netbird::NetbirdStatus {
    status.url = runtime.map(|runtime| runtime.url);
    status
}

async fn configure_handler(
    State(state): State<AppState>,
    Json(body): Json<ConfigureNetbirdRequest>,
) -> impl IntoResponse {
    let existing = moltis_config::discover_and_load();
    let mode = match body.mode.parse::<moltis_gateway::netbird::NetbirdMode>() {
        Ok(mode) => mode,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(netbird_error("NETBIRD_MODE_INVALID", error.to_string())),
            )
                .into_response();
        },
    };

    if let Err(error) =
        moltis_gateway::netbird::validate_netbird_config(mode, &existing.server.bind)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(netbird_error("NETBIRD_CONFIG_INVALID", error.to_string())),
        )
            .into_response();
    }

    let mut updated = existing.clone();
    updated.netbird.mode = mode.to_string();

    if let Err(error) = moltis_config::update_config(|config| {
        config.netbird.mode = mode.to_string();
    }) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(netbird_error(
                "NETBIRD_SAVE_FAILED",
                format!("failed to save NetBird config: {error}"),
            )),
        )
            .into_response();
    }

    if let Err(error) = state
        .netbird_controller
        .apply(
            &updated.netbird,
            &updated.server.bind,
            updated.server.port,
            updated.tls.enabled,
        )
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(netbird_error(
                "NETBIRD_APPLY_FAILED",
                format!("saved NetBird config but failed to apply it: {error}"),
            )),
        )
            .into_response();
    }

    Json(serde_json::json!({ "ok": true })).into_response()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    #[cfg(feature = "ngrok")]
    use std::sync::Weak;

    use {
        axum::body::to_bytes,
        moltis_gateway::{
            auth, methods::MethodRegistry, services::GatewayServices, state::GatewayState,
        },
    };

    #[cfg(feature = "cloudflare-tunnel")]
    use crate::server::CloudflareTunnelController;
    #[cfg(feature = "ngrok")]
    use crate::server::NgrokRuntimeStatus;
    use crate::server::{AppState, NetbirdController};

    use super::*;

    #[test]
    fn netbird_error_uses_stable_shape() {
        assert_eq!(
            netbird_error("NETBIRD_MODE_INVALID", "invalid mode"),
            serde_json::json!({
                "code": "NETBIRD_MODE_INVALID",
                "error": "invalid mode",
            })
        );
    }

    #[test]
    fn configure_request_deserializes_mode() -> Result<(), serde_json::Error> {
        let request: ConfigureNetbirdRequest = serde_json::from_value(serde_json::json!({
            "mode": "serve",
        }))?;

        assert_eq!(request.mode, "serve");
        Ok(())
    }

    #[test]
    fn status_without_runtime_does_not_expose_forwarder_url() {
        let status = status_with_runtime_forwarder(
            moltis_gateway::netbird::NetbirdStatus {
                mode: moltis_gateway::netbird::NetbirdMode::Serve,
                installed: true,
                netbird_up: true,
                version: Some("netbird version 0.42.0".to_string()),
                peer_ip: Some("100.96.0.10".to_string()),
                dns_name: Some("moltis.netbird.cloud".to_string()),
                url: Some("https://moltis.netbird.cloud:8080".to_string()),
            },
            None,
        );

        assert_eq!(status.mode, moltis_gateway::netbird::NetbirdMode::Serve);
        assert_eq!(status.peer_ip.as_deref(), Some("100.96.0.10"));
        assert!(status.url.is_none());
    }

    #[test]
    fn status_with_runtime_exposes_active_forwarder_url() {
        let status = status_with_runtime_forwarder(
            moltis_gateway::netbird::NetbirdStatus {
                mode: moltis_gateway::netbird::NetbirdMode::Serve,
                installed: true,
                netbird_up: true,
                version: Some("netbird version 0.42.0".to_string()),
                peer_ip: Some("100.96.0.10".to_string()),
                dns_name: Some("moltis.netbird.cloud".to_string()),
                url: Some("https://moltis.netbird.cloud:8080".to_string()),
            },
            Some(NetbirdRuntimeStatus {
                url: "https://100.96.0.10:8080".to_string(),
                peer_ip: "100.96.0.10".to_string(),
            }),
        );

        assert_eq!(status.url.as_deref(), Some("https://100.96.0.10:8080"));
    }

    fn test_state() -> AppState {
        let gateway = GatewayState::new(auth::resolve_auth(None, None), GatewayServices::noop());
        #[cfg(feature = "cloudflare-tunnel")]
        let cloudflare_tunnel_runtime = Arc::new(tokio::sync::RwLock::new(None));
        let netbird_runtime = Arc::new(tokio::sync::RwLock::new(None));

        AppState {
            gateway: Arc::clone(&gateway),
            methods: Arc::new(MethodRegistry::new()),
            request_throttle: Arc::new(crate::request_throttle::RequestThrottle::new()),
            webauthn_registry: None,
            #[cfg(feature = "ngrok")]
            ngrok_controller_owner: None,
            #[cfg(feature = "ngrok")]
            ngrok_controller: Weak::new(),
            #[cfg(feature = "ngrok")]
            ngrok_runtime: Arc::new(tokio::sync::RwLock::new(Some(NgrokRuntimeStatus {
                public_url: "https://existing.ngrok.app".to_string(),
                passkey_warning: None,
            }))),
            #[cfg(feature = "cloudflare-tunnel")]
            cloudflare_tunnel_controller: Arc::new(CloudflareTunnelController::new(
                Arc::clone(&gateway),
                None,
                Arc::clone(&cloudflare_tunnel_runtime),
            )),
            #[cfg(feature = "cloudflare-tunnel")]
            cloudflare_tunnel_runtime,
            netbird_controller: Arc::new(NetbirdController::new(Arc::clone(&netbird_runtime))),
            netbird_runtime,
            #[cfg(feature = "tailscale")]
            tailscale_manager: moltis_gateway::tailscale::CachedTailscaleManager::new_with_prefetch(
            ),
            #[cfg(feature = "push-notifications")]
            push_service: None,
            #[cfg(feature = "graphql")]
            graphql_schema: crate::graphql_routes::build_graphql_schema(GatewayState::new(
                auth::resolve_auth(None, None),
                GatewayServices::noop(),
            )),
        }
    }

    #[tokio::test]
    #[serial_test::serial(config_dir)]
    async fn configure_rejects_invalid_mode() -> Result<(), Box<dyn std::error::Error>> {
        let tempdir = tempfile::tempdir()?;
        moltis_config::set_config_dir(tempdir.path().to_path_buf());
        moltis_config::set_data_dir(tempdir.path().to_path_buf());

        let response = configure_handler(
            State(test_state()),
            Json(ConfigureNetbirdRequest {
                mode: "invalid".to_string(),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await?;
        let payload: serde_json::Value = serde_json::from_slice(&body)?;
        assert_eq!(payload["code"], "NETBIRD_MODE_INVALID");
        Ok(())
    }

    #[tokio::test]
    #[serial_test::serial(config_dir)]
    async fn configure_accepts_off_mode() -> Result<(), Box<dyn std::error::Error>> {
        let tempdir = tempfile::tempdir()?;
        moltis_config::set_config_dir(tempdir.path().to_path_buf());
        moltis_config::set_data_dir(tempdir.path().to_path_buf());

        let response = configure_handler(
            State(test_state()),
            Json(ConfigureNetbirdRequest {
                mode: "off".to_string(),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await?;
        let payload: serde_json::Value = serde_json::from_slice(&body)?;
        assert_eq!(payload["ok"], true);
        Ok(())
    }
}
