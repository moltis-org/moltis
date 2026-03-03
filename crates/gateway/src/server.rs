use std::{
    collections::{HashMap, HashSet},
    fs::OpenOptions,
    io::Write,
    net::SocketAddr,
    path::{Path as FsPath, PathBuf},
    sync::Arc,
};

use secrecy::ExposeSecret;

use tracing::{debug, info, warn};

use moltis_channels::ChannelPlugin;

use moltis_providers::ProviderRegistry;

use moltis_tools::{
    approval::{ApprovalManager, ApprovalMode, SecurityLevel},
    exec::EnvVarProvider,
    sessions_communicate::{
        SendToSessionFn, SendToSessionRequest, SessionsHistoryTool, SessionsListTool,
        SessionsSendTool,
    },
    sessions_manage::{
        CreateSessionFn, CreateSessionRequest, DeleteSessionFn, DeleteSessionRequest,
        SessionsCreateTool, SessionsDeleteTool,
    },
};

use {
    moltis_projects::ProjectStore,
    moltis_sessions::{
        metadata::{SessionMetadata, SqliteSessionMetadata},
        session_events::SessionEventBus,
        store::SessionStore,
    },
};

use crate::{
    approval::{GatewayApprovalBroadcaster, LiveExecApprovalService},
    auth,
    auth_webauthn::SharedWebAuthnRegistry,
    broadcast::{BroadcastOpts, broadcast},
    chat::{LiveChatService, LiveModelService},
    methods::MethodRegistry,
    provider_setup::LiveProviderSetupService,
    services::GatewayServices,
    session::LiveSessionService,
    state::GatewayState,
};

#[cfg(feature = "tailscale")]
use crate::tailscale::{
    CliTailscaleManager, TailscaleManager, TailscaleMode, validate_tailscale_config,
};

/// Options for tailscale serve/funnel passed from CLI flags.
#[cfg(feature = "tailscale")]
pub struct TailscaleOpts {
    pub mode: String,
    pub reset_on_exit: bool,
}

// ── Location requester ───────────────────────────────────────────────────────

/// Gateway implementation of [`moltis_tools::location::LocationRequester`].
///
/// Uses the `PendingInvoke` + oneshot pattern to request the user's browser
/// geolocation and waits for `location.result` RPC to resolve it.
struct GatewayLocationRequester {
    state: Arc<GatewayState>,
}

#[async_trait::async_trait]
impl moltis_tools::location::LocationRequester for GatewayLocationRequester {
    async fn request_location(
        &self,
        conn_id: &str,
        precision: moltis_tools::location::LocationPrecision,
    ) -> moltis_tools::Result<moltis_tools::location::LocationResult> {
        use moltis_tools::location::{LocationError, LocationResult};

        let request_id = uuid::Uuid::new_v4().to_string();

        // Send a location.request event to the browser client, including
        // the requested precision so JS can adjust geolocation options.
        let event = moltis_protocol::EventFrame::new(
            "location.request",
            serde_json::json!({ "requestId": request_id, "precision": precision }),
            self.state.next_seq(),
        );
        let event_json = serde_json::to_string(&event)?;

        {
            let inner = self.state.inner.read().await;
            let clients = &inner.clients;
            let client = clients.get(conn_id).ok_or_else(|| {
                moltis_tools::Error::message(format!("no client connection for conn_id {conn_id}"))
            })?;
            if !client.send(&event_json) {
                return Err(moltis_tools::Error::message(format!(
                    "failed to send location request to client {conn_id}"
                )));
            }
        }

        // Set up a oneshot for the result with timeout.
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut inner_w = self.state.inner.write().await;
            let invokes = &mut inner_w.pending_invokes;
            invokes.insert(request_id.clone(), crate::state::PendingInvoke {
                request_id: request_id.clone(),
                sender: tx,
                created_at: std::time::Instant::now(),
            });
        }

        // Wait up to 30 seconds for the user to grant/deny permission.
        let result = match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(value)) => value,
            Ok(Err(_)) => {
                // Sender dropped — clean up.
                self.state
                    .inner
                    .write()
                    .await
                    .pending_invokes
                    .remove(&request_id);
                return Ok(LocationResult {
                    location: None,
                    error: Some(LocationError::Timeout),
                });
            },
            Err(_) => {
                // Timeout — clean up.
                self.state
                    .inner
                    .write()
                    .await
                    .pending_invokes
                    .remove(&request_id);
                return Ok(LocationResult {
                    location: None,
                    error: Some(LocationError::Timeout),
                });
            },
        };

        // Parse the result from the browser.
        if let Some(loc) = result.get("location") {
            let lat = loc.get("latitude").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let lon = loc.get("longitude").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let accuracy = loc.get("accuracy").and_then(|v| v.as_f64()).unwrap_or(0.0);
            Ok(LocationResult {
                location: Some(moltis_tools::location::BrowserLocation {
                    latitude: lat,
                    longitude: lon,
                    accuracy,
                }),
                error: None,
            })
        } else if let Some(err) = result.get("error") {
            let code = err.get("code").and_then(|v| v.as_u64()).unwrap_or(0);
            let error = match code {
                1 => LocationError::PermissionDenied,
                2 => LocationError::PositionUnavailable,
                3 => LocationError::Timeout,
                _ => LocationError::NotSupported,
            };
            Ok(LocationResult {
                location: None,
                error: Some(error),
            })
        } else {
            Ok(LocationResult {
                location: None,
                error: Some(LocationError::PositionUnavailable),
            })
        }
    }

    fn cached_location(&self) -> Option<moltis_config::GeoLocation> {
        self.state.inner.try_read().ok()?.cached_location.clone()
    }

    async fn request_channel_location(
        &self,
        session_key: &str,
    ) -> moltis_tools::Result<moltis_tools::location::LocationResult> {
        use moltis_tools::location::{LocationError, LocationResult};

        // Look up channel binding from session metadata.
        let session_meta = self
            .state
            .services
            .session_metadata
            .as_ref()
            .ok_or_else(|| moltis_tools::Error::message("session metadata not available"))?;
        let entry = session_meta.get(session_key).await.ok_or_else(|| {
            moltis_tools::Error::message(format!("no session metadata for key {session_key}"))
        })?;
        let binding_json = entry.channel_binding.ok_or_else(|| {
            moltis_tools::Error::message(format!("no channel binding for session {session_key}"))
        })?;
        let reply_target: moltis_channels::ChannelReplyTarget =
            serde_json::from_str(&binding_json)?;

        // Send a message asking the user to share their location.
        let outbound = self
            .state
            .services
            .channel_outbound_arc()
            .ok_or_else(|| moltis_tools::Error::message("no channel outbound available"))?;
        outbound
            .send_text(
                &reply_target.account_id,
                &reply_target.chat_id,
                "Please share your location in this chat.",
                None,
            )
            .await
            .map_err(|e| moltis_tools::Error::external("send location request", e))?;

        // Create a pending invoke keyed by session.
        let pending_key = format!("channel_location:{session_key}");
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut inner = self.state.inner.write().await;
            inner
                .pending_invokes
                .insert(pending_key.clone(), crate::state::PendingInvoke {
                    request_id: pending_key.clone(),
                    sender: tx,
                    created_at: std::time::Instant::now(),
                });
        }

        // Wait up to 60 seconds — user needs to navigate Telegram's UI.
        let result = match tokio::time::timeout(std::time::Duration::from_secs(60), rx).await {
            Ok(Ok(value)) => value,
            Ok(Err(_)) => {
                self.state
                    .inner
                    .write()
                    .await
                    .pending_invokes
                    .remove(&pending_key);
                return Ok(LocationResult {
                    location: None,
                    error: Some(LocationError::Timeout),
                });
            },
            Err(_) => {
                self.state
                    .inner
                    .write()
                    .await
                    .pending_invokes
                    .remove(&pending_key);
                return Ok(LocationResult {
                    location: None,
                    error: Some(LocationError::Timeout),
                });
            },
        };

        // Parse the result (same format as update_location sends).
        if let Some(loc) = result.get("location") {
            let lat = loc.get("latitude").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let lon = loc.get("longitude").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let accuracy = loc.get("accuracy").and_then(|v| v.as_f64()).unwrap_or(0.0);
            Ok(LocationResult {
                location: Some(moltis_tools::location::BrowserLocation {
                    latitude: lat,
                    longitude: lon,
                    accuracy,
                }),
                error: None,
            })
        } else {
            Ok(LocationResult {
                location: None,
                error: Some(LocationError::PositionUnavailable),
            })
        }
    }
}

fn should_prebuild_sandbox_image(
    mode: &moltis_tools::sandbox::SandboxMode,
    packages: &[String],
) -> bool {
    !matches!(mode, moltis_tools::sandbox::SandboxMode::Off) && !packages.is_empty()
}

fn instance_slug(config: &moltis_config::MoltisConfig) -> String {
    let mut raw_name = config.identity.name.clone();
    if let Some(file_identity) = moltis_config::load_identity()
        && file_identity.name.is_some()
    {
        raw_name = file_identity.name;
    }

    let base = raw_name
        .unwrap_or_else(|| "moltis".to_string())
        .to_lowercase();
    let mut out = String::new();
    let mut last_dash = false;
    for ch in base.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            ch
        } else {
            '-'
        };
        if mapped == '-' {
            if !last_dash {
                out.push(mapped);
            }
            last_dash = true;
        } else {
            out.push(mapped);
            last_dash = false;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "moltis".to_string()
    } else {
        out
    }
}

fn sandbox_container_prefix(instance_slug: &str) -> String {
    format!("moltis-{instance_slug}-sandbox")
}

fn browser_container_prefix(instance_slug: &str) -> String {
    format!("moltis-{instance_slug}-browser")
}

fn env_value_with_overrides(env_overrides: &HashMap<String, String>, key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            env_overrides
                .get(key)
                .cloned()
                .filter(|value| !value.trim().is_empty())
        })
}

fn merge_env_overrides(
    base_overrides: &HashMap<String, String>,
    additional: Vec<(String, String)>,
) -> HashMap<String, String> {
    let mut merged = base_overrides.clone();
    for (key, value) in additional {
        if key.trim().is_empty() || value.trim().is_empty() {
            continue;
        }
        merged.entry(key).or_insert(value);
    }
    merged
}

fn summarize_model_ids_for_logs(sorted_model_ids: &[String], max_items: usize) -> Vec<String> {
    if max_items == 0 {
        return Vec::new();
    }

    if sorted_model_ids.len() <= max_items || max_items < 3 {
        return sorted_model_ids.iter().take(max_items).cloned().collect();
    }

    let head_count = max_items / 2;
    let tail_count = max_items - head_count - 1;
    let mut sample = Vec::with_capacity(max_items);
    sample.extend(sorted_model_ids.iter().take(head_count).cloned());
    sample.push("...".to_string());
    sample.extend(
        sorted_model_ids
            .iter()
            .skip(sorted_model_ids.len().saturating_sub(tail_count))
            .cloned(),
    );
    sample
}

fn log_startup_model_inventory(reg: &ProviderRegistry) {
    const STARTUP_MODEL_SAMPLE_SIZE: usize = 8;
    const STARTUP_PROVIDER_MODEL_SAMPLE_SIZE: usize = 4;

    let mut by_provider: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    let mut model_ids: Vec<String> = Vec::with_capacity(reg.list_models().len());
    for model in reg.list_models() {
        model_ids.push(model.id.clone());
        by_provider
            .entry(model.provider.clone())
            .or_default()
            .push(model.id.clone());
    }
    model_ids.sort();

    let provider_model_counts: Vec<(String, usize)> = by_provider
        .iter()
        .map(|(provider, provider_models)| (provider.clone(), provider_models.len()))
        .collect();

    info!(
        model_count = model_ids.len(),
        provider_count = by_provider.len(),
        provider_model_counts = ?provider_model_counts,
        sample_model_ids = ?summarize_model_ids_for_logs(&model_ids, STARTUP_MODEL_SAMPLE_SIZE),
        "startup model inventory"
    );

    for (provider, provider_models) in &mut by_provider {
        provider_models.sort();
        debug!(
            provider = %provider,
            model_count = provider_models.len(),
            sample_model_ids = ?summarize_model_ids_for_logs(
                provider_models,
                STARTUP_PROVIDER_MODEL_SAMPLE_SIZE
            ),
            "startup provider model inventory"
        );
    }
}

async fn ollama_has_model(base_url: &str, model: &str) -> bool {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let response = match reqwest::Client::new().get(url).send().await {
        Ok(resp) => resp,
        Err(_) => return false,
    };
    if !response.status().is_success() {
        return false;
    }
    let value: serde_json::Value = match response.json().await {
        Ok(v) => v,
        Err(_) => return false,
    };
    value
        .get("models")
        .and_then(|m| m.as_array())
        .map(|models| {
            models.iter().any(|m| {
                let name = m.get("name").and_then(|n| n.as_str()).unwrap_or_default();
                name == model || name.starts_with(&format!("{model}:"))
            })
        })
        .unwrap_or(false)
}

async fn ensure_ollama_model(base_url: &str, model: &str) {
    if ollama_has_model(base_url, model).await {
        return;
    }

    warn!(
        model = %model,
        base_url = %base_url,
        "memory: missing Ollama embedding model, attempting auto-pull"
    );

    let url = format!("{}/api/pull", base_url.trim_end_matches('/'));
    let pull = reqwest::Client::new()
        .post(url)
        .json(&serde_json::json!({ "name": model, "stream": false }))
        .send()
        .await;

    match pull {
        Ok(resp) if resp.status().is_success() => {
            info!(model = %model, "memory: Ollama model pull complete");
        },
        Ok(resp) => {
            warn!(
                model = %model,
                status = %resp.status(),
                "memory: Ollama model pull failed"
            );
        },
        Err(e) => {
            warn!(model = %model, error = %e, "memory: Ollama model pull request failed");
        },
    }
}

pub fn approval_manager_from_config(config: &moltis_config::MoltisConfig) -> ApprovalManager {
    let mut manager = ApprovalManager::default();

    manager.mode = ApprovalMode::parse(&config.tools.exec.approval_mode).unwrap_or_else(|| {
        warn!(
            value = %config.tools.exec.approval_mode,
            "invalid tools.exec.approval_mode; falling back to 'on-miss'"
        );
        ApprovalMode::OnMiss
    });

    manager.security_level = SecurityLevel::parse(&config.tools.exec.security_level)
        .unwrap_or_else(|| {
            warn!(
                value = %config.tools.exec.security_level,
                "invalid tools.exec.security_level; falling back to 'allowlist'"
            );
            SecurityLevel::Allowlist
        });

    manager.allowlist = config.tools.exec.allowlist.clone();
    manager
}

fn env_var_or_unset(name: &str) -> String {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "<unset>".to_string())
}

fn env_flag_enabled(name: &str) -> bool {
    std::env::var(name)
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn validate_proxy_tls_configuration(
    behind_proxy: bool,
    tls_enabled: bool,
    allow_tls_behind_proxy: bool,
) -> anyhow::Result<()> {
    if behind_proxy && tls_enabled && !allow_tls_behind_proxy {
        anyhow::bail!(
            "MOLTIS_BEHIND_PROXY=true with Moltis TLS enabled is usually a proxy misconfiguration. Run with --no-tls (or MOLTIS_NO_TLS=true). If your proxy upstream is HTTPS/TCP passthrough by design, set MOLTIS_ALLOW_TLS_BEHIND_PROXY=true."
        );
    }
    Ok(())
}

fn log_path_diagnostics(kind: &str, path: &FsPath) {
    match std::fs::metadata(path) {
        Ok(metadata) => {
            info!(
                kind,
                path = %path.display(),
                exists = true,
                is_dir = metadata.is_dir(),
                readonly = metadata.permissions().readonly(),
                size_bytes = metadata.len(),
                "startup path diagnostics"
            );
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            info!(kind, path = %path.display(), exists = false, "startup path missing");
        },
        Err(error) => {
            warn!(
                kind,
                path = %path.display(),
                error = %error,
                "failed to inspect startup path"
            );
        },
    }
}

fn log_directory_write_probe(dir: &FsPath) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let probe_path = dir.join(format!(
        ".moltis-write-check-{}-{nanos}.tmp",
        std::process::id()
    ));

    match OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&probe_path)
    {
        Ok(mut file) => {
            if let Err(error) = file.write_all(b"probe") {
                warn!(
                    path = %probe_path.display(),
                    error = %error,
                    "startup write probe could not write to config directory"
                );
            } else {
                info!(
                    path = %probe_path.display(),
                    "startup write probe succeeded for config directory"
                );
            }
            if let Err(error) = std::fs::remove_file(&probe_path) {
                warn!(
                    path = %probe_path.display(),
                    error = %error,
                    "failed to clean up startup write probe file"
                );
            }
        },
        Err(error) => {
            warn!(
                path = %probe_path.display(),
                error = %error,
                "startup write probe failed for config directory"
            );
        },
    }
}

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
                openclaw_home_env = %env_var_or_unset("OPENCLAW_HOME"),
                openclaw_profile_env = %env_var_or_unset("OPENCLAW_PROFILE"),
                "startup OpenClaw installation not detected (checked OPENCLAW_HOME and ~/.openclaw)"
            );
            None
        },
    }
}

#[cfg(feature = "openclaw-import")]
fn deferred_openclaw_status() -> String {
    "background detection pending".to_string()
}

#[cfg(not(feature = "openclaw-import"))]
fn deferred_openclaw_status() -> String {
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

fn spawn_post_listener_warmups(
    browser_service: Arc<dyn crate::services::BrowserService>,
    browser_tool: Option<Arc<dyn moltis_agents::tool_registry::AgentTool>>,
) {
    tokio::spawn(async move {
        browser_service.warmup().await;
        if let Some(tool) = browser_tool
            && let Err(error) = tool.warmup().await
        {
            warn!(%error, "browser tool warmup failed");
        }
    });
}

#[cfg(feature = "tailscale")]
fn spawn_webauthn_tailscale_registration(
    registry: SharedWebAuthnRegistry,
    default_scheme: String,
    port: u16,
) {
    tokio::spawn(async move {
        let started = std::time::Instant::now();
        match CliTailscaleManager::new().hostname().await {
            Ok(Some(ts_hostname)) => {
                let ts_host = crate::auth_webauthn::normalize_host(&ts_hostname);
                if ts_host.is_empty() {
                    debug!(
                        elapsed_ms = started.elapsed().as_millis(),
                        "tailscale hostname is empty, skipping WebAuthn RP registration"
                    );
                    return;
                }

                let ts_origin = format!("{default_scheme}://{ts_host}:{port}");
                let origin_url = match webauthn_rs::prelude::Url::parse(&ts_origin) {
                    Ok(origin_url) => origin_url,
                    Err(error) => {
                        warn!(
                            hostname = %ts_hostname,
                            origin = %ts_origin,
                            %error,
                            "invalid Tailscale WebAuthn origin URL"
                        );
                        return;
                    },
                };
                let webauthn_state =
                    match crate::auth_webauthn::WebAuthnState::new(&ts_host, &origin_url, &[]) {
                        Ok(webauthn_state) => webauthn_state,
                        Err(error) => {
                            warn!(
                                rp_id = %ts_host,
                                %error,
                                "failed to initialize Tailscale WebAuthn RP"
                            );
                            return;
                        },
                    };

                let mut registry = registry.write().await;
                if registry.contains_host(&ts_host) {
                    debug!(
                        rp_id = %ts_host,
                        elapsed_ms = started.elapsed().as_millis(),
                        "tailscale hostname already registered in WebAuthn registry"
                    );
                    return;
                }

                registry.add(ts_host.clone(), webauthn_state);
                let origins = registry.get_all_origins();
                drop(registry);

                info!(
                    rp_id = %ts_host,
                    origin = %ts_origin,
                    elapsed_ms = started.elapsed().as_millis(),
                    "WebAuthn RP registered from Tailscale hostname"
                );
                info!(origins = ?origins, "WebAuthn passkeys origins updated");
            },
            Ok(None) => {
                debug!(
                    elapsed_ms = started.elapsed().as_millis(),
                    "tailscale hostname unavailable, skipping WebAuthn RP registration"
                );
            },
            Err(error) => {
                debug!(
                    %error,
                    elapsed_ms = started.elapsed().as_millis(),
                    "tailscale hostname lookup failed, skipping WebAuthn RP registration"
                );
            },
        }
    });
}

#[cfg(feature = "openclaw-import")]
pub fn openclaw_detected_for_ui() -> bool {
    moltis_openclaw_import::detect().is_some()
}

#[cfg(not(feature = "openclaw-import"))]
pub fn openclaw_detected_for_ui() -> bool {
    false
}

fn log_startup_config_storage_diagnostics() {
    let config_dir = moltis_config::config_dir().unwrap_or_else(|| PathBuf::from(".moltis"));
    let discovered_config = moltis_config::loader::find_config_file();
    let expected_config = moltis_config::find_or_default_config_path();
    let provider_keys_path = config_dir.join("provider_keys.json");

    let discovered_display = discovered_config
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<none>".to_string());
    info!(
        user = %env_var_or_unset("USER"),
        home = %env_var_or_unset("HOME"),
        config_dir = %config_dir.display(),
        discovered_config = %discovered_display,
        expected_config = %expected_config.display(),
        provider_keys_path = %provider_keys_path.display(),
        "startup configuration storage diagnostics"
    );

    log_path_diagnostics("config-dir", &config_dir);
    log_directory_write_probe(&config_dir);

    if let Some(path) = discovered_config {
        log_path_diagnostics("config-file", &path);
    } else if expected_config.exists() {
        info!(
            path = %expected_config.display(),
            "default config file exists even though discovery did not report a named config"
        );
        log_path_diagnostics("config-file", &expected_config);
    } else {
        warn!(
            path = %expected_config.display(),
            "no config file detected on startup; Moltis is running with in-memory defaults until config is persisted"
        );
    }

    if provider_keys_path.exists() {
        log_path_diagnostics("provider-keys", &provider_keys_path);
        match std::fs::read_to_string(&provider_keys_path) {
            Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                Ok(_) => {
                    info!(
                        path = %provider_keys_path.display(),
                        bytes = content.len(),
                        "provider key store file is readable JSON"
                    );
                },
                Err(error) => {
                    warn!(
                        path = %provider_keys_path.display(),
                        error = %error,
                        "provider key store file contains invalid JSON"
                    );
                },
            },
            Err(error) => {
                warn!(
                    path = %provider_keys_path.display(),
                    error = %error,
                    "provider key store file exists but is not readable"
                );
            },
        }
    } else {
        info!(
            path = %provider_keys_path.display(),
            "provider key store file not found yet; it will be created after the first providers.save_key"
        );
    }
}

/// Core gateway state produced by [`prepare_gateway_core`].
///
/// Contains everything needed to build an HTTP server on top of the core, but
/// no HTTP/transport-specific types. Non-HTTP consumers (TUI, tests) can stop
/// at this level.
pub struct PreparedGatewayCore {
    /// Shared gateway state (sessions, services, config, etc.).
    pub state: Arc<GatewayState>,
    /// RPC method registry.
    pub methods: Arc<MethodRegistry>,
    /// WebAuthn registry for passkey auth.
    pub webauthn_registry: Option<SharedWebAuthnRegistry>,
    /// MS Teams webhook plugin (always present, may be empty).
    pub msteams_webhook_plugin: Arc<tokio::sync::RwLock<moltis_msteams::MsTeamsPlugin>>,
    /// Slack webhook plugin.
    #[cfg(feature = "slack")]
    pub slack_webhook_plugin: Arc<tokio::sync::RwLock<moltis_slack::SlackPlugin>>,
    /// Push notification service.
    #[cfg(feature = "push-notifications")]
    pub push_service: Option<Arc<crate::push::PushService>>,
    /// Network audit buffer (trusted-network proxy).
    #[cfg(feature = "trusted-network")]
    pub audit_buffer: Option<crate::network_audit::NetworkAuditBuffer>,
    /// Sandbox router for container backends.
    pub sandbox_router: Arc<moltis_tools::sandbox::SandboxRouter>,
    /// Browser service for lifecycle management.
    pub browser_for_lifecycle: Arc<dyn crate::services::BrowserService>,
    /// Cron scheduler service.
    pub cron_service: Arc<moltis_cron::service::CronService>,
    /// Log buffer for real-time log streaming.
    pub log_buffer: Option<crate::logs::LogBuffer>,
    /// Loaded configuration snapshot.
    pub config: moltis_config::schema::MoltisConfig,
    /// Resolved data directory.
    pub data_dir: PathBuf,
    /// Human-readable provider summary for the startup banner.
    pub provider_summary: String,
    /// Number of configured MCP servers.
    pub mcp_configured_count: usize,
    /// OpenClaw detection status string.
    pub openclaw_status: String,
    /// One-time setup code (when auth setup is pending).
    pub setup_code_display: Option<String>,
    /// Resolved port.
    pub port: u16,
    /// Whether TLS is active for this gateway instance.
    pub tls_enabled: bool,
    /// Tailscale mode.
    #[cfg(feature = "tailscale")]
    pub tailscale_mode: TailscaleMode,
    /// Whether to reset tailscale on exit.
    #[cfg(feature = "tailscale")]
    pub tailscale_reset_on_exit: bool,
}

/// Prepare the core gateway: load config, run migrations, wire services,
/// spawn background tasks, and return the core state without any HTTP layer.
///
/// This is the transport-agnostic initialisation. Non-HTTP consumers (TUI,
/// tests) can stop here. HTTP consumers call [`prepare_gateway`] which
/// delegates to this and then adds the router + middleware.
#[allow(clippy::expect_used)] // Startup fail-fast: DB, migrations, credential store must succeed.
pub async fn prepare_gateway_core(
    bind: &str,
    port: u16,
    no_tls: bool,
    log_buffer: Option<crate::logs::LogBuffer>,
    config_dir: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    #[cfg(feature = "tailscale")] tailscale_opts: Option<TailscaleOpts>,
    session_event_bus: Option<SessionEventBus>,
) -> anyhow::Result<PreparedGatewayCore> {
    let session_event_bus = session_event_bus.unwrap_or_default();

    // Apply directory overrides before loading config.
    if let Some(dir) = config_dir {
        moltis_config::set_config_dir(dir);
    }
    if let Some(ref dir) = data_dir {
        moltis_config::set_data_dir(dir.clone());
    }

    // Resolve auth from environment (MOLTIS_TOKEN / MOLTIS_PASSWORD).
    let token = std::env::var("MOLTIS_TOKEN").ok();
    let password = std::env::var("MOLTIS_PASSWORD").ok();

    // Cloud deploy platform — hides local-only providers (local-llm, ollama).
    let deploy_platform = std::env::var("MOLTIS_DEPLOY_PLATFORM").ok();
    let resolved_auth = auth::resolve_auth(token, password.clone());

    // Load config file (moltis.toml / .yaml / .json) if present.
    let mut config = moltis_config::discover_and_load();
    let config_env_overrides = config.env.clone();
    let instance_slug_value = instance_slug(&config);
    let browser_container_prefix = browser_container_prefix(&instance_slug_value);
    let sandbox_container_prefix = sandbox_container_prefix(&instance_slug_value);

    // CLI --no-tls / MOLTIS_NO_TLS overrides config file TLS setting.
    if no_tls {
        config.tls.enabled = false;
    }
    let behind_proxy = env_flag_enabled("MOLTIS_BEHIND_PROXY");
    let allow_tls_behind_proxy = env_flag_enabled("MOLTIS_ALLOW_TLS_BEHIND_PROXY");
    #[cfg(feature = "tls")]
    let tls_enabled_for_gateway = config.tls.enabled;
    #[cfg(not(feature = "tls"))]
    let tls_enabled_for_gateway = false;
    validate_proxy_tls_configuration(
        behind_proxy,
        tls_enabled_for_gateway,
        allow_tls_behind_proxy,
    )?;
    if behind_proxy && tls_enabled_for_gateway && allow_tls_behind_proxy {
        warn!(
            "MOLTIS_ALLOW_TLS_BEHIND_PROXY=true is set; ensure your proxy uses HTTPS upstream or TLS passthrough to avoid redirect loops"
        );
    }

    let base_provider_config = config.providers.clone();

    // Merge any previously saved API keys into the provider config so they
    // survive gateway restarts without requiring env vars.
    let key_store = crate::provider_setup::KeyStore::new();
    // Collect local-llm model IDs (if the feature is enabled and models are configured).
    #[cfg(feature = "local-llm")]
    let local_model_ids: Vec<String> = crate::local_llm_setup::LocalLlmConfig::load()
        .map(|c| c.models.iter().map(|m| m.model_id.clone()).collect())
        .unwrap_or_default();
    #[cfg(not(feature = "local-llm"))]
    let local_model_ids: Vec<String> = Vec::new();

    let effective_providers = crate::provider_setup::config_with_saved_keys(
        &base_provider_config,
        &key_store,
        &local_model_ids,
    );

    let has_explicit_provider_settings =
        crate::provider_setup::has_explicit_provider_settings(&config.providers);
    let auto_detected_provider_sources = if has_explicit_provider_settings {
        Vec::new()
    } else {
        crate::provider_setup::detect_auto_provider_sources_with_overrides(
            &config.providers,
            deploy_platform.as_deref(),
            &config_env_overrides,
        )
    };

    // Kick off discovery workers immediately, but build a static startup
    // registry first so gateway startup does not block on network I/O.
    let startup_discovery_pending =
        ProviderRegistry::fire_discoveries(&effective_providers, &config_env_overrides);
    let registry = Arc::new(tokio::sync::RwLock::new(
        ProviderRegistry::from_config_with_static_catalogs(
            &effective_providers,
            &config_env_overrides,
        ),
    ));
    let (provider_summary, providers_available_at_startup) = {
        let reg = registry.read().await;
        log_startup_model_inventory(&reg);
        (reg.provider_summary(), !reg.is_empty())
    };
    if !providers_available_at_startup {
        let config_path = moltis_config::find_or_default_config_path();
        let provider_keys_path = moltis_config::config_dir()
            .unwrap_or_else(|| PathBuf::from(".moltis"))
            .join("provider_keys.json");
        warn!(
            provider_summary = %provider_summary,
            config_path = %config_path.display(),
            provider_keys_path = %provider_keys_path.display(),
            "no LLM providers in static startup catalog; model/chat services remain active and will pick up providers after credentials are saved or background discovery completes"
        );
    }

    if !has_explicit_provider_settings {
        if auto_detected_provider_sources.is_empty() {
            info!("llm auto-detect: no providers detected from env/files");
        } else {
            for detected in &auto_detected_provider_sources {
                info!(
                    provider = %detected.provider,
                    source = %detected.source,
                    "llm auto-detected provider source"
                );
            }
            // Import external tokens (e.g. Codex CLI auth.json) into the
            // token store so all providers read from a single location.
            let import_token_store = moltis_oauth::TokenStore::new();
            crate::provider_setup::import_detected_oauth_tokens(
                &auto_detected_provider_sources,
                &import_token_store,
            );
        }
    }

    // Refresh dynamic provider model discovery hourly so long-lived sessions
    // pick up newly available models without requiring a restart.
    {
        let registry_for_refresh = Arc::clone(&registry);
        let provider_config_for_refresh = base_provider_config.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60 * 60));
            interval.tick().await;
            loop {
                interval.tick().await;
                let mut reg = registry_for_refresh.write().await;
                let refresh_results = reg.refresh_dynamic_models(&provider_config_for_refresh);
                for (provider_name, refreshed) in refresh_results {
                    if !refreshed {
                        continue;
                    }
                    let model_count = reg
                        .list_models()
                        .iter()
                        .filter(|m| m.provider == provider_name)
                        .count();
                    info!(
                        provider = %provider_name,
                        models = model_count,
                        "hourly dynamic provider model refresh complete"
                    );
                }
            }
        });
    }

    // Create shared approval manager from config.
    let approval_manager = Arc::new(approval_manager_from_config(&config));

    let mut services = GatewayServices::noop();

    // Wire live logs service if a log buffer is available.
    if let Some(ref buf) = log_buffer {
        services.logs = Arc::new(crate::logs::LiveLogsService::new(buf.clone()));
    }

    services.exec_approval = Arc::new(LiveExecApprovalService::new(Arc::clone(&approval_manager)));

    // Wire browser service if enabled.
    if let Some(browser_svc) =
        crate::services::RealBrowserService::from_config(&config, browser_container_prefix)
    {
        services.browser = Arc::new(browser_svc);
    }

    // Wire live onboarding service.
    let onboarding_config_path = moltis_config::find_or_default_config_path();
    let live_onboarding =
        moltis_onboarding::service::LiveOnboardingService::new(onboarding_config_path);
    // Wire live local-llm service when the feature is enabled.
    #[cfg(feature = "local-llm")]
    let local_llm_service: Option<Arc<crate::local_llm_setup::LiveLocalLlmService>> = {
        let svc = Arc::new(crate::local_llm_setup::LiveLocalLlmService::new(
            Arc::clone(&registry),
        ));
        services =
            services.with_local_llm(Arc::clone(&svc) as Arc<dyn crate::services::LocalLlmService>);
        Some(svc)
    };
    // When local-llm feature is disabled, this variable is not needed since
    // the only usage is also feature-gated.

    // Wire live voice services when the feature is enabled.
    #[cfg(feature = "voice")]
    {
        use crate::voice::{LiveSttService, LiveTtsService, SttServiceConfig};

        // Services read fresh config from disk on each operation,
        // so we just need to create the instances here.
        services.tts = Arc::new(LiveTtsService::new(moltis_voice::TtsConfig::default()));
        services.stt = Arc::new(LiveSttService::new(SttServiceConfig::default()));
    }

    let model_store = Arc::new(tokio::sync::RwLock::new(
        crate::chat::DisabledModelsStore::load(),
    ));

    let live_model_service = Arc::new(LiveModelService::new(
        Arc::clone(&registry),
        Arc::clone(&model_store),
        config.chat.priority_models.clone(),
    ));
    services = services
        .with_model(Arc::clone(&live_model_service) as Arc<dyn crate::services::ModelService>);

    // Create provider setup after model service so we can share the
    // priority models handle for live dropdown reordering.
    let mut provider_setup = LiveProviderSetupService::new(
        Arc::clone(&registry),
        config.providers.clone(),
        deploy_platform.clone(),
    )
    .with_env_overrides(config_env_overrides.clone())
    .with_error_parser(crate::chat_error::parse_chat_error)
    .with_callback_bind_addr(bind.to_string());
    provider_setup.set_priority_models(live_model_service.priority_models_handle());
    let provider_setup_service = Arc::new(provider_setup);
    services.provider_setup =
        Arc::clone(&provider_setup_service) as Arc<dyn crate::services::ProviderSetupService>;

    // Wire live MCP service.
    let mcp_configured_count;
    let live_mcp: Arc<crate::mcp_service::LiveMcpService>;
    {
        let mcp_registry_path = moltis_config::data_dir().join("mcp-servers.json");
        let mcp_reg = moltis_mcp::McpRegistry::load(&mcp_registry_path).unwrap_or_default();
        // Seed from config file servers that aren't already in the registry.
        let mut merged = mcp_reg;
        for (name, entry) in &config.mcp.servers {
            if !merged.servers.contains_key(name) {
                let transport = match entry.transport.as_str() {
                    "sse" => moltis_mcp::registry::TransportType::Sse,
                    _ => moltis_mcp::registry::TransportType::Stdio,
                };
                let oauth = entry
                    .oauth
                    .as_ref()
                    .map(|o| moltis_mcp::registry::McpOAuthConfig {
                        client_id: o.client_id.clone(),
                        auth_url: o.auth_url.clone(),
                        token_url: o.token_url.clone(),
                        scopes: o.scopes.clone(),
                    });
                merged
                    .servers
                    .insert(name.clone(), moltis_mcp::McpServerConfig {
                        command: entry.command.clone(),
                        args: entry.args.clone(),
                        env: entry.env.clone(),
                        enabled: entry.enabled,
                        transport,
                        url: entry.url.clone(),
                        oauth,
                    });
            }
        }
        mcp_configured_count = merged.servers.values().filter(|s| s.enabled).count();
        let mcp_manager = Arc::new(moltis_mcp::McpManager::new(merged));
        live_mcp = Arc::new(crate::mcp_service::LiveMcpService::new(Arc::clone(
            &mcp_manager,
        )));
        // Start enabled servers in the background; sync tools once done.
        let mgr = Arc::clone(&mcp_manager);
        let mcp_for_sync = Arc::clone(&live_mcp);
        tokio::spawn(async move {
            let started = mgr.start_enabled().await;
            if !started.is_empty() {
                tracing::info!(servers = ?started, "MCP servers started");
            }
            // Sync newly started tools into the agent tool registry.
            mcp_for_sync.sync_tools_if_ready().await;
        });
        services.mcp = live_mcp.clone() as Arc<dyn crate::services::McpService>;
    }

    // Initialize data directory and SQLite database.
    let data_dir = data_dir.unwrap_or_else(moltis_config::data_dir);
    std::fs::create_dir_all(&data_dir).unwrap_or_else(|e| {
        panic!(
            "failed to create data directory {}: {e}",
            data_dir.display()
        )
    });

    let config_dir = moltis_config::config_dir().unwrap_or_else(|| PathBuf::from(".moltis"));
    std::fs::create_dir_all(&config_dir).unwrap_or_else(|e| {
        panic!(
            "failed to create config directory {}: {e}",
            config_dir.display()
        )
    });
    log_startup_config_storage_diagnostics();

    let openclaw_startup_status = deferred_openclaw_status();

    // Enable log persistence so entries survive restarts.
    if let Some(ref buf) = log_buffer {
        let log_buffer_for_persistence = buf.clone();
        let persistence_path = data_dir.join("logs.jsonl");
        tokio::spawn(async move {
            let started = std::time::Instant::now();
            match tokio::task::spawn_blocking(move || {
                log_buffer_for_persistence.enable_persistence(persistence_path.clone());
                persistence_path
            })
            .await
            {
                Ok(path) => {
                    debug!(
                        path = %path.display(),
                        elapsed_ms = started.elapsed().as_millis(),
                        "startup log persistence initialized"
                    );
                },
                Err(error) => {
                    warn!(
                        %error,
                        "startup log persistence initialization worker failed"
                    );
                },
            }
        });
    }
    let db_path = data_dir.join("moltis.db");
    let db_pool = {
        use {
            sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
            std::str::FromStr,
        };
        let db_exists = db_path.exists();
        let mut options = SqliteConnectOptions::from_str(&format!("sqlite:{}", db_path.display()))
            .expect("invalid database path")
            .create_if_missing(true)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(std::time::Duration::from_secs(5));
        if !db_exists {
            // Setting journal_mode can briefly require an exclusive lock.
            // For existing databases, preserve current mode to avoid startup stalls.
            options = options.journal_mode(SqliteJournalMode::Wal);
        }

        let started = std::time::Instant::now();
        let pool = sqlx::pool::PoolOptions::new()
            .max_connections(config.server.db_pool_max_connections)
            .connect_with(options)
            .await
            .expect("failed to open moltis.db");
        debug!(
            path = %db_path.display(),
            db_exists,
            elapsed_ms = started.elapsed().as_millis(),
            "startup sqlite pool connected"
        );
        pool
    };

    // Run database migrations from each crate in dependency order.
    // Order matters: sessions depends on projects (FK reference).
    moltis_projects::run_migrations(&db_pool)
        .await
        .expect("failed to run projects migrations");
    moltis_sessions::run_migrations(&db_pool)
        .await
        .expect("failed to run sessions migrations");
    moltis_cron::run_migrations(&db_pool)
        .await
        .expect("failed to run cron migrations");
    // Gateway's own tables (auth, message_log, channels).
    crate::run_migrations(&db_pool)
        .await
        .expect("failed to run gateway migrations");

    // Vault migrations (vault_metadata table).
    #[cfg(feature = "vault")]
    moltis_vault::run_migrations(&db_pool)
        .await
        .expect("failed to run vault migrations");

    // Migrate plugins data into unified skills system (idempotent, non-fatal).
    moltis_skills::migration::migrate_plugins_to_skills(&data_dir).await;

    // Initialize vault for encryption-at-rest.
    #[cfg(feature = "vault")]
    let vault: Option<Arc<moltis_vault::Vault>> = {
        match moltis_vault::Vault::new(db_pool.clone()).await {
            Ok(v) => {
                info!(status = ?v.status().await, "vault ready");
                Some(Arc::new(v))
            },
            Err(e) => {
                warn!(error = %e, "vault init failed, encryption disabled");
                None
            },
        }
    };

    // Initialize credential store (auth tables).
    #[cfg(feature = "vault")]
    let credential_store = Arc::new(
        auth::CredentialStore::with_vault(db_pool.clone(), &config.auth, vault.clone())
            .await
            .expect("failed to init credential store"),
    );
    #[cfg(not(feature = "vault"))]
    let credential_store = Arc::new(
        auth::CredentialStore::new(db_pool.clone())
            .await
            .expect("failed to init credential store"),
    );

    // Runtime env overrides from the settings UI (`/api/env`) layered after
    // config `[env]`. Process env remains highest precedence.
    let runtime_env_overrides = match credential_store.get_all_env_values().await {
        Ok(db_env_vars) => merge_env_overrides(&config_env_overrides, db_env_vars),
        Err(error) => {
            warn!(%error, "failed to load persisted env overrides from credential store");
            config_env_overrides.clone()
        },
    };

    // Initialize WebAuthn registry for passkey support.
    // Each hostname the user may access from gets its own RP ID + origins entry
    // so passkeys work from localhost, mDNS hostname, and .local alike.
    let default_scheme = if config.tls.enabled {
        "https"
    } else {
        "http"
    };

    // Explicit RP ID from env (PaaS platforms).
    let explicit_rp_id = std::env::var("MOLTIS_WEBAUTHN_RP_ID")
        .or_else(|_| std::env::var("APP_DOMAIN"))
        .or_else(|_| std::env::var("RENDER_EXTERNAL_HOSTNAME"))
        .or_else(|_| std::env::var("FLY_APP_NAME").map(|name| format!("{name}.fly.dev")))
        .or_else(|_| std::env::var("RAILWAY_PUBLIC_DOMAIN"))
        .ok();

    let explicit_origin = std::env::var("MOLTIS_WEBAUTHN_ORIGIN")
        .or_else(|_| std::env::var("APP_URL"))
        .or_else(|_| std::env::var("RENDER_EXTERNAL_URL"))
        .ok();

    let webauthn_registry = {
        let mut registry = crate::auth_webauthn::WebAuthnRegistry::new();
        let mut any_ok = false;

        // Helper: try to add one RP ID with its origin + extras to the registry.
        let mut try_add = |rp_id: &str, origin_str: &str, extras: &[webauthn_rs::prelude::Url]| {
            let rp_id = crate::auth_webauthn::normalize_host(rp_id);
            if rp_id.is_empty() || registry.contains_host(&rp_id) {
                return;
            }
            let Ok(origin_url) = webauthn_rs::prelude::Url::parse(origin_str) else {
                tracing::warn!("invalid WebAuthn origin URL '{origin_str}'");
                return;
            };
            match crate::auth_webauthn::WebAuthnState::new(&rp_id, &origin_url, extras) {
                Ok(wa) => {
                    info!(rp_id = %rp_id, origins = ?wa.get_allowed_origins(), "WebAuthn RP registered");
                    registry.add(rp_id.clone(), wa);
                    any_ok = true;
                },
                Err(e) => tracing::warn!(rp_id = %rp_id, "failed to init WebAuthn: {e}"),
            }
        };

        if let Some(ref rp_id) = explicit_rp_id {
            // PaaS: single explicit RP ID.
            let origin = explicit_origin
                .clone()
                .unwrap_or_else(|| format!("https://{rp_id}"));
            try_add(rp_id, &origin, &[]);
        } else {
            // Local: register localhost + moltis.localhost as extras.
            let localhost_origin = format!("{default_scheme}://localhost:{port}");
            let moltis_localhost: Vec<webauthn_rs::prelude::Url> =
                webauthn_rs::prelude::Url::parse(&format!(
                    "{default_scheme}://moltis.localhost:{port}"
                ))
                .into_iter()
                .collect();
            try_add("localhost", &localhost_origin, &moltis_localhost);

            // Register identity-derived host aliases (`<bot-name>` and
            // `<bot-name>.local`) so passkeys work when clients connect using
            // bot-name based local DNS/mDNS labels.
            let bot_slug = instance_slug_value.clone();
            if bot_slug != "localhost" {
                let bot_origin = format!("{default_scheme}://{bot_slug}:{port}");
                try_add(&bot_slug, &bot_origin, &[]);

                let bot_local = format!("{bot_slug}.local");
                let bot_local_origin = format!("{default_scheme}://{bot_local}:{port}");
                try_add(&bot_local, &bot_local_origin, &[]);
            }

            // Register system hostname and hostname.local for LAN/mDNS access.
            if let Ok(hn) = hostname::get() {
                let hn_str = hn.to_string_lossy();
                if hn_str != "localhost" {
                    // hostname.local as RP ID (mDNS access)
                    let local_name = if hn_str.ends_with(".local") {
                        hn_str.to_string()
                    } else {
                        format!("{hn_str}.local")
                    };
                    let local_origin = format!("{default_scheme}://{local_name}:{port}");
                    try_add(&local_name, &local_origin, &[]);

                    // bare hostname as RP ID (direct LAN access)
                    let bare = hn_str.strip_suffix(".local").unwrap_or(&hn_str);
                    if bare != local_name {
                        let bare_origin = format!("{default_scheme}://{bare}:{port}");
                        try_add(bare, &bare_origin, &[]);
                    }
                }
            }
        }

        if any_ok {
            info!(origins = ?registry.get_all_origins(), "WebAuthn passkeys enabled");
            Some(Arc::new(tokio::sync::RwLock::new(registry)))
        } else {
            None
        }
    };

    #[cfg(feature = "tailscale")]
    if explicit_rp_id.is_none()
        && let Some(registry) = webauthn_registry.as_ref()
    {
        spawn_webauthn_tailscale_registration(
            Arc::clone(registry),
            default_scheme.to_string(),
            port,
        );
    }

    // If MOLTIS_PASSWORD is set and no password in DB yet, migrate it.
    if let Some(ref pw) = password
        && !credential_store.is_setup_complete()
    {
        info!("migrating MOLTIS_PASSWORD env var to credential store");
        if let Err(e) = credential_store.set_initial_password(pw).await {
            tracing::warn!("failed to migrate env password: {e}");
        }
    }

    let message_log: Arc<dyn moltis_channels::message_log::MessageLog> = Arc::new(
        crate::message_log_store::SqliteMessageLog::new(db_pool.clone()),
    );

    // Migrate from projects.toml if it exists.
    let config_dir = moltis_config::config_dir().unwrap_or_else(|| PathBuf::from(".moltis"));
    let projects_toml_path = config_dir.join("projects.toml");
    if projects_toml_path.exists() {
        info!("migrating projects.toml to SQLite");
        let old_store = moltis_projects::TomlProjectStore::new(projects_toml_path.clone());
        let sqlite_store = moltis_projects::SqliteProjectStore::new(db_pool.clone());
        if let Ok(projects) =
            <moltis_projects::TomlProjectStore as ProjectStore>::list(&old_store).await
        {
            for p in projects {
                if let Err(e) = sqlite_store.upsert(p).await {
                    tracing::warn!("failed to migrate project: {e}");
                }
            }
        }
        let bak = projects_toml_path.with_extension("toml.bak");
        std::fs::rename(&projects_toml_path, &bak).ok();
    }

    // Migrate from metadata.json if it exists.
    let sessions_dir = data_dir.join("sessions");
    let metadata_json_path = sessions_dir.join("metadata.json");
    if metadata_json_path.exists() {
        info!("migrating metadata.json to SQLite");
        if let Ok(old_meta) = SessionMetadata::load(metadata_json_path.clone()) {
            let sqlite_meta = SqliteSessionMetadata::new(db_pool.clone());
            for entry in old_meta.list() {
                if let Err(e) = sqlite_meta.upsert(&entry.key, entry.label.clone()).await {
                    tracing::warn!("failed to migrate session {}: {e}", entry.key);
                }
                if entry.model.is_some() {
                    sqlite_meta.set_model(&entry.key, entry.model.clone()).await;
                }
                sqlite_meta.touch(&entry.key, entry.message_count).await;
                if entry.project_id.is_some() {
                    sqlite_meta
                        .set_project_id(&entry.key, entry.project_id.clone())
                        .await;
                }
            }
        }
        let bak = metadata_json_path.with_extension("json.bak");
        std::fs::rename(&metadata_json_path, &bak).ok();
    }

    // Wire stores.
    let project_store: Arc<dyn ProjectStore> =
        Arc::new(moltis_projects::SqliteProjectStore::new(db_pool.clone()));
    let session_store = Arc::new(SessionStore::new(sessions_dir));
    let event_bus_for_metadata = session_event_bus.clone();
    let session_metadata = Arc::new(SqliteSessionMetadata::with_event_bus(
        db_pool.clone(),
        event_bus_for_metadata,
    ));
    let session_share_store = Arc::new(crate::share_store::ShareStore::new(db_pool.clone()));
    let session_state_store = Arc::new(moltis_sessions::state_store::SessionStateStore::new(
        db_pool.clone(),
    ));

    // Wire agent persona store for multi-agent support (created early so onboarding can use it).
    let agent_persona_store = Arc::new(crate::agent_persona::AgentPersonaStore::new(
        db_pool.clone(),
    ));
    if let Err(e) = agent_persona_store.ensure_main_workspace_seeded() {
        tracing::warn!(error = %e, "failed to seed main agent workspace");
    }

    // Deferred reference: populated once GatewayState is ready.
    let deferred_state: Arc<tokio::sync::OnceCell<Arc<GatewayState>>> =
        Arc::new(tokio::sync::OnceCell::new());

    services =
        services.with_onboarding(Arc::new(crate::onboarding::GatewayOnboardingService::new(
            live_onboarding,
            Arc::clone(&session_metadata),
            Arc::clone(&agent_persona_store),
            Arc::clone(&deferred_state),
        )));

    // Session service wired below after sandbox_router is created.

    // Wire live project service.
    services.project = Arc::new(crate::project::LiveProjectService::new(Arc::clone(
        &project_store,
    )));

    // Initialize cron service with file-backed store.
    let cron_store: Arc<dyn moltis_cron::store::CronStore> =
        match moltis_cron::store_file::FileStore::default_path() {
            Ok(fs) => Arc::new(fs),
            Err(e) => {
                tracing::warn!("cron file store unavailable ({e}), using in-memory");
                Arc::new(moltis_cron::store_memory::InMemoryStore::new())
            },
        };

    // System event: inject text into the main session and trigger an agent response.
    let sys_state = Arc::clone(&deferred_state);
    let on_system_event: moltis_cron::service::SystemEventFn = Arc::new(move |text| {
        let st = Arc::clone(&sys_state);
        tokio::spawn(async move {
            if let Some(state) = st.get() {
                let chat = state.chat().await;
                let params = serde_json::json!({ "text": text });
                if let Err(e) = chat.send(params).await {
                    tracing::error!("cron system event failed: {e}");
                }
            }
        });
    });

    // Create the system events queue before the callbacks so it can be shared.
    let events_queue = moltis_cron::system_events::SystemEventsQueue::new();

    // Agent turn: run an LLM turn in a session determined by the job's session_target.
    let agent_state = Arc::clone(&deferred_state);
    let agent_events_queue = Arc::clone(&events_queue);
    let on_agent_turn: moltis_cron::service::AgentTurnFn = Arc::new(move |req| {
        let st = Arc::clone(&agent_state);
        let eq = Arc::clone(&agent_events_queue);
        Box::pin(async move {
            let state = st
                .get()
                .ok_or_else(|| moltis_cron::Error::message("gateway not ready"))?;

            // OpenClaw-style cost guard: if HEARTBEAT.md exists but is effectively
            // empty (comments/blank scaffold) and there's no explicit
            // heartbeat.prompt override, skip the LLM turn entirely.
            let is_heartbeat_turn = matches!(
                &req.session_target,
                moltis_cron::types::SessionTarget::Named(name) if name == "heartbeat"
            );
            // Check for pending system events (used to bypass the empty-content guard).
            let has_pending_events = is_heartbeat_turn && !eq.is_empty().await;
            if is_heartbeat_turn && !has_pending_events {
                let hb_cfg = state.inner.read().await.heartbeat_config.clone();
                let has_prompt_override = hb_cfg
                    .prompt
                    .as_deref()
                    .is_some_and(|p| !p.trim().is_empty());
                let heartbeat_path = moltis_config::heartbeat_path();
                let heartbeat_file_exists = heartbeat_path.exists();
                let heartbeat_md = moltis_config::load_heartbeat_md();
                if heartbeat_file_exists && heartbeat_md.is_none() && !has_prompt_override {
                    tracing::info!(
                        path = %heartbeat_path.display(),
                        "skipping heartbeat LLM turn: HEARTBEAT.md is empty"
                    );
                    return Ok(moltis_cron::service::AgentTurnResult {
                        output: moltis_cron::heartbeat::HEARTBEAT_OK.to_string(),
                        input_tokens: None,
                        output_tokens: None,
                    });
                }
            }

            let chat = state.chat().await;
            let session_key = match &req.session_target {
                moltis_cron::types::SessionTarget::Named(name) => {
                    format!("cron:{name}")
                },
                _ => format!("cron:{}", uuid::Uuid::new_v4()),
            };

            // Clear session history for named cron sessions before execution
            // so the run starts fresh but the history remains readable for debugging.
            if matches!(
                req.session_target,
                moltis_cron::types::SessionTarget::Named(_)
            ) {
                let _ = chat
                    .clear(serde_json::json!({ "_session_key": session_key }))
                    .await;
            }

            // Apply sandbox overrides for this cron session.
            if let Some(ref router) = state.sandbox_router {
                router.set_override(&session_key, req.sandbox.enabled).await;
                if let Some(ref image) = req.sandbox.image {
                    router.set_image_override(&session_key, image.clone()).await;
                } else {
                    router.remove_image_override(&session_key).await;
                }
            }

            let prompt_text = if is_heartbeat_turn {
                let events = eq.drain().await;
                if events.is_empty() {
                    req.message.clone()
                } else {
                    tracing::info!(
                        event_count = events.len(),
                        "enriching heartbeat prompt with system events"
                    );
                    moltis_cron::heartbeat::build_event_enriched_prompt(&events, &req.message)
                }
            } else {
                req.message.clone()
            };

            // When the output will be delivered to a channel, prepend a
            // formatting hint so the LLM produces channel-friendly content.
            let prompt_text = if req.deliver && !is_heartbeat_turn {
                format!(
                    "Your response will be delivered to an external chat channel. \
                     Keep it concise and prefer plain text with minimal formatting.\n\n\
                     {prompt_text}"
                )
            } else {
                prompt_text
            };

            let mut params = serde_json::json!({
                "text": prompt_text,
                "_session_key": session_key,
            });
            if let Some(ref model) = req.model {
                params["model"] = serde_json::Value::String(model.clone());
            }
            let result = chat
                .send_sync(params)
                .await
                .map_err(|e| moltis_cron::Error::message(e.to_string()));

            // Clean up sandbox overrides.
            if let Some(ref router) = state.sandbox_router {
                router.remove_override(&session_key).await;
                router.remove_image_override(&session_key).await;
            }

            let val = result?;
            let input_tokens = val.get("inputTokens").and_then(|v| v.as_u64());
            let output_tokens = val.get("outputTokens").and_then(|v| v.as_u64());
            let text = val
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let delivery_text = if is_heartbeat_turn {
                let hb_cfg = state.inner.read().await.heartbeat_config.clone();
                moltis_cron::heartbeat::strip_heartbeat_token(
                    &text,
                    moltis_cron::heartbeat::StripMode::Trim,
                    hb_cfg.ack_max_chars,
                )
                .text
            } else {
                text.clone()
            };

            // Deliver output to a channel if requested.
            if req.deliver
                && !delivery_text.trim().is_empty()
                && let (Some(channel_account), Some(chat_id)) = (&req.channel, &req.to)
            {
                if let Some(outbound) = state.services.channel_outbound_arc() {
                    if let Err(e) = outbound
                        .send_text(channel_account, chat_id, &delivery_text, None)
                        .await
                    {
                        tracing::warn!(
                            channel = %channel_account,
                            to = %chat_id,
                            error = %e,
                            "cron job channel delivery failed"
                        );
                    }
                } else {
                    tracing::debug!(
                        "cron job delivery requested but no channel outbound configured"
                    );
                }
            }

            Ok(moltis_cron::service::AgentTurnResult {
                output: text,
                input_tokens,
                output_tokens,
            })
        })
    });

    // Build cron notification callback that broadcasts job changes.
    let deferred_for_cron = Arc::clone(&deferred_state);
    let on_cron_notify: moltis_cron::service::NotifyFn =
        Arc::new(move |notification: moltis_cron::types::CronNotification| {
            let state_opt = deferred_for_cron.get();
            let Some(state) = state_opt else {
                return;
            };
            let (event, payload) = match &notification {
                moltis_cron::types::CronNotification::Created { job } => {
                    ("cron.job.created", serde_json::json!({ "job": job }))
                },
                moltis_cron::types::CronNotification::Updated { job } => {
                    ("cron.job.updated", serde_json::json!({ "job": job }))
                },
                moltis_cron::types::CronNotification::Removed { job_id } => {
                    ("cron.job.removed", serde_json::json!({ "jobId": job_id }))
                },
            };
            // Spawn async broadcast in a background task since we're in a sync callback.
            let state = Arc::clone(state);
            tokio::spawn(async move {
                broadcast(&state, event, payload, BroadcastOpts {
                    drop_if_slow: true,
                    ..Default::default()
                })
                .await;
            });
        });

    // Build rate limit config from moltis config.
    let rate_limit_config = moltis_cron::service::RateLimitConfig {
        max_per_window: config.cron.rate_limit_max,
        window_ms: config.cron.rate_limit_window_secs * 1000,
    };

    let cron_service = moltis_cron::service::CronService::with_events_queue(
        cron_store,
        on_system_event,
        on_agent_turn,
        Some(on_cron_notify),
        rate_limit_config,
        events_queue,
    );

    // Wire cron into gateway services.
    let live_cron = Arc::new(crate::cron::LiveCronService::new(Arc::clone(&cron_service)));
    services = services.with_cron(live_cron);

    // Build sandbox router from config (shared across sessions).
    let mut sandbox_config = moltis_tools::sandbox::SandboxConfig::from(&config.tools.exec.sandbox);
    sandbox_config.container_prefix = Some(sandbox_container_prefix);
    sandbox_config.timezone = config
        .user
        .timezone
        .as_ref()
        .map(|tz| tz.name().to_string());
    let sandbox_router = Arc::new(moltis_tools::sandbox::SandboxRouter::new(
        sandbox_config.clone(),
    ));

    // ── Trusted-network proxy + audit ────────────────────────────────────
    #[cfg(feature = "trusted-network")]
    let audit_buffer_for_broadcast: Option<crate::network_audit::NetworkAuditBuffer>;
    #[cfg(feature = "trusted-network")]
    let proxy_url_for_tools: Option<String>;
    #[cfg(feature = "trusted-network")]
    {
        let (audit_tx, audit_rx) =
            tokio::sync::mpsc::channel::<moltis_network_filter::NetworkAuditEntry>(1024);

        info!(
            network_policy = ?sandbox_config.network,
            trusted_domains = ?sandbox_config.trusted_domains,
            "trusted-network: evaluating network policy"
        );

        if sandbox_config.network == moltis_network_filter::NetworkPolicy::Trusted {
            let domain_mgr = Arc::new(
                moltis_network_filter::domain_approval::DomainApprovalManager::new(
                    &sandbox_config.trusted_domains,
                    std::time::Duration::from_secs(30),
                ),
            );
            let proxy_addr: SocketAddr =
                ([0, 0, 0, 0], moltis_network_filter::DEFAULT_PROXY_PORT).into();
            let proxy = moltis_network_filter::proxy::NetworkProxyServer::new(
                proxy_addr,
                Arc::clone(&domain_mgr),
                Some(audit_tx.clone()),
            );
            let (_proxy_shutdown_tx, proxy_shutdown_rx) = tokio::sync::watch::channel(false);
            tokio::spawn(async move {
                if let Err(e) = proxy.run(proxy_shutdown_rx).await {
                    tracing::warn!("network proxy exited: {e}");
                }
            });
            let url = format!(
                "http://127.0.0.1:{}",
                moltis_network_filter::DEFAULT_PROXY_PORT
            );
            info!(
                proxy_url = %url,
                "trusted-network proxy started, routing all HTTP tools through proxy"
            );
            moltis_tools::init_shared_http_client(Some(&url));
            proxy_url_for_tools = Some(url);
        } else {
            info!(
                network_policy = ?sandbox_config.network,
                "trusted-network proxy not started (policy is not Trusted)"
            );
            proxy_url_for_tools = None;
        }

        // Create the live network audit service from the receiver channel.
        let audit_log_path = data_dir.join("network-audit.jsonl");
        let audit_service =
            crate::network_audit::LiveNetworkAuditService::new(audit_rx, audit_log_path, 2048);
        audit_buffer_for_broadcast = Some(audit_service.buffer().clone());
        services = services.with_network_audit(Arc::new(audit_service));
    }

    // Spawn background image pre-build. This bakes configured packages into a
    // container image so container creation is instant. Backends that don't
    // support image building return Ok(None) and the spawn is harmless.
    {
        let router = Arc::clone(&sandbox_router);
        let backend = Arc::clone(router.backend());
        let packages = router.config().packages.clone();
        let base_image = router
            .config()
            .image
            .clone()
            .unwrap_or_else(|| moltis_tools::sandbox::DEFAULT_SANDBOX_IMAGE.to_string());

        if should_prebuild_sandbox_image(router.mode(), &packages) {
            let deferred_for_build = Arc::clone(&deferred_state);
            // Mark the build as in-progress so the UI can show a banner
            // even if the WebSocket broadcast fires before the client connects.
            sandbox_router
                .building_flag
                .store(true, std::sync::atomic::Ordering::Relaxed);
            let build_router = Arc::clone(&sandbox_router);
            tokio::spawn(async move {
                // Broadcast build start event.
                if let Some(state) = deferred_for_build.get() {
                    broadcast(
                        state,
                        "sandbox.image.build",
                        serde_json::json!({ "phase": "start", "packages": packages }),
                        BroadcastOpts {
                            drop_if_slow: true,
                            ..Default::default()
                        },
                    )
                    .await;
                }

                match backend.build_image(&base_image, &packages).await {
                    Ok(Some(result)) => {
                        info!(
                            tag = %result.tag,
                            built = result.built,
                            "sandbox image pre-build complete"
                        );
                        router.set_global_image(Some(result.tag.clone())).await;
                        build_router
                            .building_flag
                            .store(false, std::sync::atomic::Ordering::Relaxed);

                        if let Some(state) = deferred_for_build.get() {
                            broadcast(
                                state,
                                "sandbox.image.build",
                                serde_json::json!({
                                    "phase": "done",
                                    "tag": result.tag,
                                    "built": result.built,
                                }),
                                BroadcastOpts {
                                    drop_if_slow: true,
                                    ..Default::default()
                                },
                            )
                            .await;
                        }
                    },
                    Ok(None) => {
                        debug!(
                            "sandbox image pre-build: no-op (no packages or unsupported backend)"
                        );
                        build_router
                            .building_flag
                            .store(false, std::sync::atomic::Ordering::Relaxed);
                    },
                    Err(e) => {
                        tracing::warn!("sandbox image pre-build failed: {e}");
                        build_router
                            .building_flag
                            .store(false, std::sync::atomic::Ordering::Relaxed);
                        if let Some(state) = deferred_for_build.get() {
                            broadcast(
                                state,
                                "sandbox.image.build",
                                serde_json::json!({
                                    "phase": "error",
                                    "error": e.to_string(),
                                }),
                                BroadcastOpts {
                                    drop_if_slow: true,
                                    ..Default::default()
                                },
                            )
                            .await;
                        }
                    },
                }
            });
        }
    }

    // When no container runtime is available and the host is Debian/Ubuntu,
    // install the configured sandbox packages directly on the host in the background.
    {
        let packages = sandbox_router.config().packages.clone();
        if sandbox_router.backend_name() == "none"
            && !packages.is_empty()
            && moltis_tools::sandbox::is_debian_host()
        {
            let deferred_for_host = Arc::clone(&deferred_state);
            let pkg_count = packages.len();
            tokio::spawn(async move {
                if let Some(state) = deferred_for_host.get() {
                    broadcast(
                        state,
                        "sandbox.host.provision",
                        serde_json::json!({
                            "phase": "start",
                            "count": pkg_count,
                        }),
                        BroadcastOpts {
                            drop_if_slow: true,
                            ..Default::default()
                        },
                    )
                    .await;
                }

                match moltis_tools::sandbox::provision_host_packages(&packages).await {
                    Ok(Some(result)) => {
                        info!(
                            installed = result.installed.len(),
                            skipped = result.skipped.len(),
                            sudo = result.used_sudo,
                            "host package provisioning complete"
                        );
                        if let Some(state) = deferred_for_host.get() {
                            broadcast(
                                state,
                                "sandbox.host.provision",
                                serde_json::json!({
                                    "phase": "done",
                                    "installed": result.installed.len(),
                                    "skipped": result.skipped.len(),
                                }),
                                BroadcastOpts {
                                    drop_if_slow: true,
                                    ..Default::default()
                                },
                            )
                            .await;
                        }
                    },
                    Ok(None) => {
                        debug!("host package provisioning: no-op (not debian or empty packages)");
                    },
                    Err(e) => {
                        warn!("host package provisioning failed: {e}");
                        if let Some(state) = deferred_for_host.get() {
                            broadcast(
                                state,
                                "sandbox.host.provision",
                                serde_json::json!({
                                    "phase": "error",
                                    "error": e.to_string(),
                                }),
                                BroadcastOpts {
                                    drop_if_slow: true,
                                    ..Default::default()
                                },
                            )
                            .await;
                        }
                    },
                }
            });
        }
    }

    // Startup GC: remove orphaned session containers from previous runs.
    // At startup no legitimate sessions exist, so any prefixed containers are stale.
    if sandbox_router.backend_name() != "none" {
        let prefix = sandbox_router.config().container_prefix.clone();
        tokio::spawn(async move {
            if let Some(prefix) = prefix {
                match moltis_tools::sandbox::clean_all_containers(&prefix).await {
                    Ok(0) => {},
                    Ok(n) => info!(
                        removed = n,
                        "startup GC: cleaned orphaned session containers"
                    ),
                    Err(e) => debug!("startup GC: container cleanup skipped: {e}"),
                }
            }
        });
    }

    // Pre-pull browser container image if browser is enabled and sandbox mode is available.
    // Browser sandbox mode follows session sandbox mode, so we pre-pull if sandboxing is available.
    // Don't pre-pull if sandbox is disabled (mode = Off).
    if config.tools.browser.enabled
        && !matches!(
            sandbox_router.config().mode,
            moltis_tools::sandbox::SandboxMode::Off
        )
        && sandbox_router.backend_name() != "none"
    {
        let sandbox_image = config.tools.browser.sandbox_image.clone();
        let deferred_for_browser = Arc::clone(&deferred_state);
        tokio::spawn(async move {
            // Broadcast pull start event.
            if let Some(state) = deferred_for_browser.get() {
                broadcast(
                    state,
                    "browser.image.pull",
                    serde_json::json!({
                        "phase": "start",
                        "image": sandbox_image,
                    }),
                    BroadcastOpts {
                        drop_if_slow: true,
                        ..Default::default()
                    },
                )
                .await;
            }

            match moltis_browser::container::ensure_image(&sandbox_image) {
                Ok(()) => {
                    info!(image = %sandbox_image, "browser container image ready");
                    if let Some(state) = deferred_for_browser.get() {
                        broadcast(
                            state,
                            "browser.image.pull",
                            serde_json::json!({
                                "phase": "done",
                                "image": sandbox_image,
                            }),
                            BroadcastOpts {
                                drop_if_slow: true,
                                ..Default::default()
                            },
                        )
                        .await;
                    }
                },
                Err(e) => {
                    tracing::warn!(image = %sandbox_image, error = %e, "browser container image pull failed");
                    if let Some(state) = deferred_for_browser.get() {
                        broadcast(
                            state,
                            "browser.image.pull",
                            serde_json::json!({
                                "phase": "error",
                                "image": sandbox_image,
                                "error": e.to_string(),
                            }),
                            BroadcastOpts {
                                drop_if_slow: true,
                                ..Default::default()
                            },
                        )
                        .await;
                    }
                },
            }
        });
    }

    // Load any persisted sandbox overrides from session metadata.
    {
        for entry in session_metadata.list().await {
            if let Some(enabled) = entry.sandbox_enabled {
                sandbox_router.set_override(&entry.key, enabled).await;
            }
            if let Some(ref image) = entry.sandbox_image {
                sandbox_router
                    .set_image_override(&entry.key, image.clone())
                    .await;
            }
        }
    }

    // Session service is wired after hook registry is built (below).

    let msteams_webhook_plugin: Arc<tokio::sync::RwLock<moltis_msteams::MsTeamsPlugin>>;
    #[cfg(feature = "slack")]
    let slack_webhook_plugin: Arc<tokio::sync::RwLock<moltis_slack::SlackPlugin>>;

    // Wire channel store, registry, and channel plugins.
    {
        use moltis_channels::{
            registry::{ChannelRegistry, RegistryOutboundRouter},
            store::ChannelStore,
        };

        let channel_store: Arc<dyn ChannelStore> = Arc::new(
            crate::channel_store::SqliteChannelStore::new(db_pool.clone()),
        );

        let channel_sink: Arc<dyn moltis_channels::ChannelEventSink> = Arc::new(
            crate::channel_events::GatewayChannelEventSink::new(Arc::clone(&deferred_state)),
        );

        // Create plugins and register with the registry.
        let mut registry = ChannelRegistry::new();

        let tg_plugin = Arc::new(tokio::sync::RwLock::new(
            moltis_telegram::TelegramPlugin::new()
                .with_message_log(Arc::clone(&message_log))
                .with_event_sink(Arc::clone(&channel_sink)),
        ));
        registry
            .register(tg_plugin as Arc<tokio::sync::RwLock<dyn ChannelPlugin>>)
            .await;

        let msteams_plugin = Arc::new(tokio::sync::RwLock::new(
            moltis_msteams::MsTeamsPlugin::new()
                .with_message_log(Arc::clone(&message_log))
                .with_event_sink(Arc::clone(&channel_sink)),
        ));
        msteams_webhook_plugin = Arc::clone(&msteams_plugin);
        registry
            .register(msteams_plugin as Arc<tokio::sync::RwLock<dyn ChannelPlugin>>)
            .await;

        let discord_plugin = Arc::new(tokio::sync::RwLock::new(
            moltis_discord::DiscordPlugin::new()
                .with_message_log(Arc::clone(&message_log))
                .with_event_sink(Arc::clone(&channel_sink)),
        ));
        registry
            .register(discord_plugin as Arc<tokio::sync::RwLock<dyn ChannelPlugin>>)
            .await;

        #[cfg(feature = "whatsapp")]
        {
            let wa_data_dir = data_dir.join("whatsapp");
            if let Err(e) = std::fs::create_dir_all(&wa_data_dir) {
                tracing::warn!("failed to create whatsapp data dir: {e}");
            }
            let whatsapp_plugin = Arc::new(tokio::sync::RwLock::new(
                moltis_whatsapp::WhatsAppPlugin::new(wa_data_dir)
                    .with_message_log(Arc::clone(&message_log))
                    .with_event_sink(Arc::clone(&channel_sink)),
            ));
            registry
                .register(whatsapp_plugin as Arc<tokio::sync::RwLock<dyn ChannelPlugin>>)
                .await;
        }
        #[cfg(not(feature = "whatsapp"))]
        let _ = &channel_sink; // silence unused warning

        #[cfg(feature = "slack")]
        {
            let slack_plugin = Arc::new(tokio::sync::RwLock::new(
                moltis_slack::SlackPlugin::new()
                    .with_message_log(Arc::clone(&message_log))
                    .with_event_sink(Arc::clone(&channel_sink)),
            ));
            slack_webhook_plugin = Arc::clone(&slack_plugin);
            registry
                .register(slack_plugin as Arc<tokio::sync::RwLock<dyn ChannelPlugin>>)
                .await;
        }

        // Collect all channel accounts to start (config + stored), then
        // spawn them concurrently so slow network calls (e.g. Telegram)
        // don't block startup sequentially.
        let mut pending_starts: Vec<(String, String, serde_json::Value)> = Vec::new();
        let mut queued: HashSet<(String, String)> = HashSet::new();

        for (channel_type, accounts) in config.channels.all_channel_configs() {
            if registry.get(channel_type).is_none() {
                if !accounts.is_empty() {
                    tracing::debug!(
                        channel_type,
                        "skipping config — no plugin registered for this channel type"
                    );
                }
                continue;
            }
            for (account_id, account_config) in accounts {
                let key = (channel_type.to_string(), account_id.clone());
                if queued.insert(key) {
                    pending_starts.push((
                        channel_type.to_string(),
                        account_id.clone(),
                        account_config.clone(),
                    ));
                }
            }
        }

        // Load persisted channels that were not queued from config.
        match channel_store.list().await {
            Ok(stored) => {
                info!("{} stored channel(s) found in database", stored.len());
                for ch in stored {
                    let key = (ch.channel_type.clone(), ch.account_id.clone());
                    if queued.contains(&key) {
                        info!(
                            account_id = ch.account_id,
                            channel_type = ch.channel_type,
                            "skipping stored channel (already started from config)"
                        );
                        continue;
                    }
                    if registry.get(&ch.channel_type).is_none() {
                        tracing::warn!(
                            account_id = ch.account_id,
                            channel_type = ch.channel_type,
                            "unsupported channel type, skipping stored account"
                        );
                        continue;
                    }
                    info!(
                        account_id = ch.account_id,
                        channel_type = ch.channel_type,
                        "starting stored channel"
                    );
                    if queued.insert(key) {
                        pending_starts.push((ch.channel_type, ch.account_id, ch.config));
                    }
                }
            },
            Err(e) => tracing::warn!("failed to load stored channels: {e}"),
        }

        let registry = Arc::new(registry);

        // Spawn all channel starts concurrently.
        if !pending_starts.is_empty() {
            let total = pending_starts.len();
            info!("{total} channel account(s) queued for startup");
            for (channel_type, account_id, account_config) in pending_starts {
                let reg = Arc::clone(&registry);
                tokio::spawn(async move {
                    if let Err(e) = reg
                        .start_account(&channel_type, &account_id, account_config)
                        .await
                    {
                        tracing::warn!(
                            account_id,
                            channel_type,
                            "failed to start channel account: {e}"
                        );
                    } else {
                        info!(account_id, channel_type, "channel account started");
                    }
                });
            }
        }
        let router = Arc::new(RegistryOutboundRouter::new(Arc::clone(&registry)));

        services = services.with_channel_registry(Arc::clone(&registry));
        let outbound_router = Arc::clone(&router) as Arc<dyn moltis_channels::ChannelOutbound>;
        services = services.with_channel_outbound(Arc::clone(&outbound_router));
        services = services.with_channel_stream_outbound(
            router as Arc<dyn moltis_channels::ChannelStreamOutbound>,
        );

        services.channel = Arc::new(crate::channel::LiveChannelService::new(
            registry,
            outbound_router,
            channel_store,
            Arc::clone(&message_log),
            Arc::clone(&session_metadata),
        ));
    }

    services = services.with_session_metadata(Arc::clone(&session_metadata));
    services = services.with_session_store(Arc::clone(&session_store));
    services = services.with_session_share_store(Arc::clone(&session_share_store));

    services = services.with_agent_persona_store(Arc::clone(&agent_persona_store));

    // Shared agents config (presets) — used by both SpawnAgentTool and RPC.
    let agents_config = Arc::new(tokio::sync::RwLock::new(config.agents.clone()));

    // Sync persona identity into presets at startup so spawn_agent sees unified agents.
    {
        let personas = agent_persona_store.list().await;
        if let Ok(personas) = personas {
            let mut guard = agents_config.write().await;
            for persona in &personas {
                if persona.id == "main" {
                    continue;
                }
                sync_persona_into_preset(&mut guard, persona);
            }
        }
    }

    services = services.with_agents_config(Arc::clone(&agents_config));

    // ── Hook discovery & registration ─────────────────────────────────────
    seed_default_workspace_markdown_files();
    seed_example_skill();
    seed_example_hook();
    seed_dcg_guard_hook();
    let persisted_disabled = crate::methods::load_disabled_hooks();
    let (hook_registry, discovered_hooks_info) =
        discover_and_build_hooks(&persisted_disabled, Some(&session_store)).await;

    // Wire live session service with sandbox router, project store, hooks, and browser.
    {
        let mut session_svc =
            LiveSessionService::new(Arc::clone(&session_store), Arc::clone(&session_metadata))
                .with_tts_service(Arc::clone(&services.tts))
                .with_share_store(Arc::clone(&session_share_store))
                .with_sandbox_router(Arc::clone(&sandbox_router))
                .with_agent_persona_store(Arc::clone(&agent_persona_store))
                .with_project_store(Arc::clone(&project_store))
                .with_state_store(Arc::clone(&session_state_store))
                .with_browser_service(Arc::clone(&services.browser));
        if let Some(ref hooks) = hook_registry {
            session_svc = session_svc.with_hooks(Arc::clone(hooks));
        }
        services.session = Arc::new(session_svc);
    }

    // ── Memory system initialization ─────────────────────────────────────
    let memory_manager: Option<Arc<moltis_memory::manager::MemoryManager>> = {
        // Build embedding provider(s) for the fallback chain.
        let mut embedding_providers: Vec<(
            String,
            Box<dyn moltis_memory::embeddings::EmbeddingProvider>,
        )> = Vec::new();

        let mem_cfg = &config.memory;

        if mem_cfg.disable_rag {
            info!("memory: RAG disabled via memory.disable_rag=true, using keyword-only search");
        } else {
            // 1. If user explicitly configured an embedding provider, use it.
            if let Some(ref provider_name) = mem_cfg.provider {
                match provider_name.as_str() {
                    "local" => {
                        // Local GGUF embeddings require the `local-embeddings` feature on moltis-memory.
                        #[cfg(feature = "local-embeddings")]
                        {
                            let cache_dir = mem_cfg
                                .base_url
                                .as_ref()
                                .map(PathBuf::from)
                                .unwrap_or_else(
                                    moltis_memory::embeddings_local::LocalGgufEmbeddingProvider::default_cache_dir,
                                );
                            match moltis_memory::embeddings_local::LocalGgufEmbeddingProvider::ensure_model(
                                cache_dir,
                            )
                            .await
                            {
                                Ok(path) => {
                                    match moltis_memory::embeddings_local::LocalGgufEmbeddingProvider::new(
                                        path,
                                    ) {
                                        Ok(p) => embedding_providers.push(("local-gguf".into(), Box::new(p))),
                                        Err(e) => warn!("memory: failed to load local GGUF model: {e}"),
                                    }
                                },
                                Err(e) => warn!("memory: failed to ensure local model: {e}"),
                            }
                        }
                        #[cfg(not(feature = "local-embeddings"))]
                        warn!(
                            "memory: 'local' embedding provider requires the 'local-embeddings' feature"
                        );
                    },
                    "ollama" | "custom" | "openai" => {
                        let base_url = mem_cfg.base_url.clone().unwrap_or_else(|| {
                            match provider_name.as_str() {
                                "ollama" => "http://localhost:11434".into(),
                                _ => "https://api.openai.com".into(),
                            }
                        });
                        if provider_name == "ollama" {
                            let model = mem_cfg.model.as_deref().unwrap_or("nomic-embed-text");
                            ensure_ollama_model(&base_url, model).await;
                        }
                        let api_key = mem_cfg
                            .api_key
                            .as_ref()
                            .map(|k| k.expose_secret().clone())
                            .or_else(|| {
                                env_value_with_overrides(&runtime_env_overrides, "OPENAI_API_KEY")
                            })
                            .unwrap_or_default();
                        let mut e =
                            moltis_memory::embeddings_openai::OpenAiEmbeddingProvider::new(api_key);
                        if base_url != "https://api.openai.com" {
                            e = e.with_base_url(base_url);
                        }
                        if let Some(ref model) = mem_cfg.model {
                            // Use a sensible default dims; the API returns the actual dims.
                            e = e.with_model(model.clone(), 1536);
                        }
                        embedding_providers.push((provider_name.clone(), Box::new(e)));
                    },
                    other => warn!("memory: unknown embedding provider '{other}'"),
                }
            }

            // 2. Auto-detect: try Ollama health check.
            if embedding_providers.is_empty() {
                let ollama_ok = reqwest::Client::new()
                    .get("http://localhost:11434/api/tags")
                    .timeout(std::time::Duration::from_secs(2))
                    .send()
                    .await
                    .is_ok();
                if ollama_ok {
                    ensure_ollama_model("http://localhost:11434", "nomic-embed-text").await;
                    let e = moltis_memory::embeddings_openai::OpenAiEmbeddingProvider::new(
                        String::new(),
                    )
                    .with_base_url("http://localhost:11434".into())
                    .with_model("nomic-embed-text".into(), 768);
                    embedding_providers.push(("ollama".into(), Box::new(e)));
                    info!("memory: detected Ollama at localhost:11434");
                }
            }

            // 3. Auto-detect: try remote API-key providers.
            const EMBEDDING_CANDIDATES: &[(&str, &str, &str)] = &[
                ("openai", "OPENAI_API_KEY", "https://api.openai.com"),
                ("mistral", "MISTRAL_API_KEY", "https://api.mistral.ai/v1"),
                (
                    "openrouter",
                    "OPENROUTER_API_KEY",
                    "https://openrouter.ai/api/v1",
                ),
                ("groq", "GROQ_API_KEY", "https://api.groq.com/openai"),
                ("xai", "XAI_API_KEY", "https://api.x.ai"),
                ("deepseek", "DEEPSEEK_API_KEY", "https://api.deepseek.com"),
                ("cerebras", "CEREBRAS_API_KEY", "https://api.cerebras.ai/v1"),
                ("minimax", "MINIMAX_API_KEY", "https://api.minimax.io/v1"),
                ("moonshot", "MOONSHOT_API_KEY", "https://api.moonshot.ai/v1"),
                ("venice", "VENICE_API_KEY", "https://api.venice.ai/api/v1"),
            ];

            for (config_name, env_key, default_base) in EMBEDDING_CANDIDATES {
                let key = effective_providers
                    .get(config_name)
                    .and_then(|e| e.api_key.as_ref().map(|k| k.expose_secret().clone()))
                    .or_else(|| env_value_with_overrides(&runtime_env_overrides, env_key))
                    .filter(|k| !k.is_empty());
                if let Some(api_key) = key {
                    let base = effective_providers
                        .get(config_name)
                        .and_then(|e| e.base_url.clone())
                        .unwrap_or_else(|| default_base.to_string());
                    let mut e =
                        moltis_memory::embeddings_openai::OpenAiEmbeddingProvider::new(api_key);
                    if base != "https://api.openai.com" {
                        e = e.with_base_url(base);
                    }
                    embedding_providers.push((config_name.to_string(), Box::new(e)));
                }
            }
        }

        // Build the final embedder: fallback chain, single provider, or keyword-only.
        let embedder: Option<Box<dyn moltis_memory::embeddings::EmbeddingProvider>> = if mem_cfg
            .disable_rag
        {
            None
        } else if embedding_providers.is_empty() {
            info!("memory: no embedding provider found, using keyword-only search");
            None
        } else {
            let names: Vec<&str> = embedding_providers
                .iter()
                .map(|(n, _)| n.as_str())
                .collect();
            if embedding_providers.len() == 1 {
                if let Some((name, provider)) = embedding_providers.into_iter().next() {
                    info!(provider = %name, "memory: using single embedding provider");
                    Some(provider)
                } else {
                    None
                }
            } else {
                info!(providers = ?names, active = names[0], "memory: fallback chain configured");
                Some(Box::new(
                    moltis_memory::embeddings_fallback::FallbackEmbeddingProvider::new(
                        embedding_providers,
                    ),
                ))
            }
        };

        let memory_db_path = data_dir.join("memory.db");
        let memory_pool_result = {
            use {
                sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
                std::str::FromStr,
            };
            let options =
                SqliteConnectOptions::from_str(&format!("sqlite:{}", memory_db_path.display()))
                    .expect("invalid memory database path")
                    .create_if_missing(true)
                    .journal_mode(SqliteJournalMode::Wal)
                    .synchronous(SqliteSynchronous::Normal)
                    .busy_timeout(std::time::Duration::from_secs(5));
            sqlx::pool::PoolOptions::new()
                .max_connections(config.server.db_pool_max_connections)
                .connect_with(options)
                .await
        };
        match memory_pool_result {
            Ok(memory_pool) => {
                if let Err(e) = moltis_memory::schema::run_migrations(&memory_pool).await {
                    tracing::warn!("memory migration failed: {e}");
                    None
                } else {
                    // Scan the data directory for memory files written by the
                    // silent memory turn (MEMORY.md, memory/*.md).
                    let data_memory_file = data_dir.join("MEMORY.md");
                    let data_memory_file_lower = data_dir.join("memory.md");
                    let data_memory_sub = data_dir.join("memory");
                    let agents_root = data_dir.join("agents");

                    let config = moltis_memory::config::MemoryConfig {
                        db_path: memory_db_path.to_string_lossy().into(),
                        data_dir: Some(data_dir.clone()),
                        memory_dirs: vec![
                            data_memory_file,
                            data_memory_file_lower,
                            data_memory_sub,
                            // Include all agent workspaces so per-agent memory writes
                            // remain indexed across periodic full syncs.
                            agents_root,
                        ],
                        ..Default::default()
                    };

                    let store = Box::new(moltis_memory::store_sqlite::SqliteMemoryStore::new(
                        memory_pool,
                    ));
                    // Map file entries to their parent directory so that
                    // root-level files like MEMORY.md are covered by the
                    // watcher. Deduplicate via BTreeSet to avoid watching
                    // the same directory twice.
                    let watch_dirs: Vec<_> = config
                        .memory_dirs
                        .iter()
                        .map(|p| {
                            if p.is_dir() {
                                p.clone()
                            } else {
                                p.parent().unwrap_or(p.as_path()).to_path_buf()
                            }
                        })
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
                        .collect();
                    let manager = Arc::new(if let Some(embedder) = embedder {
                        moltis_memory::manager::MemoryManager::new(config, store, embedder)
                    } else {
                        moltis_memory::manager::MemoryManager::keyword_only(config, store)
                    });

                    // Initial sync + periodic re-sync (15min with watcher, 5min without).
                    let sync_manager = Arc::clone(&manager);
                    tokio::spawn(async move {
                        match sync_manager.sync().await {
                            Ok(report) => {
                                info!(
                                    updated = report.files_updated,
                                    unchanged = report.files_unchanged,
                                    removed = report.files_removed,
                                    errors = report.errors,
                                    cache_hits = report.cache_hits,
                                    cache_misses = report.cache_misses,
                                    "memory: initial sync complete"
                                );
                                match sync_manager.status().await {
                                    Ok(status) => info!(
                                        files = status.total_files,
                                        chunks = status.total_chunks,
                                        db_size = %status.db_size_display(),
                                        model = %status.embedding_model,
                                        "memory: status"
                                    ),
                                    Err(e) => tracing::warn!("memory: failed to get status: {e}"),
                                }
                            },
                            Err(e) => tracing::warn!("memory: initial sync failed: {e}"),
                        }

                        // Start file watcher for real-time sync (if feature enabled).
                        #[cfg(feature = "file-watcher")]
                        {
                            let watcher_manager = Arc::clone(&sync_manager);
                            match moltis_memory::watcher::MemoryFileWatcher::start(watch_dirs) {
                                Ok((_watcher, mut rx)) => {
                                    info!("memory: file watcher started");
                                    tokio::spawn(async move {
                                        while let Some(event) = rx.recv().await {
                                            let path = match &event {
                                                moltis_memory::watcher::WatchEvent::Created(p)
                                                | moltis_memory::watcher::WatchEvent::Modified(p) => {
                                                    Some(p.clone())
                                                },
                                                moltis_memory::watcher::WatchEvent::Removed(p) => {
                                                    // For removed files, trigger a full sync
                                                    if let Err(e) = watcher_manager.sync().await {
                                                        tracing::warn!(
                                                            path = %p.display(),
                                                            error = %e,
                                                            "memory: watcher sync (removal) failed"
                                                        );
                                                    }
                                                    None
                                                },
                                            };
                                            if let Some(path) = path
                                                && let Err(e) =
                                                    watcher_manager.sync_path(&path).await
                                            {
                                                tracing::warn!(
                                                    path = %path.display(),
                                                    error = %e,
                                                    "memory: watcher sync_path failed"
                                                );
                                            }
                                        }
                                    });
                                },
                                Err(e) => {
                                    tracing::warn!("memory: failed to start file watcher: {e}");
                                },
                            }
                        }

                        // Periodic full sync as safety net (longer interval with watcher).
                        #[cfg(feature = "file-watcher")]
                        let interval_secs = 900; // 15 minutes
                        #[cfg(not(feature = "file-watcher"))]
                        let interval_secs = 300; // 5 minutes

                        let mut interval =
                            tokio::time::interval(std::time::Duration::from_secs(interval_secs));
                        interval.tick().await; // skip first immediate tick
                        loop {
                            interval.tick().await;
                            if let Err(e) = sync_manager.sync().await {
                                tracing::warn!("memory: periodic sync failed: {e}");
                            }
                        }
                    });

                    info!(
                        embeddings = manager.has_embeddings(),
                        "memory system initialized"
                    );
                    Some(manager)
                }
            },
            Err(e) => {
                tracing::warn!("memory: failed to open memory.db: {e}");
                None
            },
        }
    };

    let is_localhost =
        matches!(bind, "127.0.0.1" | "::1" | "localhost") || bind.ends_with(".localhost");
    // Initialize metrics system.
    #[cfg(feature = "metrics")]
    let metrics_handle = {
        let metrics_config = moltis_metrics::MetricsRecorderConfig {
            enabled: config.metrics.enabled,
            prefix: None,
            global_labels: vec![
                ("service".to_string(), "moltis-gateway".to_string()),
                ("version".to_string(), env!("CARGO_PKG_VERSION").to_string()),
            ],
        };
        match moltis_metrics::init_metrics(metrics_config) {
            Ok(handle) => {
                if config.metrics.enabled {
                    info!("Metrics collection enabled");
                }
                Some(handle)
            },
            Err(e) => {
                warn!("Failed to initialize metrics: {e}");
                None
            },
        }
    };

    // Initialize metrics store for persistence.
    #[cfg(feature = "metrics")]
    let metrics_store: Option<Arc<dyn crate::state::MetricsStore>> = {
        let metrics_db_path = data_dir.join("metrics.db");
        match moltis_metrics::SqliteMetricsStore::new(&metrics_db_path).await {
            Ok(store) => {
                info!(
                    "Metrics history store initialized at {}",
                    metrics_db_path.display()
                );
                Some(Arc::new(store))
            },
            Err(e) => {
                warn!("Failed to initialize metrics store: {e}");
                None
            },
        }
    };

    // Keep a reference to the browser service for periodic cleanup and shutdown.
    let browser_for_lifecycle = Arc::clone(&services.browser);

    let pairing_store = Arc::new(crate::pairing::PairingStore::new(db_pool.clone()));

    let state = GatewayState::with_options(
        resolved_auth,
        services,
        Some(Arc::clone(&sandbox_router)),
        Some(Arc::clone(&credential_store)),
        Some(pairing_store),
        is_localhost,
        behind_proxy,
        tls_enabled_for_gateway,
        hook_registry.clone(),
        memory_manager.clone(),
        port,
        config.server.ws_request_logs,
        deploy_platform.clone(),
        Some(session_event_bus),
        #[cfg(feature = "metrics")]
        metrics_handle,
        #[cfg(feature = "metrics")]
        metrics_store.clone(),
        #[cfg(feature = "vault")]
        vault.clone(),
    );

    // Store discovered hook info, disabled set, and config overrides in state for the web UI.
    {
        let mut inner = state.inner.write().await;
        inner.discovered_hooks = discovered_hooks_info;
        inner.disabled_hooks = persisted_disabled;
        inner.shiki_cdn_url = config.server.shiki_cdn_url.clone();
        #[cfg(feature = "metrics")]
        {
            inner.metrics_history =
                crate::state::MetricsHistory::new(config.metrics.history_points);
        }
    }

    // Note: LLM provider registry is available through the ChatService,
    // not stored separately in GatewayState.

    // Generate a one-time setup code if setup is pending and auth is not disabled.
    let setup_code_display =
        if !credential_store.is_setup_complete() && !credential_store.is_auth_disabled() {
            let code = std::env::var("MOLTIS_E2E_SETUP_CODE")
                .unwrap_or_else(|_| auth::generate_setup_code());
            state.inner.write().await.setup_code = Some(secrecy::Secret::new(code.clone()));
            Some(code)
        } else {
            None
        };

    // ── Tailscale Serve/Funnel ─────────────────────────────────────────
    #[cfg(feature = "tailscale")]
    let tailscale_mode: TailscaleMode = {
        // CLI flag overrides config file.
        let mode_str = tailscale_opts
            .as_ref()
            .map(|o| o.mode.clone())
            .unwrap_or_else(|| config.tailscale.mode.clone());
        mode_str.parse().unwrap_or(TailscaleMode::Off)
    };
    #[cfg(feature = "tailscale")]
    let tailscale_reset_on_exit = tailscale_opts
        .as_ref()
        .map(|o| o.reset_on_exit)
        .unwrap_or(config.tailscale.reset_on_exit);

    #[cfg(feature = "tailscale")]
    if tailscale_mode != TailscaleMode::Off {
        validate_tailscale_config(tailscale_mode, bind, credential_store.is_setup_complete())?;
    }

    // Populate the deferred reference so cron callbacks can reach the gateway.
    let _ = deferred_state.set(Arc::clone(&state));

    // Set the state on local-llm service for broadcasting download progress.
    #[cfg(feature = "local-llm")]
    if let Some(svc) = &local_llm_service {
        svc.set_state(Arc::clone(&state));
    }

    // Set the broadcaster on provider setup service for validation progress updates.
    provider_setup_service.set_broadcaster(Arc::new(crate::provider_setup::GatewayBroadcaster {
        state: Arc::clone(&state),
    }));

    // Set the state on model service for broadcasting model update events.
    live_model_service.set_state(crate::chat::GatewayChatRuntime::from_state(Arc::clone(
        &state,
    )));

    // Finish startup model discovery in the background, then atomically swap
    // in the fully discovered registry and notify connected clients.
    if startup_discovery_pending.is_empty() {
        debug!("startup model discovery skipped, no pending provider discoveries");
    } else {
        let registry_for_startup_discovery = Arc::clone(&registry);
        let state_for_startup_discovery = Arc::clone(&state);
        let provider_config_for_startup_discovery = effective_providers.clone();
        let env_overrides_for_startup_discovery = config_env_overrides.clone();
        tokio::spawn(async move {
            let startup_discovery_started = std::time::Instant::now();
            let prefetched = match tokio::task::spawn_blocking(move || {
                ProviderRegistry::collect_discoveries(startup_discovery_pending)
            })
            .await
            {
                Ok(prefetched) => prefetched,
                Err(error) => {
                    warn!(
                        error = %error,
                        "startup background model discovery worker failed while collecting results"
                    );
                    return;
                },
            };

            let prefetched_models: usize = prefetched.values().map(Vec::len).sum();
            let new_registry = match tokio::task::spawn_blocking(move || {
                ProviderRegistry::from_config_with_prefetched(
                    &provider_config_for_startup_discovery,
                    &env_overrides_for_startup_discovery,
                    &prefetched,
                )
            })
            .await
            {
                Ok(new_registry) => new_registry,
                Err(error) => {
                    warn!(
                        error = %error,
                        "startup background model discovery worker failed while rebuilding registry"
                    );
                    return;
                },
            };

            let provider_summary = new_registry.provider_summary();
            let model_count = new_registry.list_models().len();
            {
                let mut reg = registry_for_startup_discovery.write().await;
                *reg = new_registry;
            }

            info!(
                provider_summary = %provider_summary,
                models = model_count,
                prefetched_models,
                elapsed_ms = startup_discovery_started.elapsed().as_millis(),
                "startup background model discovery complete, provider registry updated"
            );

            broadcast(
                &state_for_startup_discovery,
                "models.updated",
                serde_json::json!({
                    "reason": "startup-discovery",
                    "models": model_count,
                    "providerSummary": provider_summary,
                }),
                BroadcastOpts::default(),
            )
            .await;
        });
    }

    // Model support probing is triggered on-demand by the web UI when the
    // user opens the model selector (via the `models.detect_supported` RPC).
    // With dynamic model discovery, automatic probing at startup is too
    // expensive and noisy — non-chat models (image, audio, video) would
    // generate spurious warnings.

    // Store heartbeat config and channels offered on state for gon data and RPC methods.
    {
        let mut inner = state.inner.write().await;
        inner.heartbeat_config = config.heartbeat.clone();
        inner.channels_offered = config.channels.offered.clone();
    }
    #[cfg(feature = "graphql")]
    state.set_graphql_enabled(config.graphql.enabled);

    // Wire live chat service (needs state reference, so done after state creation).
    {
        let broadcaster = Arc::new(GatewayApprovalBroadcaster::new(Arc::clone(&state)));
        let env_provider: Arc<dyn EnvVarProvider> = credential_store.clone();
        let eq = cron_service.events_queue().clone();
        let cs = Arc::clone(&cron_service);
        let exec_cb: moltis_tools::exec::ExecCompletionFn = Arc::new(move |event| {
            let summary = format!("Command `{}` exited {}", event.command, event.exit_code);
            let eq = Arc::clone(&eq);
            let cs = Arc::clone(&cs);
            tokio::spawn(async move {
                eq.enqueue(summary, "exec-event".into()).await;
                cs.wake("exec-event").await;
            });
        });
        let mut exec_tool = moltis_tools::exec::ExecTool::default()
            .with_approval(Arc::clone(&approval_manager), broadcaster)
            .with_sandbox_router(Arc::clone(&sandbox_router))
            .with_env_provider(Arc::clone(&env_provider))
            .with_completion_callback(exec_cb);

        // Always attach the node exec provider so the LLM can target nodes
        // via the `node` parameter. When tools.exec.host = "node", also set
        // the default node so commands route there without an explicit param.
        {
            let provider = Arc::new(crate::node_exec::GatewayNodeExecProvider::new(Arc::clone(
                &state,
            )));
            let default_node = if config.tools.exec.host == "node" {
                config.tools.exec.node.clone()
            } else {
                None
            };
            exec_tool = exec_tool.with_node_provider(provider, default_node);
        }

        let cron_tool = moltis_tools::cron_tool::CronTool::new(Arc::clone(&cron_service));

        let mut tool_registry = moltis_agents::tool_registry::ToolRegistry::new();
        let process_tool = moltis_tools::process::ProcessTool::new()
            .with_sandbox_router(Arc::clone(&sandbox_router));

        let sandbox_packages_tool = moltis_tools::sandbox_packages::SandboxPackagesTool::new()
            .with_sandbox_router(Arc::clone(&sandbox_router));

        tool_registry.register(Box::new(exec_tool));
        tool_registry.register(Box::new(moltis_tools::calc::CalcTool::new()));
        #[cfg(feature = "wasm")]
        {
            let wasm_limits = sandbox_router
                .config()
                .wasm_tool_limits
                .clone()
                .unwrap_or_default();
            let epoch_interval_ms = sandbox_router
                .config()
                .wasm_epoch_interval_ms
                .unwrap_or(100);
            let brave_api_key = config
                .tools
                .web
                .search
                .api_key
                .as_ref()
                .map(|s| s.expose_secret().clone())
                .or_else(|| env_value_with_overrides(&runtime_env_overrides, "BRAVE_API_KEY"))
                .filter(|k| !k.trim().is_empty());
            if let Err(e) = moltis_tools::wasm_tool_runner::register_wasm_tools(
                &mut tool_registry,
                &wasm_limits,
                epoch_interval_ms,
                config.tools.web.fetch.timeout_seconds,
                config.tools.web.fetch.cache_ttl_minutes,
                config.tools.web.search.timeout_seconds,
                config.tools.web.search.cache_ttl_minutes,
                brave_api_key.as_deref(),
            ) {
                warn!(%e, "wasm tool registration failed");
            }
        }
        tool_registry.register(Box::new(process_tool));
        tool_registry.register(Box::new(sandbox_packages_tool));
        tool_registry.register(Box::new(cron_tool));
        tool_registry.register(Box::new(crate::channel_agent_tools::SendMessageTool::new(
            Arc::clone(&state.services.channel),
        )));
        tool_registry.register(Box::new(
            moltis_tools::send_image::SendImageTool::new()
                .with_sandbox_router(Arc::clone(&sandbox_router)),
        ));
        if let Some(t) = moltis_tools::web_search::WebSearchTool::from_config_with_env_overrides(
            &config.tools.web.search,
            &runtime_env_overrides,
        ) {
            tool_registry.register(Box::new(t.with_env_provider(Arc::clone(&env_provider))));
        }
        if let Some(t) = moltis_tools::web_fetch::WebFetchTool::from_config(&config.tools.web.fetch)
        {
            #[cfg(feature = "trusted-network")]
            let t = if let Some(ref url) = proxy_url_for_tools {
                t.with_proxy(url.clone())
            } else {
                t
            };
            tool_registry.register(Box::new(t));
        }
        if let Some(t) = moltis_tools::browser::BrowserTool::from_config(&config.tools.browser) {
            let t = if sandbox_router.backend_name() != "none" {
                t.with_sandbox_router(Arc::clone(&sandbox_router))
            } else {
                t
            };
            tool_registry.register(Box::new(t));
        }

        #[cfg(feature = "caldav")]
        {
            if let Some(t) = moltis_caldav::tool::CalDavTool::from_config(&config.caldav) {
                tool_registry.register(Box::new(t));
            }
        }

        // Register memory tools if the memory system is available.
        if let Some(ref mm) = memory_manager {
            tool_registry.register(Box::new(moltis_memory::tools::MemorySearchTool::new(
                Arc::clone(mm),
            )));
            tool_registry.register(Box::new(moltis_memory::tools::MemoryGetTool::new(
                Arc::clone(mm),
            )));
            tool_registry.register(Box::new(moltis_memory::tools::MemorySaveTool::new(
                Arc::clone(mm),
            )));
        }

        // Register node info tools (list, describe, select).
        {
            let node_info_provider: Arc<dyn moltis_tools::nodes::NodeInfoProvider> = Arc::new(
                crate::node_exec::GatewayNodeInfoProvider::new(Arc::clone(&state)),
            );
            tool_registry.register(Box::new(moltis_tools::nodes::NodesListTool::new(
                Arc::clone(&node_info_provider),
            )));
            tool_registry.register(Box::new(moltis_tools::nodes::NodesDescribeTool::new(
                Arc::clone(&node_info_provider),
            )));
            tool_registry.register(Box::new(moltis_tools::nodes::NodesSelectTool::new(
                Arc::clone(&node_info_provider),
            )));
        }

        // Register session state tool for per-session persistent KV store.
        tool_registry.register(Box::new(
            moltis_tools::session_state::SessionStateTool::new(Arc::clone(&session_state_store)),
        ));

        // Register session lifecycle tools for explicit session creation/deletion.
        let state_for_session_create = Arc::clone(&state);
        let metadata_for_session_create = Arc::clone(&session_metadata);
        let create_session: CreateSessionFn = Arc::new(move |req: CreateSessionRequest| {
            let state = Arc::clone(&state_for_session_create);
            let metadata = Arc::clone(&metadata_for_session_create);
            Box::pin(async move {
                let key = req.key;

                let mut resolve_params = serde_json::json!({ "key": key.clone() });
                if let Some(inherit) = req.inherit_agent_from {
                    resolve_params["inherit_agent_from"] = serde_json::json!(inherit);
                }
                state
                    .services
                    .session
                    .resolve(resolve_params)
                    .await
                    .map_err(|e| moltis_tools::Error::message(e.to_string()))?;

                let mut patch = serde_json::Map::new();
                patch.insert("key".to_string(), serde_json::json!(key.clone()));
                if let Some(label) = req.label {
                    patch.insert("label".to_string(), serde_json::json!(label));
                }
                if let Some(model) = req.model {
                    patch.insert("model".to_string(), serde_json::json!(model));
                }
                if let Some(project_id) = req.project_id {
                    patch.insert("projectId".to_string(), serde_json::json!(project_id));
                }
                if patch.len() > 1 {
                    state
                        .services
                        .session
                        .patch(serde_json::Value::Object(patch))
                        .await
                        .map_err(|e| moltis_tools::Error::message(e.to_string()))?;
                }

                let entry = metadata.get(&key).await.ok_or_else(|| {
                    moltis_tools::Error::message(format!("session '{key}' not found after create"))
                })?;
                Ok(serde_json::json!({
                    "entry": {
                        "id": entry.id,
                        "key": entry.key,
                        "label": entry.label,
                        "model": entry.model,
                        "createdAt": entry.created_at,
                        "updatedAt": entry.updated_at,
                        "messageCount": entry.message_count,
                        "projectId": entry.project_id,
                        "agent_id": entry.agent_id,
                        "agentId": entry.agent_id,
                        "version": entry.version,
                    }
                }))
            })
        });

        let state_for_session_delete = Arc::clone(&state);
        let delete_session: DeleteSessionFn = Arc::new(move |req: DeleteSessionRequest| {
            let state = Arc::clone(&state_for_session_delete);
            Box::pin(async move {
                state
                    .services
                    .session
                    .delete(serde_json::json!({
                        "key": req.key,
                        "force": req.force,
                    }))
                    .await
                    .map_err(|e| moltis_tools::Error::message(e.to_string()))
            })
        });

        tool_registry.register(Box::new(SessionsCreateTool::new(
            Arc::clone(&session_metadata),
            create_session,
        )));
        tool_registry.register(Box::new(SessionsDeleteTool::new(
            Arc::clone(&session_metadata),
            delete_session,
        )));

        // Register cross-session communication tools.
        tool_registry.register(Box::new(SessionsListTool::new(Arc::clone(
            &session_metadata,
        ))));
        tool_registry.register(Box::new(SessionsHistoryTool::new(
            Arc::clone(&session_store),
            Arc::clone(&session_metadata),
        )));

        let state_for_session_send = Arc::clone(&state);
        let send_to_session: SendToSessionFn = Arc::new(move |req: SendToSessionRequest| {
            let state = Arc::clone(&state_for_session_send);
            Box::pin(async move {
                let mut params = serde_json::json!({
                    "text": req.message,
                    "_session_key": req.key,
                });
                if let Some(model) = req.model {
                    params["model"] = serde_json::json!(model);
                }
                let chat = state.chat().await;
                if req.wait_for_reply {
                    chat.send_sync(params)
                        .await
                        .map_err(|e| moltis_tools::Error::message(e.to_string()))
                } else {
                    chat.send(params)
                        .await
                        .map_err(|e| moltis_tools::Error::message(e.to_string()))
                }
            })
        });
        tool_registry.register(Box::new(SessionsSendTool::new(
            Arc::clone(&session_metadata),
            send_to_session,
        )));

        // Register shared task coordination tool for multi-agent workflows.
        tool_registry.register(Box::new(moltis_tools::task_list::TaskListTool::new(
            &data_dir,
        )));

        // Register built-in voice tools for explicit TTS/STT calls in agents.
        tool_registry.register(Box::new(crate::voice_agent_tools::SpeakTool::new(
            Arc::clone(&state.services.tts),
        )));
        tool_registry.register(Box::new(crate::voice_agent_tools::TranscribeTool::new(
            Arc::clone(&state.services.stt),
        )));

        // Register skill management tools for agent self-extension.
        // Use data_dir so created skills land in the configured workspace root.
        {
            tool_registry.register(Box::new(moltis_tools::skill_tools::CreateSkillTool::new(
                data_dir.clone(),
            )));
            tool_registry.register(Box::new(moltis_tools::skill_tools::UpdateSkillTool::new(
                data_dir.clone(),
            )));
            tool_registry.register(Box::new(moltis_tools::skill_tools::DeleteSkillTool::new(
                data_dir.clone(),
            )));
        }

        // Register branch session tool for session forking.
        tool_registry.register(Box::new(
            moltis_tools::branch_session::BranchSessionTool::new(
                Arc::clone(&session_store),
                Arc::clone(&session_metadata),
            ),
        ));

        // Register location tool for browser geolocation requests.
        let location_requester = Arc::new(GatewayLocationRequester {
            state: Arc::clone(&state),
        });
        tool_registry.register(Box::new(moltis_tools::location::LocationTool::new(
            location_requester,
        )));

        // Register map tool for showing static map images with links.
        let map_provider = match config.tools.maps.provider {
            moltis_config::schema::MapProvider::GoogleMaps => {
                moltis_tools::map::MapProvider::GoogleMaps
            },
            moltis_config::schema::MapProvider::AppleMaps => {
                moltis_tools::map::MapProvider::AppleMaps
            },
            moltis_config::schema::MapProvider::OpenStreetMap => {
                moltis_tools::map::MapProvider::OpenStreetMap
            },
        };
        tool_registry.register(Box::new(moltis_tools::map::ShowMapTool::with_provider(
            map_provider,
        )));

        // Register spawn_agent tool for sub-agent support.
        // The tool gets a snapshot of the current registry (without itself)
        // so sub-agents have access to all other tools.
        if let Some(default_provider) = registry.read().await.first_with_tools() {
            let base_tools = Arc::new(tool_registry.clone_without(&[]));
            let state_for_spawn = Arc::clone(&state);
            let on_spawn_event: moltis_tools::spawn_agent::OnSpawnEvent = Arc::new(move |event| {
                use moltis_agents::runner::RunnerEvent;
                let state = Arc::clone(&state_for_spawn);
                let payload = match &event {
                    RunnerEvent::SubAgentStart { task, model, depth } => {
                        serde_json::json!({
                            "state": "sub_agent_start",
                            "task": task,
                            "model": model,
                            "depth": depth,
                        })
                    },
                    RunnerEvent::SubAgentEnd {
                        task,
                        model,
                        depth,
                        iterations,
                        tool_calls_made,
                    } => serde_json::json!({
                        "state": "sub_agent_end",
                        "task": task,
                        "model": model,
                        "depth": depth,
                        "iterations": iterations,
                        "toolCallsMade": tool_calls_made,
                    }),
                    _ => return, // Only broadcast sub-agent lifecycle events.
                };
                tokio::spawn(async move {
                    broadcast(&state, "chat", payload, BroadcastOpts::default()).await;
                });
            });
            let spawn_tool = moltis_tools::spawn_agent::SpawnAgentTool::new(
                Arc::clone(&registry),
                default_provider,
                base_tools,
            )
            .with_on_event(on_spawn_event)
            .with_agents_config(agents_config);
            tool_registry.register(Box::new(spawn_tool));
        }

        let shared_tool_registry = Arc::new(tokio::sync::RwLock::new(tool_registry));
        let mut chat_service = LiveChatService::new(
            Arc::clone(&registry),
            Arc::clone(&model_store),
            crate::chat::GatewayChatRuntime::from_state(Arc::clone(&state)),
            Arc::clone(&session_store),
            Arc::clone(&session_metadata),
        )
        .with_tools(Arc::clone(&shared_tool_registry))
        .with_failover(config.failover.clone());

        if let Some(ref hooks) = state.inner.read().await.hook_registry {
            chat_service = chat_service.with_hooks_arc(Arc::clone(hooks));
        }

        let live_chat = Arc::new(chat_service);
        state.set_chat(live_chat).await;

        // Store registry in the MCP service so runtime mutations auto-sync,
        // and do an initial sync for any servers that already started.
        live_mcp
            .set_tool_registry(Arc::clone(&shared_tool_registry))
            .await;
        crate::mcp_service::sync_mcp_tools(live_mcp.manager(), &shared_tool_registry).await;

        // Log registered tools for debugging.
        let schemas = shared_tool_registry.read().await.list_schemas();
        let tool_names: Vec<&str> = schemas.iter().filter_map(|s| s["name"].as_str()).collect();
        info!(tools = ?tool_names, "agent tools registered");
    }

    // Spawn skill file watcher for hot-reload.
    #[cfg(feature = "file-watcher")]
    {
        let search_paths = moltis_skills::discover::FsSkillDiscoverer::default_paths();
        let watch_dirs: Vec<PathBuf> = search_paths.into_iter().map(|(p, _)| p).collect();
        if let Ok((_watcher, mut rx)) = moltis_skills::watcher::SkillWatcher::start(watch_dirs) {
            let watcher_state = Arc::clone(&state);
            tokio::spawn(async move {
                let _watcher = _watcher; // keep alive
                while let Some(_event) = rx.recv().await {
                    broadcast(
                        &watcher_state,
                        "skills.changed",
                        serde_json::json!({}),
                        BroadcastOpts::default(),
                    )
                    .await;
                }
            });
        }
    }

    // Spawn MCP health polling + auto-restart background task.
    {
        let health_state = Arc::clone(&state);
        let health_mcp = Arc::clone(&live_mcp);
        tokio::spawn(async move {
            crate::mcp_health::run_health_monitor(health_state, health_mcp).await;
        });
    }

    let methods = Arc::new(MethodRegistry::new());

    // Initialize push notification service if the feature is enabled.
    #[cfg(feature = "push-notifications")]
    let push_service: Option<Arc<crate::push::PushService>> = {
        match crate::push::PushService::new(&data_dir).await {
            Ok(svc) => {
                info!("push notification service initialized");
                // Store in GatewayState for use by chat service
                state.set_push_service(Arc::clone(&svc)).await;
                Some(svc)
            },
            Err(e) => {
                tracing::warn!("failed to initialize push notification service: {e}");
                None
            },
        }
    };

    Ok(PreparedGatewayCore {
        state: Arc::clone(&state),
        methods: Arc::clone(&methods),
        webauthn_registry,
        msteams_webhook_plugin,
        #[cfg(feature = "slack")]
        slack_webhook_plugin,
        #[cfg(feature = "push-notifications")]
        push_service,
        #[cfg(feature = "trusted-network")]
        audit_buffer: audit_buffer_for_broadcast,
        sandbox_router,
        browser_for_lifecycle,
        cron_service,
        log_buffer,
        config,
        data_dir,
        provider_summary,
        mcp_configured_count,
        openclaw_status: openclaw_startup_status,
        setup_code_display,
        port,
        tls_enabled: tls_enabled_for_gateway,
        #[cfg(feature = "tailscale")]
        tailscale_mode,
        #[cfg(feature = "tailscale")]
        tailscale_reset_on_exit,
    })
}

// ── Hook discovery helper ────────────────────────────────────────────────────

/// Metadata for built-in hooks (compiled Rust, always active).
/// Returns `(name, description, events, source_file)` tuples.
fn builtin_hook_metadata() -> Vec<(
    &'static str,
    &'static str,
    Vec<moltis_common::hooks::HookEvent>,
    &'static str,
)> {
    use moltis_common::hooks::HookEvent;
    vec![
        (
            "boot-md",
            "Reads BOOT.md from the workspace on startup and injects its content as the initial user message to the agent.",
            vec![HookEvent::GatewayStart],
            "crates/plugins/src/bundled/boot_md.rs",
        ),
        (
            "command-logger",
            "Logs all slash-command invocations to a JSONL audit file at ~/.moltis/logs/commands.log.",
            vec![HookEvent::Command],
            "crates/plugins/src/bundled/command_logger.rs",
        ),
        (
            "session-memory",
            "Saves the conversation history to a markdown file in the memory directory when a session is reset or a new session is created, making it searchable for future sessions.",
            vec![HookEvent::Command],
            "crates/plugins/src/bundled/session_memory.rs",
        ),
    ]
}

/// Seed a skeleton example hook into `~/.moltis/hooks/example/` on first run.
///
/// The hook has no command, so it won't execute — it's a template showing
/// users what's possible. If the directory already exists it's a no-op.
fn seed_example_hook() {
    let hook_dir = moltis_config::data_dir().join("hooks/example");
    let hook_md = hook_dir.join("HOOK.md");
    if hook_md.exists() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&hook_dir) {
        tracing::debug!("could not create example hook dir: {e}");
        return;
    }
    if let Err(e) = std::fs::write(&hook_md, EXAMPLE_HOOK_MD) {
        tracing::debug!("could not write example HOOK.md: {e}");
    }
}

/// Seed the `dcg-guard` hook into `~/.moltis/hooks/dcg-guard/` on first run.
///
/// Writes both `HOOK.md` and `handler.sh`. The handler gracefully no-ops when
/// `dcg` is not installed, so the hook is always eligible.
fn seed_dcg_guard_hook() {
    let hook_dir = moltis_config::data_dir().join("hooks/dcg-guard");
    let hook_md = hook_dir.join("HOOK.md");
    if hook_md.exists() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&hook_dir) {
        tracing::debug!("could not create dcg-guard hook dir: {e}");
        return;
    }
    if let Err(e) = std::fs::write(&hook_md, DCG_GUARD_HOOK_MD) {
        tracing::debug!("could not write dcg-guard HOOK.md: {e}");
    }
    let handler = hook_dir.join("handler.sh");
    if let Err(e) = std::fs::write(&handler, DCG_GUARD_HANDLER_SH) {
        tracing::debug!("could not write dcg-guard handler.sh: {e}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&handler, std::fs::Permissions::from_mode(0o755));
    }
}

/// Seed built-in personal skills into `~/.moltis/skills/`.
///
/// These are safe defaults shipped with the binary. Existing user content
/// is never overwritten.
fn seed_example_skill() {
    seed_skill_if_missing("template-skill", EXAMPLE_SKILL_MD);
    seed_skill_if_missing("tmux", TMUX_SKILL_MD);
}

/// Write a skill's `SKILL.md` into `<data_dir>/skills/<name>/` if it doesn't
/// already exist.
fn seed_skill_if_missing(name: &str, content: &str) {
    let skill_dir = moltis_config::data_dir().join(format!("skills/{name}"));
    let skill_md = skill_dir.join("SKILL.md");
    if skill_md.exists() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&skill_dir) {
        tracing::debug!("could not create {name} skill dir: {e}");
        return;
    }
    if let Err(e) = std::fs::write(&skill_md, content) {
        tracing::debug!("could not write {name} SKILL.md: {e}");
    }
}

/// Merge a persona's identity into an `AgentsConfig` preset entry.
///
/// If a preset already exists for this persona, identity fields from the persona
/// take precedence (name/emoji/theme) while TOML-defined fields (model, tools,
/// timeout, etc.) are preserved. The soul is synced into `system_prompt_suffix`.
pub(crate) fn sync_persona_into_preset(
    agents: &mut moltis_config::AgentsConfig,
    persona: &crate::agent_persona::AgentPersona,
) {
    let soul = moltis_config::load_soul_for_agent(&persona.id);

    let entry = agents.presets.entry(persona.id.clone()).or_default();

    // Persona identity always wins for name/emoji/theme.
    entry.identity.name = Some(persona.name.clone());
    entry.identity.emoji = persona.emoji.clone();
    entry.identity.theme = persona.theme.clone();

    // Sync soul into system_prompt_suffix if the persona has one.
    if let Some(ref soul) = soul
        && !soul.trim().is_empty()
    {
        entry.system_prompt_suffix = Some(soul.clone());
    }
}

/// Seed default workspace markdown files in workspace root on first run.
fn seed_default_workspace_markdown_files() {
    let data_dir = moltis_config::data_dir();
    seed_file_if_missing(data_dir.join("BOOT.md"), DEFAULT_BOOT_MD);
    seed_file_if_missing(data_dir.join("AGENTS.md"), DEFAULT_WORKSPACE_AGENTS_MD);
    seed_file_if_missing(data_dir.join("TOOLS.md"), DEFAULT_TOOLS_MD);
    seed_file_if_missing(data_dir.join("HEARTBEAT.md"), DEFAULT_HEARTBEAT_MD);
}

fn seed_file_if_missing(path: PathBuf, content: &str) {
    if path.exists() {
        return;
    }
    if let Err(e) = std::fs::write(&path, content) {
        tracing::debug!(path = %path.display(), "could not write default markdown file: {e}");
    }
}

/// Content for the skeleton example hook.
const EXAMPLE_HOOK_MD: &str = r#"+++
name = "example"
description = "Skeleton hook — edit this to build your own"
emoji = "🪝"
events = ["BeforeToolCall"]
# command = "./handler.sh"
# timeout = 10
# priority = 0

# [requires]
# os = ["darwin", "linux"]
# bins = ["jq", "curl"]
# env = ["SLACK_WEBHOOK_URL"]
+++

# Example Hook

This is a skeleton hook to help you get started. It subscribes to
`BeforeToolCall` but has no `command`, so it won't execute anything.

## Quick start

1. Uncomment the `command` line above and point it at your script
2. Create `handler.sh` (or any executable) in this directory
3. Click **Reload** in the Hooks UI (or restart moltis)

## How hooks work

Your script receives the event payload as **JSON on stdin** and communicates
its decision via **exit code** and **stdout**:

| Exit code | Stdout | Action |
|-----------|--------|--------|
| 0 | *(empty)* | **Continue** — let the action proceed |
| 0 | `{"action":"modify","data":{...}}` | **Modify** — alter the payload |
| 1 | *(stderr used as reason)* | **Block** — prevent the action |

## Example handler (bash)

```bash
#!/usr/bin/env bash
# handler.sh — log every tool call to a file
payload=$(cat)
tool=$(echo "$payload" | jq -r '.tool_name // "unknown"')
echo "$(date -Iseconds) tool=$tool" >> /tmp/moltis-hook.log
# Exit 0 with no stdout = Continue
```

## Available events

**Can modify or block (sequential dispatch):**
- `BeforeAgentStart` — before a new agent run begins
- `BeforeToolCall` — before executing a tool (inspect/modify arguments)
- `BeforeCompaction` — before compacting chat history
- `MessageSending` — before sending a message to the LLM
- `ToolResultPersist` — before persisting a tool result

**Read-only (parallel dispatch, Block/Modify ignored):**
- `AgentEnd` — after an agent run completes
- `AfterToolCall` — after a tool finishes (observe result)
- `AfterCompaction` — after compaction completes
- `MessageReceived` — after receiving an LLM response
- `MessageSent` — after a message is sent
- `SessionStart` / `SessionEnd` — session lifecycle
- `GatewayStart` / `GatewayStop` — server lifecycle

## Frontmatter reference

```toml
name = "my-hook"           # unique identifier
description = "What it does"
emoji = "🔧"               # optional, shown in UI
events = ["BeforeToolCall"] # which events to subscribe to
command = "./handler.sh"    # script to run (relative to this dir)
timeout = 10                # seconds before kill (default: 10)
priority = 0                # higher runs first (default: 0)

[requires]
os = ["darwin", "linux"]    # skip on other OSes
bins = ["jq"]               # required binaries in PATH
env = ["MY_API_KEY"]        # required environment variables
```
"#;

/// Content for the seeded dcg-guard hook manifest.
const DCG_GUARD_HOOK_MD: &str = r#"+++
name = "dcg-guard"
description = "Blocks destructive commands using Destructive Command Guard (dcg)"
emoji = "🛡️"
events = ["BeforeToolCall"]
command = "./handler.sh"
timeout = 5
+++

# Destructive Command Guard (dcg)

Uses the external [dcg](https://github.com/Dicklesworthstone/destructive_command_guard)
tool to scan shell commands before execution. dcg ships 49+ pattern categories
covering filesystem, git, database, cloud, and infrastructure commands.

This hook is **seeded by default** into `~/.moltis/hooks/dcg-guard/` on first
run. When `dcg` is not installed the hook is a no-op (all commands pass through).

## Install dcg

```bash
cargo install dcg
```

Once installed, the hook will automatically start guarding destructive commands
on the next Moltis restart.
"#;

/// Content for the seeded dcg-guard handler script.
const DCG_GUARD_HANDLER_SH: &str = r#"#!/usr/bin/env bash
# Hook handler: translates Moltis BeforeToolCall payload to dcg format.
# When dcg is not installed the hook is a no-op (all commands pass through).

set -euo pipefail

# Gracefully skip when dcg is not installed.
if ! command -v dcg >/dev/null 2>&1; then
    cat >/dev/null   # drain stdin
    exit 0
fi

INPUT=$(cat)

# Only inspect exec tool calls.
TOOL_NAME=$(printf '%s' "$INPUT" | grep -o '"tool_name":"[^"]*"' | head -1 | cut -d'"' -f4)
if [ "$TOOL_NAME" != "exec" ]; then
    exit 0
fi

# Extract the command string from the arguments object.
COMMAND=$(printf '%s' "$INPUT" | grep -o '"command":"[^"]*"' | head -1 | cut -d'"' -f4)
if [ -z "$COMMAND" ]; then
    exit 0
fi

# Build the payload dcg expects and pipe it in.
DCG_INPUT=$(printf '{"tool_name":"Bash","tool_input":{"command":"%s"}}' "$COMMAND")
DCG_RESULT=$(printf '%s' "$DCG_INPUT" | dcg 2>&1) || {
    # dcg returned non-zero — command is destructive.
    echo "$DCG_RESULT" >&2
    exit 1
}

# dcg returned 0 — command is safe.
exit 0
"#;

/// Content for the starter example personal skill.
const EXAMPLE_SKILL_MD: &str = r#"---
name: template-skill
description: Starter skill template (safe to copy and edit)
---

# Template Skill

Use this as a starting point for your own skills.

## How to use

1. Copy this folder to a new skill name (or edit in place)
2. Update `name` and `description` in frontmatter
3. Replace this body with clear, specific instructions

## Tips

- Keep instructions explicit and task-focused
- Avoid broad permissions unless required
- Document required tools and expected inputs
"#;

/// Content for the built-in tmux skill (interactive terminal processes).
const TMUX_SKILL_MD: &str = r#"---
name: tmux
description: Run and interact with terminal applications (htop, vim, etc.) using tmux sessions in the sandbox
allowed-tools:
  - process
---

# tmux — Interactive Terminal Sessions

Use the `process` tool to run and interact with interactive or long-running
programs inside the sandbox. Every command runs in a named **tmux session**,
giving you full control over TUI apps, REPLs, and background processes.

## When to use this skill

- **TUI / ncurses apps**: htop, vim, nano, less, top, iftop
- **Interactive REPLs**: python3, node, irb, psql, sqlite3
- **Long-running commands**: tail -f, watch, servers, builds
- **Programs that need keyboard input**: anything that waits for keypresses

For simple one-shot commands (ls, cat, echo), use `exec` instead.

## Workflow

1. **Start** a session with a command
2. **Poll** to see the current terminal output
3. **Send keys** or **paste text** to interact
4. **Poll** again to see the result
5. **Kill** when done

Always poll after sending keys — the terminal updates asynchronously.

## Actions

### start — Launch a program

```json
{"action": "start", "command": "htop", "session_name": "my-htop"}
```

- `session_name` is optional (auto-generated if omitted)
- The command runs in a 200x50 terminal

### poll — Read terminal output

```json
{"action": "poll", "session_name": "my-htop"}
```

Returns the visible pane content (what a user would see on screen).

### send_keys — Send keystrokes

```json
{"action": "send_keys", "session_name": "my-htop", "keys": "q"}
```

Common key names:
- `Enter`, `Escape`, `Tab`, `Space`
- `Up`, `Down`, `Left`, `Right`
- `C-c` (Ctrl+C), `C-d` (Ctrl+D), `C-z` (Ctrl+Z)
- `C-l` (clear screen), `C-a` / `C-e` (line start/end)
- Single characters: `q`, `y`, `n`, `/`

### paste — Insert text

```json
{"action": "paste", "session_name": "repl", "text": "print('hello world')\n"}
```

Use paste for multi-character input (code, file content). For single
keystrokes, prefer `send_keys`.

### kill — End a session

```json
{"action": "kill", "session_name": "my-htop"}
```

### list — Show active sessions

```json
{"action": "list"}
```

## Examples

### Run htop and report system load

1. `start` with `"command": "htop"`
2. `poll` to capture the htop display
3. Summarize CPU/memory usage from the output
4. `send_keys` with `"keys": "q"` to quit
5. `kill` the session

### Interactive Python REPL

1. `start` with `"command": "python3"`
2. `paste` with `"text": "2 + 2\n"`
3. `poll` to see the result
4. `send_keys` with `"keys": "C-d"` to exit

### Watch a log file

1. `start` with `"command": "tail -f /var/log/syslog"`, `"session_name": "logs"`
2. `poll` periodically to read new lines
3. `send_keys` with `"keys": "C-c"` when done
4. `kill` the session

## Tips

- Session names must be `[a-zA-Z0-9_-]` only (no spaces or special chars)
- Always `kill` sessions when done to free resources
- If a program is unresponsive, `send_keys` with `C-c` or `C-\` first
- Poll output is a snapshot; poll again for updates after sending input
"#;

/// Default BOOT.md content seeded into workspace root.
const DEFAULT_BOOT_MD: &str = r#"<!--
BOOT.md is optional startup context.

How Moltis uses this file:
- Read on every GatewayStart by the built-in boot-md hook.
- Missing/empty/comment-only file = no startup injection.
- Non-empty content = injected as startup user message context.

Recommended usage:
- Keep it short and explicit.
- Use for startup checks/reminders, not onboarding identity setup.
-->"#;

/// Default workspace AGENTS.md content seeded into workspace root.
const DEFAULT_WORKSPACE_AGENTS_MD: &str = r#"<!--
Workspace AGENTS.md contains global instructions for this workspace.

How Moltis uses this file:
- Loaded from data_dir/AGENTS.md when present.
- Injected as workspace context in the system prompt.
- Separate from project AGENTS.md/CLAUDE.md discovery.

Use this for cross-project rules that should apply everywhere in this workspace.
-->"#;

/// Default TOOLS.md content seeded into workspace root.
const DEFAULT_TOOLS_MD: &str = r#"<!--
TOOLS.md contains workspace-specific tool notes and constraints.

How Moltis uses this file:
- Loaded from data_dir/TOOLS.md when present.
- Injected as workspace context in the system prompt.

Use this for local setup details (hosts, aliases, device names) and
tool behavior constraints (safe defaults, forbidden actions, etc.).
-->"#;

/// Default HEARTBEAT.md content seeded into workspace root.
const DEFAULT_HEARTBEAT_MD: &str = r#"<!--
HEARTBEAT.md is an optional heartbeat prompt source.

Prompt precedence:
1) heartbeat.prompt from config
2) HEARTBEAT.md
3) built-in default prompt

Cost guard:
- If HEARTBEAT.md exists but is empty/comment-only and there is no explicit
  heartbeat.prompt override, Moltis skips heartbeat LLM turns to avoid token use.
-->"#;

/// Discover hooks from the filesystem, check eligibility, and build a
/// [`HookRegistry`] plus a `Vec<DiscoveredHookInfo>` for the web UI.
///
/// Hooks whose names appear in `disabled` are still returned in the info list
/// (with `enabled: false`) but are not registered in the registry.
pub(crate) async fn discover_and_build_hooks(
    disabled: &HashSet<String>,
    session_store: Option<&Arc<SessionStore>>,
) -> (
    Option<Arc<moltis_common::hooks::HookRegistry>>,
    Vec<crate::state::DiscoveredHookInfo>,
) {
    use moltis_plugins::{
        bundled::{
            boot_md::BootMdHook, command_logger::CommandLoggerHook,
            session_memory::SessionMemoryHook,
        },
        hook_discovery::{FsHookDiscoverer, HookDiscoverer, HookSource},
        hook_eligibility::check_hook_eligibility,
        shell_hook::ShellHookHandler,
    };

    let discoverer = FsHookDiscoverer::new(FsHookDiscoverer::default_paths());
    let discovered = discoverer.discover().await.unwrap_or_default();

    let mut registry = moltis_common::hooks::HookRegistry::new();
    let mut info_list = Vec::with_capacity(discovered.len());

    for (parsed, source) in &discovered {
        let meta = &parsed.metadata;
        let elig = check_hook_eligibility(meta);
        let is_disabled = disabled.contains(&meta.name);
        let is_enabled = elig.eligible && !is_disabled;

        if !elig.eligible {
            info!(
                hook = %meta.name,
                source = ?source,
                missing_os = elig.missing_os,
                missing_bins = ?elig.missing_bins,
                missing_env = ?elig.missing_env,
                "hook ineligible, skipping"
            );
        }

        // Read the raw HOOK.md content for the UI editor.
        let raw_content =
            std::fs::read_to_string(parsed.source_path.join("HOOK.md")).unwrap_or_default();

        let source_str = match source {
            HookSource::Project => "project",
            HookSource::User => "user",
            HookSource::Bundled => "bundled",
        };

        info_list.push(crate::state::DiscoveredHookInfo {
            name: meta.name.clone(),
            description: meta.description.clone(),
            emoji: meta.emoji.clone(),
            events: meta.events.iter().map(|e| e.to_string()).collect(),
            command: meta.command.clone(),
            timeout: meta.timeout,
            priority: meta.priority,
            source: source_str.to_string(),
            source_path: parsed.source_path.display().to_string(),
            eligible: elig.eligible,
            missing_os: elig.missing_os,
            missing_bins: elig.missing_bins.clone(),
            missing_env: elig.missing_env.clone(),
            enabled: is_enabled,
            body: raw_content,
            body_html: crate::services::markdown_to_html(&parsed.body),
            call_count: 0,
            failure_count: 0,
            avg_latency_ms: 0,
        });

        // Only register eligible, non-disabled hooks.
        if is_enabled && let Some(ref command) = meta.command {
            let handler = ShellHookHandler::new(
                meta.name.clone(),
                command.clone(),
                meta.events.clone(),
                std::time::Duration::from_secs(meta.timeout),
                meta.env.clone(),
                Some(parsed.source_path.clone()),
            );
            registry.register(Arc::new(handler));
        }
    }

    // ── Built-in hooks (compiled Rust, always active) ──────────────────
    {
        let data = moltis_config::data_dir();

        // boot-md: inject BOOT.md content on GatewayStart.
        let boot = BootMdHook::new(data.clone());
        registry.register(Arc::new(boot));

        // command-logger: append JSONL entries for every slash command.
        let log_path =
            CommandLoggerHook::default_path().unwrap_or_else(|| data.join("logs/commands.log"));
        let logger = CommandLoggerHook::new(log_path);
        registry.register(Arc::new(logger));

        // session-memory: save conversation to memory on /new or /reset.
        if let Some(store) = session_store {
            let memory_hook = SessionMemoryHook::new(data.clone(), Arc::clone(store));
            registry.register(Arc::new(memory_hook));
        }
    }

    for (name, description, events, source_file) in builtin_hook_metadata() {
        info_list.push(crate::state::DiscoveredHookInfo {
            name: name.to_string(),
            description: description.to_string(),
            emoji: Some("\u{2699}\u{fe0f}".to_string()), // ⚙️
            events: events.iter().map(|e| e.to_string()).collect(),
            command: None,
            timeout: 0,
            priority: 0,
            source: "builtin".to_string(),
            source_path: source_file.to_string(),
            eligible: true,
            missing_os: false,
            missing_bins: vec![],
            missing_env: vec![],
            enabled: true,
            body: String::new(),
            body_html: format!(
                "<p><em>Built-in hook implemented in Rust.</em></p><p>{}</p>",
                description
            ),
            call_count: 0,
            failure_count: 0,
            avg_latency_ms: 0,
        });
    }

    if !info_list.is_empty() {
        info!(
            "{} hook(s) discovered ({} shell, {} built-in), {} registered",
            info_list.len(),
            discovered.len(),
            info_list.len() - discovered.len(),
            registry.handler_names().len()
        );
    }

    (Some(Arc::new(registry)), info_list)
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {
        super::*,
        std::collections::{HashMap, HashSet},
    };

    #[test]
    fn summarize_model_ids_for_logs_returns_all_when_within_limit() {
        let model_ids = vec!["a", "b", "c"]
            .into_iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();

        let summary = summarize_model_ids_for_logs(&model_ids, 8);
        assert_eq!(summary, model_ids);
    }

    #[test]
    fn summarize_model_ids_for_logs_truncates_to_head_and_tail() {
        let model_ids = vec!["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]
            .into_iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();

        let summary = summarize_model_ids_for_logs(&model_ids, 7);
        let expected = vec!["a", "b", "c", "...", "h", "i", "j"]
            .into_iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();

        assert_eq!(summary, expected);
    }

    #[test]
    fn approval_manager_uses_config_values() {
        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.tools.exec.approval_mode = "always".into();
        cfg.tools.exec.security_level = "strict".into();
        cfg.tools.exec.allowlist = vec!["git*".into()];

        let manager = approval_manager_from_config(&cfg);
        assert_eq!(manager.mode, ApprovalMode::Always);
        assert_eq!(manager.security_level, SecurityLevel::Deny);
        assert_eq!(manager.allowlist, vec!["git*".to_string()]);
    }

    #[test]
    fn approval_manager_falls_back_for_invalid_values() {
        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.tools.exec.approval_mode = "bogus".into();
        cfg.tools.exec.security_level = "bogus".into();

        let manager = approval_manager_from_config(&cfg);
        assert_eq!(manager.mode, ApprovalMode::OnMiss);
        assert_eq!(manager.security_level, SecurityLevel::Allowlist);
    }

    #[tokio::test]
    async fn discover_hooks_registers_builtin_handlers() {
        let tmp = tempfile::tempdir().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let session_store = Arc::new(SessionStore::new(sessions_dir));

        let (registry, info) =
            discover_and_build_hooks(&HashSet::new(), Some(&session_store)).await;
        let registry = registry.expect("expected hook registry to be created");
        let handler_names = registry.handler_names();

        assert!(handler_names.iter().any(|n| n == "boot-md"));
        assert!(handler_names.iter().any(|n| n == "command-logger"));
        assert!(handler_names.iter().any(|n| n == "session-memory"));

        assert!(
            info.iter()
                .any(|h| h.name == "boot-md" && h.source == "builtin")
        );
        assert!(
            info.iter()
                .any(|h| h.name == "command-logger" && h.source == "builtin")
        );
        assert!(
            info.iter()
                .any(|h| h.name == "session-memory" && h.source == "builtin")
        );
    }

    #[tokio::test]
    async fn command_hook_dispatch_saves_session_memory_file() {
        let tmp = tempfile::tempdir().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let session_store = Arc::new(SessionStore::new(sessions_dir));

        session_store
            .append(
                "smoke-session",
                &serde_json::json!({"role": "user", "content": "Hello from smoke test"}),
            )
            .await
            .unwrap();
        session_store
            .append(
                "smoke-session",
                &serde_json::json!({"role": "assistant", "content": "Hi there"}),
            )
            .await
            .unwrap();

        let mut registry = moltis_common::hooks::HookRegistry::new();
        registry.register(Arc::new(
            moltis_plugins::bundled::session_memory::SessionMemoryHook::new(
                tmp.path().to_path_buf(),
                Arc::clone(&session_store),
            ),
        ));

        let payload = moltis_common::hooks::HookPayload::Command {
            session_key: "smoke-session".into(),
            action: "new".into(),
            sender_id: None,
        };
        let result = registry.dispatch(&payload).await.unwrap();
        assert!(matches!(result, moltis_common::hooks::HookAction::Continue));

        let memory_dir = tmp.path().join("memory");
        assert!(memory_dir.is_dir());

        let files: Vec<_> = std::fs::read_dir(&memory_dir).unwrap().flatten().collect();
        assert_eq!(files.len(), 1);

        let content = std::fs::read_to_string(files[0].path()).unwrap();
        assert!(content.contains("smoke-session"));
        assert!(content.contains("Hello from smoke test"));
        assert!(content.contains("Hi there"));
    }

    #[test]
    fn prebuild_runs_only_when_mode_enabled_and_packages_present() {
        let packages = vec!["curl".to_string()];
        assert!(should_prebuild_sandbox_image(
            &moltis_tools::sandbox::SandboxMode::All,
            &packages
        ));
        assert!(should_prebuild_sandbox_image(
            &moltis_tools::sandbox::SandboxMode::NonMain,
            &packages
        ));
        assert!(!should_prebuild_sandbox_image(
            &moltis_tools::sandbox::SandboxMode::Off,
            &packages
        ));
        assert!(!should_prebuild_sandbox_image(
            &moltis_tools::sandbox::SandboxMode::All,
            &[]
        ));
    }

    #[test]
    fn proxy_tls_validation_rejects_common_misconfiguration() {
        let err = validate_proxy_tls_configuration(true, true, false)
            .expect_err("behind proxy with TLS should fail without explicit override");
        let message = err.to_string();
        assert!(message.contains("MOLTIS_BEHIND_PROXY=true"));
        assert!(message.contains("--no-tls"));
    }

    #[test]
    fn proxy_tls_validation_allows_proxy_mode_when_tls_is_disabled() {
        assert!(validate_proxy_tls_configuration(true, false, false).is_ok());
    }

    #[test]
    fn proxy_tls_validation_allows_explicit_tls_override() {
        assert!(validate_proxy_tls_configuration(true, true, true).is_ok());
    }

    #[test]
    fn merge_env_overrides_keeps_existing_config_values() {
        let base = HashMap::from([
            ("OPENAI_API_KEY".to_string(), "config-openai".to_string()),
            ("BRAVE_API_KEY".to_string(), "config-brave".to_string()),
        ]);
        let merged = merge_env_overrides(&base, vec![
            ("OPENAI_API_KEY".to_string(), "db-openai".to_string()),
            (
                "PERPLEXITY_API_KEY".to_string(),
                "db-perplexity".to_string(),
            ),
        ]);
        assert_eq!(
            merged.get("OPENAI_API_KEY").map(String::as_str),
            Some("config-openai")
        );
        assert_eq!(
            merged.get("PERPLEXITY_API_KEY").map(String::as_str),
            Some("db-perplexity")
        );
        assert_eq!(
            merged.get("BRAVE_API_KEY").map(String::as_str),
            Some("config-brave")
        );
    }

    #[test]
    fn env_value_with_overrides_uses_override_when_process_env_missing() {
        let unique_key = format!("MOLTIS_TEST_LOOKUP_{}", std::process::id());
        let overrides = HashMap::from([(unique_key.clone(), "override-value".to_string())]);
        assert_eq!(
            env_value_with_overrides(&overrides, &unique_key).as_deref(),
            Some("override-value")
        );
    }

    #[test]
    fn sync_persona_into_preset_creates_new_entry() {
        let mut agents = moltis_config::AgentsConfig::default();
        let persona = crate::agent_persona::AgentPersona {
            id: "writer".into(),
            name: "Creative Writer".into(),
            is_default: false,
            emoji: Some("\u{270d}\u{fe0f}".into()),
            theme: Some("poetic".into()),
            description: None,
            created_at: 0,
            updated_at: 0,
        };

        sync_persona_into_preset(&mut agents, &persona);

        let preset = agents.presets.get("writer").expect("preset should exist");
        assert_eq!(preset.identity.name.as_deref(), Some("Creative Writer"));
        assert_eq!(preset.identity.emoji.as_deref(), Some("\u{270d}\u{fe0f}"));
        assert_eq!(preset.identity.theme.as_deref(), Some("poetic"));
    }

    #[test]
    fn sync_persona_preserves_existing_preset_fields() {
        let mut agents = moltis_config::AgentsConfig::default();
        let existing = moltis_config::AgentPreset {
            model: Some("haiku".into()),
            timeout_secs: Some(30),
            tools: moltis_config::PresetToolPolicy {
                deny: vec!["exec".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        agents.presets.insert("coder".into(), existing);

        let persona = crate::agent_persona::AgentPersona {
            id: "coder".into(),
            name: "Code Bot".into(),
            is_default: false,
            emoji: None,
            theme: None,
            description: None,
            created_at: 0,
            updated_at: 0,
        };

        sync_persona_into_preset(&mut agents, &persona);

        let preset = agents.presets.get("coder").expect("preset should exist");
        assert_eq!(preset.identity.name.as_deref(), Some("Code Bot"));
        assert_eq!(preset.model.as_deref(), Some("haiku"));
        assert_eq!(preset.timeout_secs, Some(30));
        assert_eq!(preset.tools.deny, vec!["exec".to_string()]);
    }
}
