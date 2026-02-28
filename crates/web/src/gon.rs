//! `/api/gon` handler — returns server-side gon data as JSON.

use {
    axum::{Json, extract::State, response::IntoResponse},
    moltis_gateway::server::AppState,
};

use crate::templates::build_gon_data;

pub async fn api_gon_handler(State(state): State<AppState>) -> impl IntoResponse {
    Json(build_gon_data(&state.gateway).await)
}
