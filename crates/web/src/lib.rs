//! Web UI: static asset serving, SPA routing, share pages, terminal PTY,
//! and all browser-facing handlers.
//!
//! This crate depends on `moltis-gateway` for [`AppState`] and server-side
//! services. It provides [`web_routes()`] which returns an Axum `Router` that
//! the CLI (or any other host) can merge into the gateway router.

pub mod api;
pub mod assets;
pub mod error;
pub mod gon;
pub mod oauth;
pub mod share;
pub mod share_render;
pub mod spa;
pub mod templates;
pub mod terminal;

pub use error::{Error, Result};

use {
    axum::{Router, routing::get},
    moltis_gateway::server::AppState,
};

/// Build the web-UI router: pages, API routes, assets, and SPA fallback.
///
/// Does **not** include the `auth_gate` middleware — the caller applies it
/// globally after merging with the gateway base router (matching the old
/// architecture where auth_gate ran on every inbound request).
pub fn web_routes() -> Router<AppState> {
    let api = build_api_routes();
    let api = add_feature_routes(api);

    Router::new()
        .route("/auth/callback", get(oauth::oauth_callback_handler))
        .route(
            "/share/{share_id}/og-image.svg",
            get(share::share_social_image_handler),
        )
        .route("/share/{share_id}", get(share::share_page_handler))
        .route("/onboarding", get(spa::onboarding_handler))
        .route("/login", get(spa::login_handler_page))
        .route(
            "/assets/v/{version}/{*path}",
            get(assets::versioned_asset_handler),
        )
        .route("/assets/{*path}", get(assets::asset_handler))
        .route("/manifest.json", get(assets::manifest_handler))
        .route("/sw.js", get(assets::service_worker_handler))
        .merge(api)
        .fallback(spa::spa_fallback)
}

/// API routes served behind auth.
fn build_api_routes() -> Router<AppState> {
    let protected = Router::new()
        .route("/api/bootstrap", get(api::api_bootstrap_handler))
        .route("/api/gon", get(gon::api_gon_handler))
        .route("/api/skills", get(api::api_skills_handler))
        .route("/api/skills/search", get(api::api_skills_search_handler))
        .route("/api/mcp", get(api::api_mcp_handler))
        .route("/api/hooks", get(api::api_hooks_handler))
        .route(
            "/api/images/cached",
            get(api::api_cached_images_handler).delete(api::api_prune_cached_images_handler),
        )
        .route(
            "/api/images/cached/{tag}",
            axum::routing::delete(api::api_delete_cached_image_handler),
        )
        .route(
            "/api/images/build",
            axum::routing::post(api::api_build_image_handler),
        )
        .route(
            "/api/images/check-packages",
            axum::routing::post(api::api_check_packages_handler),
        )
        .route(
            "/api/images/default",
            get(api::api_get_default_image_handler).put(api::api_set_default_image_handler),
        )
        .route(
            "/api/sandbox/shared-home",
            get(api::api_get_shared_home_handler).put(api::api_set_shared_home_handler),
        )
        .route(
            "/api/sandbox/containers",
            get(api::api_list_containers_handler),
        )
        .route(
            "/api/sandbox/containers/clean",
            axum::routing::post(api::api_clean_all_containers_handler),
        )
        .route(
            "/api/sandbox/containers/{name}/stop",
            axum::routing::post(api::api_stop_container_handler),
        )
        .route(
            "/api/sandbox/containers/{name}",
            axum::routing::delete(api::api_remove_container_handler),
        )
        .route("/api/sandbox/disk-usage", get(api::api_disk_usage_handler))
        .route(
            "/api/sandbox/daemon/restart",
            axum::routing::post(api::api_restart_daemon_handler),
        )
        .route(
            "/api/terminal/windows",
            get(terminal::api_terminal_windows_handler)
                .post(terminal::api_terminal_windows_create_handler),
        )
        .route(
            "/api/terminal/ws",
            get(terminal::api_terminal_ws_upgrade_handler),
        )
        .route(
            "/api/env",
            get(moltis_gateway::env_routes::env_list).post(moltis_gateway::env_routes::env_set),
        )
        .route(
            "/api/env/{id}",
            axum::routing::delete(moltis_gateway::env_routes::env_delete),
        )
        .route(
            "/api/config",
            get(moltis_gateway::tools_routes::config_get)
                .post(moltis_gateway::tools_routes::config_save),
        )
        .route(
            "/api/config/validate",
            axum::routing::post(moltis_gateway::tools_routes::config_validate),
        )
        .route(
            "/api/config/template",
            get(moltis_gateway::tools_routes::config_template),
        )
        .route(
            "/api/restart",
            axum::routing::post(moltis_gateway::tools_routes::restart),
        )
        .route(
            "/api/sessions/{session_key}/upload",
            axum::routing::post(moltis_gateway::upload_routes::session_upload).layer(
                axum::extract::DefaultBodyLimit::max(
                    moltis_gateway::upload_routes::MAX_UPLOAD_SIZE,
                ),
            ),
        )
        .route(
            "/api/sessions/{session_key}/media/{filename}",
            get(api::api_session_media_handler),
        )
        .route("/api/logs/download", get(api::api_logs_download_handler));

    // Add metrics API routes (protected).
    #[cfg(feature = "metrics")]
    let protected = protected
        .route(
            "/api/metrics",
            get(moltis_gateway::metrics_routes::api_metrics_handler),
        )
        .route(
            "/api/metrics/summary",
            get(moltis_gateway::metrics_routes::api_metrics_summary_handler),
        )
        .route(
            "/api/metrics/history",
            get(moltis_gateway::metrics_routes::api_metrics_history_handler),
        );

    protected
}

/// Add feature-specific routes to API routes.
fn add_feature_routes(routes: Router<AppState>) -> Router<AppState> {
    #[cfg(feature = "tailscale")]
    let routes = routes.nest(
        "/api/tailscale",
        moltis_gateway::tailscale_routes::tailscale_router(),
    );

    #[cfg(feature = "push-notifications")]
    let routes = routes.nest("/api/push", moltis_gateway::push_routes::push_router());

    routes
}
