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

async fn status_handler() -> impl IntoResponse {
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
        Ok(status) => Json(status).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(netbird_error("NETBIRD_STATUS_FAILED", error.to_string())),
        )
            .into_response(),
    }
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
        .apply(&updated.netbird, updated.server.port, updated.tls.enabled)
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
