use {
    axum::{
        Json,
        extract::{Path, State},
        http::StatusCode,
        response::{IntoResponse, Response},
    },
    serde::Serialize,
};

use crate::auth::EnvVarEntry;

// ── Typed responses ──────────────────────────────────────────────────────────

/// Successful mutation response (`{"ok": true}`).
#[derive(Serialize)]
pub(crate) struct OkResponse {
    ok: bool,
}

impl OkResponse {
    const fn success() -> Self {
        Self { ok: true }
    }
}

impl IntoResponse for OkResponse {
    fn into_response(self) -> Response {
        Json(self).into_response()
    }
}

/// JSON error with an HTTP status code.
pub(crate) struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn service_unavailable(msg: &str) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: msg.into(),
        }
    }

    fn bad_request(msg: &str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: msg.into(),
        }
    }

    fn internal(err: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: err.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        #[derive(Serialize)]
        struct Body {
            error: String,
        }
        (
            self.status,
            Json(Body {
                error: self.message,
            }),
        )
            .into_response()
    }
}

/// Env var listing response (`{"env_vars": [...]}`).
#[derive(Serialize)]
pub(crate) struct EnvListResponse {
    env_vars: Vec<EnvVarEntry>,
}

impl IntoResponse for EnvListResponse {
    fn into_response(self) -> Response {
        Json(self).into_response()
    }
}

// ── Route handlers ───────────────────────────────────────────────────────────

/// List all environment variables (names only, no values).
pub(crate) async fn env_list(
    State(state): State<crate::server::AppState>,
) -> Result<EnvListResponse, ApiError> {
    let store = state
        .gateway
        .credential_store
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("no credential store"))?;

    let env_vars = store.list_env_vars().await.map_err(ApiError::internal)?;
    Ok(EnvListResponse { env_vars })
}

/// Set (upsert) an environment variable.
pub(crate) async fn env_set(
    State(state): State<crate::server::AppState>,
    Json(body): Json<serde_json::Value>,
) -> Result<OkResponse, ApiError> {
    let store = state
        .gateway
        .credential_store
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("no credential store"))?;

    let key = body
        .get("key")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let value = body
        .get("value")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if key.is_empty() {
        return Err(ApiError::bad_request("key is required"));
    }

    // Validate key format: letters, digits, underscores.
    if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(ApiError::bad_request(
            "key must contain only letters, digits, and underscores",
        ));
    }

    store
        .set_env_var(key, &value)
        .await
        .map_err(ApiError::internal)?;

    Ok(OkResponse::success())
}

/// Delete an environment variable by id.
pub(crate) async fn env_delete(
    State(state): State<crate::server::AppState>,
    Path(id): Path<i64>,
) -> Result<OkResponse, ApiError> {
    let store = state
        .gateway
        .credential_store
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("no credential store"))?;

    let _ = store.delete_env_var(id).await.map_err(ApiError::internal)?;

    Ok(OkResponse::success())
}
