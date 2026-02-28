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

const ENV_STORE_UNAVAILABLE: &str = "ENV_STORE_UNAVAILABLE";
const ENV_KEY_REQUIRED: &str = "ENV_KEY_REQUIRED";
const ENV_KEY_INVALID: &str = "ENV_KEY_INVALID";
const ENV_LIST_FAILED: &str = "ENV_LIST_FAILED";
const ENV_SET_FAILED: &str = "ENV_SET_FAILED";
const ENV_DELETE_FAILED: &str = "ENV_DELETE_FAILED";

/// Successful mutation response (`{"ok": true}`).
#[derive(Serialize)]
pub struct OkResponse {
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
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    fn service_unavailable(code: &'static str, msg: &str) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code,
            message: msg.into(),
        }
    }

    fn bad_request(code: &'static str, msg: &str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            message: msg.into(),
        }
    }

    fn internal(code: &'static str, err: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code,
            message: err.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        #[derive(Serialize)]
        struct Body {
            code: &'static str,
            error: String,
        }
        (
            self.status,
            Json(Body {
                code: self.code,
                error: self.message,
            }),
        )
            .into_response()
    }
}

/// Env var listing response (`{"env_vars": [...]}`).
#[derive(Serialize)]
pub struct EnvListResponse {
    env_vars: Vec<EnvVarEntry>,
}

impl IntoResponse for EnvListResponse {
    fn into_response(self) -> Response {
        Json(self).into_response()
    }
}

// ── Route handlers ───────────────────────────────────────────────────────────

/// List all environment variables (names only, no values).
pub async fn env_list(
    State(state): State<crate::server::AppState>,
) -> Result<EnvListResponse, ApiError> {
    let store = state.gateway.credential_store.as_ref().ok_or_else(|| {
        ApiError::service_unavailable(ENV_STORE_UNAVAILABLE, "no credential store")
    })?;

    let env_vars = store
        .list_env_vars()
        .await
        .map_err(|err| ApiError::internal(ENV_LIST_FAILED, err))?;
    Ok(EnvListResponse { env_vars })
}

/// Set (upsert) an environment variable.
pub async fn env_set(
    State(state): State<crate::server::AppState>,
    Json(body): Json<serde_json::Value>,
) -> Result<OkResponse, ApiError> {
    let store = state.gateway.credential_store.as_ref().ok_or_else(|| {
        ApiError::service_unavailable(ENV_STORE_UNAVAILABLE, "no credential store")
    })?;

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
        return Err(ApiError::bad_request(ENV_KEY_REQUIRED, "key is required"));
    }

    // Validate key format: letters, digits, underscores.
    if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(ApiError::bad_request(
            ENV_KEY_INVALID,
            "key must contain only letters, digits, and underscores",
        ));
    }

    store
        .set_env_var(key, &value)
        .await
        .map_err(|err| ApiError::internal(ENV_SET_FAILED, err))?;

    Ok(OkResponse::success())
}

/// Delete an environment variable by id.
pub async fn env_delete(
    State(state): State<crate::server::AppState>,
    Path(id): Path<i64>,
) -> Result<OkResponse, ApiError> {
    let store = state.gateway.credential_store.as_ref().ok_or_else(|| {
        ApiError::service_unavailable(ENV_STORE_UNAVAILABLE, "no credential store")
    })?;

    let _ = store
        .delete_env_var(id)
        .await
        .map_err(|err| ApiError::internal(ENV_DELETE_FAILED, err))?;

    Ok(OkResponse::success())
}

#[cfg(test)]
mod tests {
    use {super::*, axum::body::to_bytes};

    async fn response_json(response: Response) -> serde_json::Value {
        let body = match to_bytes(response.into_body(), usize::MAX).await {
            Ok(bytes) => bytes,
            Err(err) => panic!("failed to read body bytes: {err}"),
        };
        match serde_json::from_slice::<serde_json::Value>(&body) {
            Ok(value) => value,
            Err(err) => panic!("failed to parse json body: {err}"),
        }
    }

    #[tokio::test]
    async fn api_error_includes_code_and_message() {
        let response = ApiError::bad_request(ENV_KEY_REQUIRED, "key is required").into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = response_json(response).await;
        assert_eq!(json["code"], ENV_KEY_REQUIRED);
        assert_eq!(json["error"], "key is required");
    }

    #[tokio::test]
    async fn internal_error_uses_provided_code() {
        let response = ApiError::internal(ENV_SET_FAILED, "boom").into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let json = response_json(response).await;
        assert_eq!(json["code"], ENV_SET_FAILED);
        assert_eq!(json["error"], "boom");
    }
}
