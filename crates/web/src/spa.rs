//! SPA fallback, onboarding redirect, and login page handlers.

use {
    axum::{
        extract::State,
        http::Uri,
        response::{IntoResponse, Redirect},
    },
    moltis_httpd::AppState,
};

use crate::templates::{
    ErrorPageKind, SpaTemplate, is_known_spa_route, onboarding_completed, render_error_page,
    render_spa_template, should_redirect_from_onboarding, should_redirect_to_onboarding,
};

pub async fn spa_fallback(State(state): State<AppState>, uri: Uri) -> impl IntoResponse {
    let path = uri.path();
    if let Some(canonical) = canonical_standalone_path(path) {
        return Redirect::to(canonical).into_response();
    }

    if is_non_page_path(path) {
        return (axum::http::StatusCode::NOT_FOUND, "not found").into_response();
    }

    let onboarded = onboarding_completed(&state.gateway).await;
    if should_redirect_to_onboarding(path, onboarded) {
        return Redirect::to("/onboarding").into_response();
    }

    if !is_known_spa_route(path) {
        return render_error_page(
            axum::http::StatusCode::NOT_FOUND,
            ErrorPageKind::NotFound,
            Some(path),
        );
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

fn canonical_standalone_path(path: &str) -> Option<&'static str> {
    match path {
        "/onboarding/" => Some("/onboarding"),
        "/login/" => Some("/login"),
        "/setup-required/" => Some("/setup-required"),
        _ => None,
    }
}

fn is_non_page_path(path: &str) -> bool {
    path.starts_with("/assets/")
        || path.starts_with("/api/")
        || path == "/ws"
        || path.starts_with("/ws/")
        || path.starts_with("/auth/")
        || path.contains('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalizes_standalone_paths() {
        assert_eq!(
            canonical_standalone_path("/onboarding/"),
            Some("/onboarding")
        );
        assert_eq!(canonical_standalone_path("/login/"), Some("/login"));
        assert_eq!(
            canonical_standalone_path("/setup-required/"),
            Some("/setup-required")
        );
        assert_eq!(canonical_standalone_path("/chats/main/"), None);
    }

    #[test]
    fn filters_non_page_prefixes() {
        assert!(is_non_page_path("/api/unknown"));
        assert!(is_non_page_path("/assets/js/missing.js"));
        assert!(is_non_page_path("/ws"));
        assert!(is_non_page_path("/ws/chat"));
        assert!(is_non_page_path("/favicon.ico"));
        assert!(!is_non_page_path("/ws-hook"));
        assert!(!is_non_page_path("/does-not-exist"));
        assert!(!is_non_page_path("/settings/identity"));
    }
}
