//! SPA fallback, onboarding redirect, and login page handlers.

use {
    axum::{
        extract::State,
        http::{StatusCode, Uri},
        response::{IntoResponse, Redirect},
    },
    moltis_gateway::server::AppState,
};

use crate::templates::{
    SpaTemplate, onboarding_completed, render_spa_template, should_redirect_from_onboarding,
    should_redirect_to_onboarding,
};

pub async fn spa_fallback(State(state): State<AppState>, uri: Uri) -> impl IntoResponse {
    let path = uri.path();
    if path.starts_with("/assets/") || path.contains('.') {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }

    let onboarded = onboarding_completed(&state.gateway).await;
    if should_redirect_to_onboarding(path, onboarded) {
        return Redirect::to("/onboarding").into_response();
    }
    render_spa_template(&state.gateway, SpaTemplate::Index).await
}

pub async fn onboarding_handler(State(state): State<AppState>) -> impl IntoResponse {
    let onboarded = onboarding_completed(&state.gateway).await;

    if should_redirect_from_onboarding(onboarded) {
        return Redirect::to("/").into_response();
    }

    render_spa_template(&state.gateway, SpaTemplate::Onboarding).await
}

pub async fn login_handler_page(State(state): State<AppState>) -> impl IntoResponse {
    render_spa_template(&state.gateway, SpaTemplate::Login).await
}
