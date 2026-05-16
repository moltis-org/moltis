//! HTTP routes for Cloudflare Tunnel configuration and runtime status.

use {
    axum::{
        Json, Router,
        extract::State,
        http::StatusCode,
        response::IntoResponse,
        routing::{get, post},
    },
    secrecy::Secret,
    serde::Deserialize,
};

use {crate::server::AppState, moltis_config::schema::CloudflareTunnelConfig};

fn tunnel_error(code: &str, error: impl Into<String>) -> serde_json::Value {
    serde_json::json!({ "code": code, "error": error.into() })
}

fn normalize_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn token_source(config: &CloudflareTunnelConfig) -> Option<&'static str> {
    if config.token.is_some() {
        Some("config")
    } else if env_token_present() {
        Some("env")
    } else {
        None
    }
}

fn env_token_present() -> bool {
    std::env::var("CLOUDFLARE_TUNNEL_TOKEN")
        .ok()
        .and_then(|token| normalize_optional(Some(&token)))
        .is_some()
}

fn cloudflare_token_will_exist(
    clear_token: bool,
    new_token: Option<&str>,
    existing_token_present: bool,
    env_token_present: bool,
) -> bool {
    if clear_token {
        new_token.is_some() || env_token_present
    } else {
        new_token.is_some() || existing_token_present || env_token_present
    }
}

fn status_payload(
    config: &moltis_config::MoltisConfig,
    runtime: Option<crate::server::CloudflareTunnelRuntimeStatus>,
) -> serde_json::Value {
    serde_json::json!({
        "enabled": config.cloudflare_tunnel.enabled,
        "hostname": config.cloudflare_tunnel.hostname,
        "token_present": token_source(&config.cloudflare_tunnel).is_some(),
        "token_source": token_source(&config.cloudflare_tunnel),
        "public_url": runtime.as_ref().and_then(|status| status.public_url.clone()),
        "passkey_warning": runtime.and_then(|status| status.passkey_warning),
    })
}

#[derive(Deserialize)]
struct SaveCloudflareTunnelConfigRequest {
    enabled: bool,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    clear_token: bool,
    #[serde(default)]
    hostname: Option<String>,
}

pub fn cloudflare_tunnel_router() -> Router<AppState> {
    Router::new()
        .route("/status", get(status_handler))
        .route("/config", post(save_config_handler))
}

async fn status_handler(State(state): State<AppState>) -> impl IntoResponse {
    let config = moltis_config::discover_and_load();
    let runtime = state.cloudflare_tunnel_runtime.read().await.clone();
    Json(status_payload(&config, runtime)).into_response()
}

async fn save_config_handler(
    State(state): State<AppState>,
    Json(body): Json<SaveCloudflareTunnelConfigRequest>,
) -> impl IntoResponse {
    let existing = moltis_config::discover_and_load();
    let hostname = normalize_optional(body.hostname.as_deref());
    let new_token = normalize_optional(body.token.as_deref());
    let token_will_exist = cloudflare_token_will_exist(
        body.clear_token,
        new_token.as_deref(),
        existing.cloudflare_tunnel.token.is_some(),
        env_token_present(),
    );

    if body.enabled && !token_will_exist {
        return (
            StatusCode::BAD_REQUEST,
            Json(tunnel_error(
                "CLOUDFLARE_TUNNEL_CONFIG_INVALID",
                "Cloudflare Tunnel requires a token in config or CLOUDFLARE_TUNNEL_TOKEN in the environment",
            )),
        )
            .into_response();
    }

    let mut updated = existing.clone();
    updated.cloudflare_tunnel.enabled = body.enabled;
    updated.cloudflare_tunnel.hostname = hostname;
    if body.clear_token {
        updated.cloudflare_tunnel.token = None;
    }
    if let Some(token) = new_token {
        updated.cloudflare_tunnel.token = Some(Secret::new(token));
    }

    if let Err(error) = moltis_config::update_config(|config| {
        config.cloudflare_tunnel = updated.cloudflare_tunnel.clone();
    }) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(tunnel_error(
                "CLOUDFLARE_TUNNEL_SAVE_FAILED",
                format!("failed to save Cloudflare Tunnel config: {error}"),
            )),
        )
            .into_response();
    }

    if let Err(error) = state
        .cloudflare_tunnel_controller
        .apply(
            &updated.cloudflare_tunnel,
            &updated.server.bind,
            updated.server.port,
            updated.tls.enabled,
        )
        .await
    {
        let runtime = state.cloudflare_tunnel_runtime.read().await.clone();
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "code": "CLOUDFLARE_TUNNEL_APPLY_FAILED",
                "error": format!("saved Cloudflare Tunnel config but failed to apply it: {error}"),
                "status": status_payload(&updated, runtime),
            })),
        )
            .into_response();
    }

    let runtime = state.cloudflare_tunnel_runtime.read().await.clone();
    Json(serde_json::json!({ "ok": true, "status": status_payload(&updated, runtime) }))
        .into_response()
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
        secrecy::Secret,
    };

    #[cfg(feature = "netbird")]
    use crate::server::NetbirdController;
    #[cfg(feature = "ngrok")]
    use crate::server::NgrokRuntimeStatus;
    use crate::server::{AppState, CloudflareTunnelController, CloudflareTunnelRuntimeStatus};

    use super::*;

    #[test]
    fn normalize_optional_trims_blank_values() {
        assert_eq!(
            normalize_optional(Some("  example.com  ")),
            Some("example.com".to_string())
        );
        assert_eq!(normalize_optional(Some("   ")), None);
        assert_eq!(normalize_optional(None), None);
    }

    #[test]
    fn status_payload_reports_config_token_and_runtime() {
        let mut config = moltis_config::MoltisConfig::default();
        config.cloudflare_tunnel.enabled = true;
        config.cloudflare_tunnel.hostname = Some("moltis.example.com".to_string());
        config.cloudflare_tunnel.token = Some(Secret::new("token".to_string()));
        let runtime = CloudflareTunnelRuntimeStatus {
            public_url: Some("https://moltis.example.com".to_string()),
            hostname: Some("moltis.example.com".to_string()),
            passkey_warning: Some("register passkey origin".to_string()),
        };

        let payload = status_payload(&config, Some(runtime));

        assert_eq!(payload["enabled"], true);
        assert_eq!(payload["hostname"], "moltis.example.com");
        assert_eq!(payload["token_present"], true);
        assert_eq!(payload["token_source"], "config");
        assert_eq!(payload["public_url"], "https://moltis.example.com");
        assert_eq!(payload["passkey_warning"], "register passkey origin");
    }

    #[test]
    fn tunnel_error_uses_stable_shape() {
        assert_eq!(
            tunnel_error("CLOUDFLARE_TUNNEL_CONFIG_INVALID", "missing token"),
            serde_json::json!({
                "code": "CLOUDFLARE_TUNNEL_CONFIG_INVALID",
                "error": "missing token",
            })
        );
    }

    #[test]
    fn token_will_exist_ignores_blank_env_tokens() {
        assert!(!cloudflare_token_will_exist(false, None, false, false));
        assert!(cloudflare_token_will_exist(false, None, false, true));
        assert!(!cloudflare_token_will_exist(true, None, true, false));
        assert!(cloudflare_token_will_exist(
            true,
            Some("token"),
            true,
            false
        ));
    }

    fn test_state() -> AppState {
        let gateway = GatewayState::new(auth::resolve_auth(None, None), GatewayServices::noop());
        let cloudflare_tunnel_runtime = Arc::new(tokio::sync::RwLock::new(None));
        #[cfg(feature = "netbird")]
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
            cloudflare_tunnel_controller: Arc::new(CloudflareTunnelController::new(
                Arc::clone(&gateway),
                None,
                Arc::clone(&cloudflare_tunnel_runtime),
            )),
            cloudflare_tunnel_runtime,
            #[cfg(feature = "netbird")]
            netbird_controller: Arc::new(NetbirdController::new(Arc::clone(&netbird_runtime))),
            #[cfg(feature = "netbird")]
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
    async fn save_config_rejects_enabled_without_token() -> Result<(), Box<dyn std::error::Error>> {
        let tempdir = tempfile::tempdir()?;
        moltis_config::set_config_dir(tempdir.path().to_path_buf());
        moltis_config::set_data_dir(tempdir.path().to_path_buf());

        let response = save_config_handler(
            State(test_state()),
            Json(SaveCloudflareTunnelConfigRequest {
                enabled: true,
                token: None,
                clear_token: true,
                hostname: None,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await?;
        let payload: serde_json::Value = serde_json::from_slice(&body)?;
        assert_eq!(payload["code"], "CLOUDFLARE_TUNNEL_CONFIG_INVALID");
        Ok(())
    }

    #[tokio::test]
    #[serial_test::serial(config_dir)]
    async fn save_config_can_disable_without_token() -> Result<(), Box<dyn std::error::Error>> {
        let tempdir = tempfile::tempdir()?;
        moltis_config::set_config_dir(tempdir.path().to_path_buf());
        moltis_config::set_data_dir(tempdir.path().to_path_buf());

        let response = save_config_handler(
            State(test_state()),
            Json(SaveCloudflareTunnelConfigRequest {
                enabled: false,
                token: None,
                clear_token: true,
                hostname: Some("  moltis.example.com  ".to_string()),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await?;
        let payload: serde_json::Value = serde_json::from_slice(&body)?;
        assert_eq!(payload["ok"], true);
        assert_eq!(payload["status"]["hostname"], "moltis.example.com");
        assert_eq!(payload["status"]["token_present"], false);
        Ok(())
    }
}
