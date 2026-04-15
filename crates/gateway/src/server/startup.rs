use std::{path::PathBuf, sync::Arc};

use tracing::{debug, info, warn};

use crate::{auth_webauthn::SharedWebAuthnRegistry, state::GatewayState};

// ── OpenClaw detection / import ──────────────────────────────────────────────

#[cfg(feature = "openclaw-import")]
fn detect_openclaw_with_startup_logs() -> Option<moltis_openclaw_import::OpenClawDetection> {
    match moltis_openclaw_import::detect() {
        Some(detection) => {
            info!(
                openclaw_home = %detection.home_dir.display(),
                openclaw_workspace = %detection.workspace_dir.display(),
                has_config = detection.has_config,
                has_credentials = detection.has_credentials,
                has_memory = detection.has_memory,
                has_skills = detection.has_skills,
                has_mcp_servers = detection.has_mcp_servers,
                sessions = detection.session_count,
                agents = detection.agent_ids.len(),
                agent_ids = ?detection.agent_ids,
                unsupported_channels = ?detection.unsupported_channels,
                "startup OpenClaw installation detected"
            );
            Some(detection)
        },
        None => {
            info!(
                openclaw_home_env = %super::helpers::env_var_or_unset("OPENCLAW_HOME"),
                openclaw_profile_env = %super::helpers::env_var_or_unset("OPENCLAW_PROFILE"),
                "startup OpenClaw installation not detected (checked OPENCLAW_HOME and ~/.openclaw)"
            );
            None
        },
    }
}

#[cfg(feature = "openclaw-import")]
pub(crate) fn deferred_openclaw_status() -> String {
    "background detection pending".to_string()
}

#[cfg(not(feature = "openclaw-import"))]
pub(crate) fn deferred_openclaw_status() -> String {
    "feature disabled".to_string()
}

#[cfg(feature = "openclaw-import")]
#[cfg_attr(not(feature = "file-watcher"), allow(unused_variables))]
fn spawn_openclaw_background_init(data_dir: PathBuf) {
    tokio::spawn(async move {
        #[cfg_attr(not(feature = "file-watcher"), allow(unused_variables))]
        let detection = match tokio::task::spawn_blocking(detect_openclaw_with_startup_logs).await {
            Ok(detection) => detection,
            Err(error) => {
                warn!(
                    error = %error,
                    "startup OpenClaw background detection worker failed"
                );
                return;
            },
        };

        #[cfg(feature = "file-watcher")]
        if let Some(detection) = detection {
            let import_agent = if detection.agent_ids.contains(&"main".to_string()) {
                "main"
            } else {
                detection
                    .agent_ids
                    .first()
                    .map(|s| s.as_str())
                    .unwrap_or("main")
            };
            let sessions_dir = detection
                .home_dir
                .join("agents")
                .join(import_agent)
                .join("agent")
                .join("sessions");
            if sessions_dir.is_dir() {
                match moltis_openclaw_import::watcher::ImportWatcher::start(sessions_dir) {
                    Ok((_watcher, mut rx)) => {
                        info!("openclaw: session watcher started");
                        let watcher_data_dir = data_dir;
                        tokio::spawn(async move {
                            let _watcher = _watcher; // keep alive
                            let mut interval =
                                tokio::time::interval(std::time::Duration::from_secs(60));
                            interval.tick().await; // skip first immediate tick
                            loop {
                                tokio::select! {
                                    Some(_event) = rx.recv() => {
                                        debug!("openclaw: session change detected, running incremental import");
                                        let report = moltis_openclaw_import::import_sessions_only(
                                            &detection, &watcher_data_dir,
                                        );
                                        if report.items_imported > 0 || report.items_updated > 0 {
                                            info!(
                                                imported = report.items_imported,
                                                updated = report.items_updated,
                                                skipped = report.items_skipped,
                                                "openclaw: incremental session sync complete"
                                            );
                                        }
                                    }
                                    _ = interval.tick() => {
                                        debug!("openclaw: periodic session sync");
                                        let report = moltis_openclaw_import::import_sessions_only(
                                            &detection, &watcher_data_dir,
                                        );
                                        if report.items_imported > 0 || report.items_updated > 0 {
                                            info!(
                                                imported = report.items_imported,
                                                updated = report.items_updated,
                                                skipped = report.items_skipped,
                                                "openclaw: periodic session sync complete"
                                            );
                                        }
                                    }
                                }
                            }
                        });
                    },
                    Err(error) => {
                        warn!("openclaw: failed to start session watcher: {error}");
                    },
                }
            }
        }
    });
}

#[cfg(not(feature = "openclaw-import"))]
fn spawn_openclaw_background_init(_data_dir: PathBuf) {}

/// Launch OpenClaw detection/import background tasks without blocking startup.
pub fn start_openclaw_background_tasks(data_dir: PathBuf) {
    spawn_openclaw_background_init(data_dir);
}

// ── Browser warmup ───────────────────────────────────────────────────────────

fn spawn_post_listener_warmups(
    browser_service: Arc<dyn crate::services::BrowserService>,
    browser_tool: Option<Arc<dyn moltis_agents::tool_registry::AgentTool>>,
) {
    // Warm the container CLI OnceLock off the async worker threads.
    tokio::task::spawn_blocking(|| {
        let cli = moltis_tools::sandbox::container_cli();
        debug!(cli, "container CLI detected");
    });

    if !super::helpers::env_flag_enabled("MOLTIS_BROWSER_WARMUP") {
        debug!("startup browser warmup disabled (set MOLTIS_BROWSER_WARMUP=1 to enable)");
        return;
    }

    tokio::spawn(async move {
        browser_service.warmup().await;
        if let Some(tool) = browser_tool
            && let Err(error) = tool.warmup().await
        {
            warn!(%error, "browser tool warmup failed");
        }
    });
}

/// Start browser warmup after the transport listener is ready.
pub fn start_browser_warmup_after_listener(
    browser_service: Arc<dyn crate::services::BrowserService>,
    browser_tool: Option<Arc<dyn moltis_agents::tool_registry::AgentTool>>,
) {
    spawn_post_listener_warmups(browser_service, browser_tool);
}

// ── WebAuthn runtime sync ────────────────────────────────────────────────────

/// Register a runtime-discovered host in the WebAuthn registry.
///
/// Returns a user-facing warning when the host is newly registered and
/// existing passkeys may need to be re-added for that hostname.
pub async fn sync_runtime_webauthn_host_and_notice(
    gateway: &GatewayState,
    registry: Option<&SharedWebAuthnRegistry>,
    hostname: Option<&str>,
    origin_override: Option<&str>,
    source: &str,
) -> Option<String> {
    let hostname = hostname?;
    let normalized = crate::auth_webauthn::normalize_host(hostname);
    if normalized.is_empty() {
        return None;
    }

    let registry = registry?;
    if registry.read().await.contains_host(&normalized) {
        return None;
    }

    let origin = if let Some(origin_override) = origin_override {
        origin_override.to_string()
    } else {
        let scheme = if gateway.tls_active {
            "https"
        } else {
            "http"
        };
        format!("{scheme}://{normalized}:{}", gateway.port)
    };

    let origin_url = match webauthn_rs::prelude::Url::parse(&origin) {
        Ok(url) => url,
        Err(error) => {
            warn!(
                host = %normalized,
                origin = %origin,
                %error,
                "invalid runtime WebAuthn origin from {source}"
            );
            return None;
        },
    };
    let webauthn = match crate::auth_webauthn::WebAuthnState::new(&normalized, &origin_url, &[]) {
        Ok(webauthn) => webauthn,
        Err(error) => {
            warn!(
                host = %normalized,
                origin = %origin,
                %error,
                "failed to initialize runtime WebAuthn RP from {source}"
            );
            return None;
        },
    };

    {
        let mut reg = registry.write().await;
        if reg.contains_host(&normalized) {
            return None;
        }
        reg.add(normalized.clone(), webauthn);
        info!(
            host = %normalized,
            origin = %origin,
            origins = ?reg.get_all_origins(),
            "WebAuthn RP registered from {source}"
        );
    }

    let has_passkeys = if let Some(store) = gateway.credential_store.as_ref() {
        store.has_passkeys().await.unwrap_or(false)
    } else {
        false
    };

    if has_passkeys {
        gateway.add_passkey_host_update_pending(&normalized).await;
        Some(format!(
            "New host detected ({normalized}). Existing passkeys may not work on this host. Sign in with password, then add a new passkey in Settings > Authentication."
        ))
    } else {
        None
    }
}

// ── Feature-gated UI helpers ─────────────────────────────────────────────────

#[cfg(feature = "openclaw-import")]
pub fn openclaw_detected_for_ui() -> bool {
    moltis_openclaw_import::detect().is_some()
}

#[cfg(not(feature = "openclaw-import"))]
pub fn openclaw_detected_for_ui() -> bool {
    false
}

#[cfg(feature = "local-llm")]
#[must_use]
pub fn local_llama_cpp_bytes_for_ui() -> u64 {
    moltis_providers::local_llm::loaded_llama_model_bytes()
}

#[cfg(not(feature = "local-llm"))]
#[must_use]
pub const fn local_llama_cpp_bytes_for_ui() -> u64 {
    0
}
