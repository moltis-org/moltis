use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};

const ENV_CREDENTIAL_STORE_UNAVAILABLE: &str = "ENV_CREDENTIAL_STORE_UNAVAILABLE";
const ENV_KEY_REQUIRED: &str = "ENV_KEY_REQUIRED";
const ENV_KEY_INVALID: &str = "ENV_KEY_INVALID";
const ENV_LIST_FAILED: &str = "ENV_LIST_FAILED";
const ENV_SET_FAILED: &str = "ENV_SET_FAILED";
const ENV_DELETE_FAILED: &str = "ENV_DELETE_FAILED";

fn error_body(code: &str, error: impl Into<String>) -> serde_json::Value {
    serde_json::json!({ "code": code, "error": error.into() })
}

fn is_valid_env_key(key: &str) -> bool {
    key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// List all environment variables (names only, no values).
pub async fn env_list(State(state): State<crate::server::AppState>) -> impl IntoResponse {
    let Some(ref store) = state.gateway.credential_store else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(error_body(
                ENV_CREDENTIAL_STORE_UNAVAILABLE,
                "no credential store",
            )),
        )
            .into_response();
    };
    match store.list_env_vars().await {
        Ok(vars) => Json(serde_json::json!({ "env_vars": vars })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(error_body(ENV_LIST_FAILED, e.to_string())),
        )
            .into_response(),
    }
}

/// Set (upsert) an environment variable.
pub async fn env_set(
    State(state): State<crate::server::AppState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let Some(ref store) = state.gateway.credential_store else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(error_body(
                ENV_CREDENTIAL_STORE_UNAVAILABLE,
                "no credential store",
            )),
        )
            .into_response();
    };

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
        return (
            StatusCode::BAD_REQUEST,
            Json(error_body(ENV_KEY_REQUIRED, "key is required")),
        )
            .into_response();
    }

    // Validate key format: letters, digits, underscores.
    if !is_valid_env_key(key) {
        return (
            StatusCode::BAD_REQUEST,
            Json(error_body(
                ENV_KEY_INVALID,
                "key must contain only letters, digits, and underscores",
            )),
        )
            .into_response();
    }

    match store.set_env_var(key, &value).await {
        Ok(_) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(error_body(ENV_SET_FAILED, e.to_string())),
        )
            .into_response(),
    }
}

/// Delete an environment variable by id.
pub async fn env_delete(
    State(state): State<crate::server::AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let Some(ref store) = state.gateway.credential_store else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(error_body(
                ENV_CREDENTIAL_STORE_UNAVAILABLE,
                "no credential store",
            )),
        )
            .into_response();
    };
    match store.delete_env_var(id).await {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(error_body(ENV_DELETE_FAILED, e.to_string())),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_body_contains_code_and_error() {
        let value = error_body("ENV_KEY_REQUIRED", "key is required");
        assert_eq!(value["code"], "ENV_KEY_REQUIRED");
        assert_eq!(value["error"], "key is required");
    }

    #[test]
    fn env_key_validation_accepts_letters_digits_and_underscore() {
        assert!(is_valid_env_key("ABC_123"));
        assert!(is_valid_env_key("my_key"));
        assert!(!is_valid_env_key("bad-key"));
        assert!(!is_valid_env_key("bad key"));
        assert!(!is_valid_env_key("bad.key"));
    }
}
