//! API routes for tools configuration.
//!
//! Provides endpoints to get, validate, and save tools config as TOML.

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};

/// Get the current tools configuration as TOML.
pub async fn tools_config_get(State(_state): State<crate::server::AppState>) -> impl IntoResponse {
    // Load the current config
    let config = moltis_config::discover_and_load();

    // Serialize just the tools section to TOML
    match toml::to_string_pretty(&config.tools) {
        Ok(toml_str) => Json(serde_json::json!({
            "toml": toml_str,
            "valid": true,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("failed to serialize config: {e}") })),
        )
            .into_response(),
    }
}

/// Validate tools configuration TOML without saving.
pub async fn tools_config_validate(
    State(_state): State<crate::server::AppState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let Some(toml_str) = body.get("toml").and_then(|v| v.as_str()) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "missing 'toml' field" })),
        )
            .into_response();
    };

    // Try to parse the TOML as ToolsConfig
    match toml::from_str::<moltis_config::schema::ToolsConfig>(toml_str) {
        Ok(tools_config) => {
            // Additional validation can be added here
            let warnings = validate_tools_config(&tools_config);

            Json(serde_json::json!({
                "valid": true,
                "warnings": warnings,
            }))
            .into_response()
        },
        Err(e) => {
            // Parse error message to extract line/column if available
            let error_msg = e.to_string();
            Json(serde_json::json!({
                "valid": false,
                "error": error_msg,
            }))
            .into_response()
        },
    }
}

/// Save tools configuration from TOML.
pub async fn tools_config_save(
    State(_state): State<crate::server::AppState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let Some(toml_str) = body.get("toml").and_then(|v| v.as_str()) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "missing 'toml' field" })),
        )
            .into_response();
    };

    // Parse the TOML
    let tools_config: moltis_config::schema::ToolsConfig = match toml::from_str(toml_str) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!("invalid TOML: {e}"),
                    "valid": false,
                })),
            )
                .into_response();
        },
    };

    // Load current config, update tools section, and save
    let mut config = moltis_config::discover_and_load();
    config.tools = tools_config;

    match moltis_config::save_config(&config) {
        Ok(path) => {
            tracing::info!(path = %path.display(), "saved tools config");
            Json(serde_json::json!({
                "ok": true,
                "path": path.to_string_lossy(),
                "restart_required": true,
            }))
            .into_response()
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("failed to save config: {e}") })),
        )
            .into_response(),
    }
}

/// Validate tools config and return warnings.
fn validate_tools_config(config: &moltis_config::schema::ToolsConfig) -> Vec<String> {
    let mut warnings = Vec::new();

    // Check browser config
    if config.browser.enabled {
        if config.browser.sandbox {
            warnings.push(
                "Browser sandbox mode is enabled but not yet implemented. \
                 Browser will run on host."
                    .to_string(),
            );
        }

        if config.browser.allowed_domains.is_empty() {
            warnings.push(
                "No allowed_domains set for browser. All domains are accessible. \
                 Consider restricting to trusted domains for security."
                    .to_string(),
            );
        }

        if config.browser.max_instances > 10 {
            warnings.push(format!(
                "max_instances={} is high. Consider reducing to prevent resource exhaustion.",
                config.browser.max_instances
            ));
        }
    }

    // Check exec config
    if config.exec.sandbox.mode == "off" {
        warnings.push(
            "Sandbox mode is off. Commands will run directly on host without isolation."
                .to_string(),
        );
    }

    warnings
}
