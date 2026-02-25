//! OAuth callback handler.

use std::collections::HashMap;

use {
    axum::{
        extract::{Query, State},
        http::StatusCode,
        response::{Html, IntoResponse},
    },
    moltis_gateway::server::AppState,
};

pub async fn oauth_callback_handler(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let Some(code) = params.get("code") else {
        return (
            StatusCode::BAD_REQUEST,
            Html("<h1>Authentication failed</h1><p>Missing authorization code.</p>".to_string()),
        )
            .into_response();
    };
    let Some(oauth_state) = params.get("state") else {
        return (
            StatusCode::BAD_REQUEST,
            Html("<h1>Authentication failed</h1><p>Missing OAuth state.</p>".to_string()),
        )
            .into_response();
    };

    let completion_params = serde_json::json!({
        "code": code,
        "state": oauth_state,
    });

    let completion = match state
        .gateway
        .services
        .provider_setup
        .oauth_complete(completion_params.clone())
        .await
    {
        Ok(result) => Ok(result),
        Err(provider_error) => state
            .gateway
            .services
            .mcp
            .oauth_complete(completion_params)
            .await
            .map_err(|mcp_error| (provider_error, mcp_error)),
    };

    match completion {
        Ok(_) => {
            let nonce = uuid::Uuid::new_v4().to_string();
            let html = format!(
                "<h1>Authentication successful!</h1><p>You can close this window.</p>\
                 <script nonce=\"{nonce}\">window.close();</script>"
            );
            let csp = format!(
                "default-src 'none'; script-src 'nonce-{nonce}'; style-src 'unsafe-inline'"
            );
            let mut resp = Html(html).into_response();
            if let Ok(val) = csp.parse() {
                resp.headers_mut()
                    .insert(axum::http::header::CONTENT_SECURITY_POLICY, val);
            }
            resp
        },
        Err((provider_error, mcp_error)) => {
            tracing::warn!(
                provider_error = %provider_error,
                mcp_error = %mcp_error,
                "OAuth callback completion failed"
            );
            (
                StatusCode::BAD_REQUEST,
                Html(
                    "<h1>Authentication failed</h1><p>Could not complete OAuth flow.</p>"
                        .to_string(),
                ),
            )
                .into_response()
        },
    }
}
