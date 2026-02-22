//! Share page and social image handlers.

use {
    axum::{
        extract::{Path, Query, State},
        http::StatusCode,
        response::{Html, IntoResponse},
    },
    axum_extra::extract::{
        CookieJar,
        cookie::{Cookie, SameSite},
    },
    moltis_gateway::server::AppState,
    tracing::warn,
};

use crate::templates::ShareAccessQuery;

fn not_found_share_response() -> axum::response::Response {
    (StatusCode::NOT_FOUND, "share not found").into_response()
}

fn share_cookie_name(share_id: &str) -> String {
    format!("moltis_share_{}", share_id)
}

fn request_origin(headers: &axum::http::HeaderMap, tls_active: bool) -> Option<String> {
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let forwarded_proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| *value == "http" || *value == "https");
    let scheme = forwarded_proto.unwrap_or(if tls_active {
        "https"
    } else {
        "http"
    });
    Some(format!("{scheme}://{host}"))
}

fn share_social_image_url(
    headers: &axum::http::HeaderMap,
    tls_active: bool,
    share_id: &str,
) -> String {
    let path = format!("/share/{share_id}/og-image.svg");
    match request_origin(headers, tls_active) {
        Some(origin) => format!("{origin}{path}"),
        None => path,
    }
}

pub async fn share_page_handler(
    Path(share_id): Path<String>,
    Query(query): Query<ShareAccessQuery>,
    jar: CookieJar,
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    if uuid::Uuid::parse_str(&share_id).is_err() {
        return not_found_share_response();
    }

    let static_path = moltis_config::data_dir()
        .join("shares")
        .join(format!("{share_id}.html"));
    if let Ok(html) = tokio::fs::read_to_string(&static_path).await {
        return serve_static_share_html(html);
    }

    let Some(ref share_store) = state.gateway.services.session_share_store else {
        return not_found_share_response();
    };

    let share = match share_store.get_active_by_id(&share_id).await {
        Ok(Some(share)) => share,
        Ok(None) => return not_found_share_response(),
        Err(e) => {
            warn!(share_id, error = %e, "failed to load shared session");
            return not_found_share_response();
        },
    };

    let cookie_name = share_cookie_name(&share.id);
    let cookie_access_granted = jar.get(&cookie_name).is_some_and(|cookie| {
        moltis_gateway::share_store::ShareStore::verify_access_key(&share, cookie.value())
    });
    let query_access_granted = query
        .k
        .as_deref()
        .is_some_and(|key| moltis_gateway::share_store::ShareStore::verify_access_key(&share, key));

    if share.visibility == moltis_gateway::share_store::ShareVisibility::Private
        && !(cookie_access_granted || query_access_granted)
    {
        return not_found_share_response();
    }

    if share.visibility == moltis_gateway::share_store::ShareVisibility::Private
        && query_access_granted
        && !cookie_access_granted
    {
        let Some(access_key) = query.k else {
            return not_found_share_response();
        };
        let mut cookie = Cookie::new(cookie_name, access_key);
        cookie.set_http_only(true);
        cookie.set_same_site(Some(SameSite::Lax));
        cookie.set_path(format!("/share/{}", share.id));
        cookie.set_secure(state.gateway.tls_active);
        return (
            jar.add(cookie),
            axum::response::Redirect::to(&format!("/share/{}", share.id)),
        )
            .into_response();
    }

    let view_count = share_store
        .increment_views(&share.id)
        .await
        .unwrap_or(share.views);

    let snapshot: moltis_gateway::share_store::ShareSnapshot =
        match serde_json::from_str(&share.snapshot_json) {
            Ok(snapshot) => snapshot,
            Err(e) => {
                warn!(share_id, error = %e, "failed to parse session share snapshot");
                return not_found_share_response();
            },
        };

    let identity = state
        .gateway
        .services
        .onboarding
        .identity_get()
        .await
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let share_image_url = share_social_image_url(&headers, state.gateway.tls_active, &share.id);

    let body = match moltis_gateway::share_render::render_share_html(
        &snapshot,
        &identity,
        &share_id,
        share.visibility,
        view_count,
        &share_image_url,
    ) {
        Ok(html) => html,
        Err(e) => {
            warn!(share_id, error = %e, "failed to render share template");
            return (StatusCode::INTERNAL_SERVER_ERROR, "failed to render share").into_response();
        },
    };

    serve_static_share_html(body)
}

fn serve_static_share_html(body: String) -> axum::response::Response {
    let mut response = Html(body).into_response();
    let headers = response.headers_mut();
    if let Ok(value) = "no-store".parse() {
        headers.insert(axum::http::header::CACHE_CONTROL, value);
    }
    if let Ok(value) = "no-referrer".parse() {
        headers.insert(axum::http::header::REFERRER_POLICY, value);
    }
    if let Ok(value) = "noindex, nofollow, noarchive".parse() {
        headers.insert(
            axum::http::header::HeaderName::from_static("x-robots-tag"),
            value,
        );
    }
    let csp = "default-src 'none'; \
               script-src 'self'; \
               style-src 'unsafe-inline'; \
               img-src 'self' data: https://www.moltis.org; \
               media-src 'self' data:; \
               connect-src 'self' data:; \
               base-uri 'none'; \
               frame-ancestors 'none'; \
               form-action 'none'; \
               object-src 'none'";
    if let Ok(value) = csp.parse() {
        headers.insert(axum::http::header::CONTENT_SECURITY_POLICY, value);
    }
    response
}

pub async fn share_social_image_handler(
    Path(share_id): Path<String>,
    Query(query): Query<ShareAccessQuery>,
    jar: CookieJar,
    State(state): State<AppState>,
) -> axum::response::Response {
    if uuid::Uuid::parse_str(&share_id).is_err() {
        return not_found_share_response();
    }

    let static_path = moltis_config::data_dir()
        .join("shares")
        .join(format!("{share_id}-og.svg"));
    if let Ok(svg) = tokio::fs::read_to_string(&static_path).await {
        return serve_static_share_svg(svg);
    }

    let Some(ref share_store) = state.gateway.services.session_share_store else {
        return not_found_share_response();
    };

    let share = match share_store.get_active_by_id(&share_id).await {
        Ok(Some(share)) => share,
        Ok(None) => return not_found_share_response(),
        Err(e) => {
            warn!(share_id, error = %e, "failed to load shared session for social image");
            return not_found_share_response();
        },
    };

    let cookie_name = share_cookie_name(&share.id);
    let cookie_access_granted = jar.get(&cookie_name).is_some_and(|cookie| {
        moltis_gateway::share_store::ShareStore::verify_access_key(&share, cookie.value())
    });
    let query_access_granted = query
        .k
        .as_deref()
        .is_some_and(|key| moltis_gateway::share_store::ShareStore::verify_access_key(&share, key));

    if share.visibility == moltis_gateway::share_store::ShareVisibility::Private
        && !(cookie_access_granted || query_access_granted)
    {
        return not_found_share_response();
    }

    let snapshot: moltis_gateway::share_store::ShareSnapshot =
        match serde_json::from_str(&share.snapshot_json) {
            Ok(snapshot) => snapshot,
            Err(e) => {
                warn!(
                    share_id,
                    error = %e,
                    "failed to parse shared session snapshot for social image"
                );
                return not_found_share_response();
            },
        };

    let identity = state
        .gateway
        .services
        .onboarding
        .identity_get()
        .await
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let svg = moltis_gateway::share_render::render_share_og_svg(&snapshot, &identity);
    serve_static_share_svg(svg)
}

fn serve_static_share_svg(svg: String) -> axum::response::Response {
    let mut response = (StatusCode::OK, svg).into_response();
    let headers = response.headers_mut();
    if let Ok(value) = "image/svg+xml".parse() {
        headers.insert(axum::http::header::CONTENT_TYPE, value);
    }
    if let Ok(value) = "no-cache".parse() {
        headers.insert(axum::http::header::CACHE_CONTROL, value);
    }
    if let Ok(value) = "nosniff".parse() {
        headers.insert(
            axum::http::header::HeaderName::from_static("x-content-type-options"),
            value,
        );
    }
    if let Ok(value) = "default-src 'none'; img-src 'self' data:; style-src 'none'; script-src 'none'; object-src 'none'; frame-ancestors 'none'".parse() {
        headers.insert(axum::http::header::CONTENT_SECURITY_POLICY, value);
    }
    response
}
