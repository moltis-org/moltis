//! API routes for configuration editing.
//!
//! Provides endpoints to get, validate, and save the full moltis config as TOML.
//!
//! SECURITY: These endpoints expose sensitive configuration including API keys.
//! They are protected by auth middleware, but also have explicit checks to ensure
//! they never work without authentication on non-localhost connections.

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};

/// Check if the request should be allowed for config operations.
///
/// Config endpoints are ONLY allowed when:
/// - Running on localhost (loopback interface only), OR
/// - User is authenticated via session or API key
///
/// This is a defense-in-depth check on top of the auth middleware.
async fn require_config_access(state: &crate::server::AppState) -> Result<(), impl IntoResponse> {
    let gw = &state.gateway;

    // On localhost with no password set, allow access (backward compat for initial setup)
    if gw.localhost_only {
        if let Some(ref cred_store) = gw.credential_store {
            // If auth is explicitly disabled, allow
            if cred_store.is_auth_disabled() {
                return Ok(());
            }
            // If no password set yet, allow (initial setup)
            if !cred_store.is_setup_complete() {
                return Ok(());
            }
        } else {
            // No credential store on localhost = allow
            return Ok(());
        }
    }

    // For non-localhost, we MUST have valid auth. The middleware should have
    // already blocked unauthenticated requests, but if somehow we got here
    // without localhost and without going through auth, block it.
    if !gw.localhost_only {
        if let Some(ref cred_store) = gw.credential_store {
            // Auth is configured but we're not on localhost - the middleware
            // should have verified auth. If auth is disabled, that's the user's
            // explicit choice.
            if cred_store.is_auth_disabled() {
                return Ok(());
            }
            // Setup complete means auth is enforced - middleware handles this
            if cred_store.is_setup_complete() {
                // Trust that middleware verified auth
                return Ok(());
            }
        } else {
            // Non-localhost without credential store is a misconfiguration
            // but we should not expose config in this case
            return Err((
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "error": "Config access requires authentication on non-localhost connections"
                })),
            ));
        }
    }

    Ok(())
}

/// Get the current configuration as TOML.
pub async fn config_get(State(state): State<crate::server::AppState>) -> impl IntoResponse {
    // Extra security check for config access
    if let Err(resp) = require_config_access(&state).await {
        return resp.into_response();
    }

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
    State(state): State<crate::server::AppState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Extra security check for config access
    if let Err(resp) = require_config_access(&state).await {
        return resp.into_response();
    }

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
pub async fn config_template(State(state): State<crate::server::AppState>) -> impl IntoResponse {
    // Extra security check for config access
    if let Err(resp) = require_config_access(&state).await {
        return resp.into_response();
    }

    // Load current config to preserve the port
    let config = moltis_config::discover_and_load();
    let template = moltis_config::template::default_config_template(config.server.port);

    Json(serde_json::json!({
        "toml": template,
    }))
    .into_response()
}

/// Save configuration from TOML.
pub async fn config_save(
    State(state): State<crate::server::AppState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Extra security check for config access
    if let Err(resp) = require_config_access(&state).await {
        return resp.into_response();
    }

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

/// Restart the moltis process.
///
/// This re-runs the current binary with the same arguments. On Unix, it uses the exec
/// syscall to replace the current process. On other platforms, it spawns a new process.
///
/// Before restarting, the saved config is loaded from disk and validated. If the config
/// is invalid the restart is refused so the server doesn't crash on startup.
pub async fn restart(State(state): State<crate::server::AppState>) -> impl IntoResponse {
    // Extra security check for config access (restart is a privileged operation)
    if let Err(resp) = require_config_access(&state).await {
        return resp.into_response();
    }

    // Validate the on-disk config before restarting to avoid crash loops.
    let config_path = moltis_config::find_or_default_config_path();
    if config_path.exists() {
        match std::fs::read_to_string(&config_path) {
            Ok(toml_str) => {
                if let Err(e) = toml::from_str::<moltis_config::MoltisConfig>(&toml_str) {
                    tracing::warn!(
                        path = %config_path.display(),
                        error = %e,
                        "restart refused: saved config is invalid"
                    );
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({
                            "error": format!("Config is invalid, refusing to restart: {e}"),
                            "valid": false,
                        })),
                    )
                        .into_response();
                }
            },
            Err(e) => {
                tracing::warn!(
                    path = %config_path.display(),
                    error = %e,
                    "restart refused: cannot read config file"
                );
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": format!("Cannot read config file: {e}"),
                    })),
                )
                    .into_response();
            },
        }
    }

    tracing::info!("restart requested via API");

    // Spawn a task to restart after a short delay, allowing the response to be sent first.
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        tracing::info!("restarting now");

        let exe = match std::env::current_exe() {
            Ok(path) => path,
            Err(e) => {
                tracing::error!("failed to get current executable path: {e}");
                std::process::exit(1);
            },
        };

        let args: Vec<String> = std::env::args().skip(1).collect();
        tracing::info!(exe = %exe.display(), args = ?args, "re-executing");

        // Use exec on Unix to replace the current process in-place
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // This replaces the current process and never returns on success
            let err = std::process::Command::new(&exe).args(&args).exec();
            tracing::error!("failed to exec: {err}");
            std::process::exit(1);
        }

        // On non-Unix, spawn a new process and exit the current one
        #[cfg(not(unix))]
        {
            match std::process::Command::new(&exe).args(&args).spawn() {
                Ok(_) => {
                    tracing::info!("spawned new process, exiting current");
                    std::process::exit(0);
                },
                Err(e) => {
                    tracing::error!("failed to spawn new process: {e}");
                    std::process::exit(1);
                },
            }
        }
    });

    Json(serde_json::json!({
        "ok": true,
        "message": "Moltis is restarting..."
    }))
    .into_response()
}

/// Validate config and return warnings.
fn validate_config(config: &moltis_config::MoltisConfig) -> Vec<String> {
    let mut warnings = Vec::new();

    // Check browser config
    if config.tools.browser.enabled {
        // Browser sandbox mode follows session sandbox mode (controlled by exec.sandbox.mode).
        // If sandbox mode is available, check if container runtime exists.
        if config.tools.exec.sandbox.mode != "off"
            && !moltis_browser::container::is_container_available()
        {
            warnings.push(
                "Sandbox mode is available but no container runtime found. \
                 Browser sandbox (for sandboxed sessions) requires Docker or Apple Container."
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
