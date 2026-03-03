//! `/api/gon` handler — returns server-side gon data as JSON.

use {
    axum::{Json, extract::State, response::IntoResponse},
    moltis_httpd::AppState,
};

use crate::templates::build_gon_data;

pub async fn api_gon_handler(State(state): State<AppState>) -> impl IntoResponse {
    Json(build_gon_data(&state.gateway).await)
}

#[derive(serde::Serialize)]
struct PublicIdentityPayload {
    identity: PublicIdentity,
    graphql_enabled: bool,
}

#[derive(serde::Serialize)]
struct PublicIdentity {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    emoji: Option<String>,
}

/// Public branding payload for unauthenticated discovery clients.
pub async fn api_public_identity_handler(State(state): State<AppState>) -> impl IntoResponse {
    let identity: moltis_config::ResolvedIdentity = state
        .gateway
        .services
        .onboarding
        .identity_get()
        .await
        .ok()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();

    let name = {
        let trimmed = identity.name.trim();
        if trimmed.is_empty() {
            "moltis".to_owned()
        } else {
            trimmed.to_owned()
        }
    };

    let emoji = identity.emoji.and_then(|raw| {
        let trimmed = raw.trim().to_owned();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });

    Json(PublicIdentityPayload {
        identity: PublicIdentity { name, emoji },
        graphql_enabled: state.gateway.is_graphql_enabled(),
    })
}
