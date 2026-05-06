//! HTTP routes for ngrok tunnel configuration and runtime status.

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

use crate::server::AppState;

const NGROK_CONFIG_INVALID: &str = "NGROK_CONFIG_INVALID";
const NGROK_SAVE_FAILED: &str = "NGROK_SAVE_FAILED";
const NGROK_APPLY_FAILED: &str = "NGROK_APPLY_FAILED";

fn ngrok_error(code: &str, error: impl Into<String>) -> serde_json::Value {
    serde_json::json!({
        "code": code,
        "error": error.into(),
    })
}

fn normalize_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn authtoken_source(config: &moltis_config::NgrokConfig) -> Option<&'static str> {
    if config.authtoken.is_some() {
        Some("config")
    } else if std::env::var_os("NGROK_AUTHTOKEN").is_some() {
        Some("env")
    } else {
        None
    }
}

fn status_payload_with(
    config: &moltis_config::MoltisConfig,
    runtime: Option<crate::server::NgrokRuntimeStatus>,
) -> serde_json::Value {
    let authtoken_source = authtoken_source(&config.ngrok);
    let runtime_active = runtime.is_some();

    serde_json::json!({
        "enabled": config.ngrok.enabled,
        "domain": config.ngrok.domain,
        "authtoken_present": authtoken_source.is_some(),
        "authtoken_source": authtoken_source,
        "public_url": runtime.as_ref().map(|status| status.public_url.clone()),
        "passkey_warning": runtime.and_then(|status| status.passkey_warning),
        "runtime_active": runtime_active,
    })
}

async fn status_payload(state: &AppState) -> serde_json::Value {
    let config = moltis_config::discover_and_load();
    let runtime = state.ngrok_runtime.read().await.clone();
    status_payload_with(&config, runtime)
}

#[derive(Deserialize)]
struct SaveNgrokConfigRequest {
    enabled: bool,
    #[serde(default)]
    authtoken: Option<String>,
    #[serde(default)]
    clear_authtoken: bool,
    #[serde(default)]
    domain: Option<String>,
}

/// Build the ngrok API router.
pub fn ngrok_router() -> Router<AppState> {
    Router::new()
        .route("/status", get(status_handler))
        .route("/config", post(save_config_handler))
}

async fn status_handler(State(state): State<AppState>) -> impl IntoResponse {
    Json(status_payload(&state).await).into_response()
}

async fn save_config_handler(
    State(state): State<AppState>,
    Json(body): Json<SaveNgrokConfigRequest>,
) -> impl IntoResponse {
    let existing = moltis_config::discover_and_load();
    let domain = normalize_optional(body.domain.as_deref());
    let new_authtoken = normalize_optional(body.authtoken.as_deref());
    let mut updated = existing.clone();

    let token_will_exist = if body.clear_authtoken {
        new_authtoken.is_some() || std::env::var_os("NGROK_AUTHTOKEN").is_some()
    } else {
        new_authtoken.is_some()
            || existing.ngrok.authtoken.is_some()
            || std::env::var_os("NGROK_AUTHTOKEN").is_some()
    };

    if body.enabled && !token_will_exist {
        return (
            StatusCode::BAD_REQUEST,
            Json(ngrok_error(
                NGROK_CONFIG_INVALID,
                "ngrok requires an authtoken in config or NGROK_AUTHTOKEN in the environment",
            )),
        )
            .into_response();
    }

    updated.ngrok.enabled = body.enabled;
    updated.ngrok.domain = domain.clone();
    if body.clear_authtoken {
        updated.ngrok.authtoken = None;
    }
    if let Some(authtoken) = new_authtoken.as_ref() {
        updated.ngrok.authtoken = Some(Secret::new(authtoken.clone()));
    }

    if let Err(error) = moltis_config::update_config(|config| {
        config.ngrok = updated.ngrok.clone();
    }) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ngrok_error(
                NGROK_SAVE_FAILED,
                format!("failed to save ngrok config: {error}"),
            )),
        )
            .into_response();
    }

    let Some(controller) = state.ngrok_controller.upgrade() else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ngrok_error(
                NGROK_APPLY_FAILED,
                "ngrok controller is not available in this build context",
            )),
        )
            .into_response();
    };
    if let Err(error) = controller.apply(&updated.ngrok).await {
        let runtime = state.ngrok_runtime.read().await.clone();
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "code": NGROK_APPLY_FAILED,
                "error": format!("saved ngrok config but failed to apply it: {error}"),
                "status": status_payload_with(&updated, runtime),
            })),
        )
            .into_response();
    }

    let runtime = state.ngrok_runtime.read().await.clone();
    Json(serde_json::json!({
        "ok": true,
        "status": status_payload_with(&updated, runtime),
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Weak};

    use {
        axum::{Json, body::to_bytes, extract::State},
        moltis_gateway::{
            auth, methods::MethodRegistry, services::GatewayServices, state::GatewayState,
        },
    };

    use crate::server::NgrokRuntimeStatus;

    use super::*;

    #[tokio::test]
    async fn save_config_returns_error_when_ngrok_controller_is_unavailable()
    -> Result<(), Box<dyn std::error::Error>> {
        let tempdir = tempfile::tempdir()?;
        moltis_config::set_config_dir(tempdir.path().to_path_buf());
        moltis_config::set_data_dir(tempdir.path().to_path_buf());

        let state = AppState {
            gateway: GatewayState::new(auth::resolve_auth(None, None), GatewayServices::noop()),
            methods: Arc::new(MethodRegistry::new()),
            request_throttle: Arc::new(crate::request_throttle::RequestThrottle::new()),
            webauthn_registry: None,
            ngrok_controller_owner: None,
            ngrok_controller: Weak::new(),
            ngrok_runtime: Arc::new(tokio::sync::RwLock::new(Some(NgrokRuntimeStatus {
                public_url: "https://existing.ngrok.app".to_string(),
                passkey_warning: None,
            }))),
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
        };

        let response = save_config_handler(
            State(state),
            Json(SaveNgrokConfigRequest {
                enabled: true,
                authtoken: Some("test-token".to_string()),
                clear_authtoken: false,
                domain: None,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = to_bytes(response.into_body(), usize::MAX).await?;
        let payload: serde_json::Value = serde_json::from_slice(&body)?;
        assert_eq!(payload["code"], NGROK_APPLY_FAILED);
        assert_eq!(
            payload["error"],
            "ngrok controller is not available in this build context"
        );

        Ok(())
    }
}
