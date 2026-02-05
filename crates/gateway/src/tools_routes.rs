//! API routes for configuration editing.
//!
//! Provides endpoints to get, validate, and save the full moltis config as TOML.

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};

/// Get the current configuration as TOML.
pub async fn config_get(State(_state): State<crate::server::AppState>) -> impl IntoResponse {
    // Load the current config
    let config = moltis_config::discover_and_load();

    // Serialize the full config to TOML
    match toml::to_string_pretty(&config) {
        Ok(toml_str) => Json(serde_json::json!({
            "toml": toml_str,
            "valid": true,
            "path": moltis_config::find_or_default_config_path().to_string_lossy(),
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("failed to serialize config: {e}") })),
        )
            .into_response(),
    }
}

/// Validate configuration TOML without saving.
pub async fn config_validate(
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

    // Try to parse the TOML as MoltisConfig
    match toml::from_str::<moltis_config::MoltisConfig>(toml_str) {
        Ok(config) => {
            // Run validation checks
            let warnings = validate_config(&config);

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

/// Get the default configuration template with all options documented.
/// Preserves the current port from the existing config.
pub async fn config_template(State(_state): State<crate::server::AppState>) -> impl IntoResponse {
    // Load current config to preserve the port
    let config = moltis_config::discover_and_load();
    let template = moltis_config::template::default_config_template(config.server.port);

    Json(serde_json::json!({
        "toml": template,
    }))
}

/// Save configuration from TOML.
pub async fn config_save(
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
    let config: moltis_config::MoltisConfig = match toml::from_str(toml_str) {
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

    match moltis_config::save_config(&config) {
        Ok(path) => {
            tracing::info!(path = %path.display(), "saved config");
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

/// Validate config and return warnings.
fn validate_config(config: &moltis_config::MoltisConfig) -> Vec<String> {
    let mut warnings = Vec::new();

    // Check browser config
    if config.tools.browser.enabled {
        if config.tools.browser.sandbox {
            warnings.push(
                "Browser sandbox mode is enabled but not yet implemented. \
                 Browser will run on host."
                    .to_string(),
            );
        }

        if config.tools.browser.allowed_domains.is_empty() {
            warnings.push(
                "No allowed_domains set for browser. All domains are accessible. \
                 Consider restricting to trusted domains for security."
                    .to_string(),
            );
        }

        if config.tools.browser.max_instances > 10 {
            warnings.push(format!(
                "max_instances={} is high. Consider reducing to prevent resource exhaustion.",
                config.tools.browser.max_instances
            ));
        }
    }

    // Check exec config
    if config.tools.exec.sandbox.mode == "off" {
        warnings.push(
            "Sandbox mode is off. Commands will run directly on host without isolation."
                .to_string(),
        );
    }

    // Check auth config
    if config.auth.disabled {
        warnings.push(
            "Authentication is disabled. Anyone with network access can use the gateway."
                .to_string(),
        );
    }

    // Check TLS config
    if !config.tls.enabled {
        warnings.push("TLS is disabled. Connections will use unencrypted HTTP.".to_string());
    }

    // Check heartbeat active hours
    if config.heartbeat.enabled
        && config.heartbeat.active_hours.start == config.heartbeat.active_hours.end
    {
        warnings.push(
            "Heartbeat active_hours start and end are the same. Heartbeat may not run.".to_string(),
        );
    }

    warnings
}
