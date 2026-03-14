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
    let auth_setup_pending = state
        .gateway
        .credential_store
        .as_ref()
        .is_some_and(|store| store.is_auth_disabled() || !store.is_setup_complete());

    if should_redirect_from_onboarding(onboarded, auth_setup_pending) {
        return Redirect::to("/").into_response();
    }

    render_spa_template(&state.gateway, SpaTemplate::Onboarding).await
}

pub async fn login_handler_page(State(state): State<AppState>) -> impl IntoResponse {
    render_spa_template(&state.gateway, SpaTemplate::Login).await
}

pub async fn setup_required_handler(State(state): State<AppState>) -> impl IntoResponse {
    // If auth is already configured, redirect so stale bookmarks don't show
    // a misleading "Authentication Not Configured" page.
    if let Some(ref store) = state.gateway.credential_store
        && store.is_setup_complete()
    {
        return Redirect::to("/login").into_response();
    }
    render_spa_template(&state.gateway, SpaTemplate::SetupRequired)
        .await
        .into_response()
}
