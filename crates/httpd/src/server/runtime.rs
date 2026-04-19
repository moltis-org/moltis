use super::*;

pub(super) struct FinalizeGatewayArgs<'a> {
    pub bind: &'a str,
    pub port: u16,
    pub tls_enabled_for_gateway: bool,
    pub state: Arc<GatewayState>,
    pub browser_for_lifecycle: Arc<dyn moltis_gateway::services::BrowserService>,
    pub browser_tool_for_warmup: Option<Arc<dyn moltis_agents::tool_registry::AgentTool>>,
    pub sandbox_router: Arc<moltis_tools::sandbox::SandboxRouter>,
    pub cron_service: Arc<moltis_cron::service::CronService>,
    pub log_buffer: Option<moltis_gateway::logs::LogBuffer>,
    pub config: moltis_config::schema::MoltisConfig,
    pub data_dir: PathBuf,
    pub provider_summary: String,
    pub mcp_configured_count: usize,
    pub method_count: usize,
    pub openclaw_startup_status: String,
    pub setup_code_display: Option<String>,
    pub webauthn_registry: Option<SharedWebAuthnRegistry>,
    #[cfg(feature = "ngrok")]
    pub ngrok_controller: Arc<NgrokController>,
    #[cfg(feature = "trusted-network")]
    pub audit_buffer_for_broadcast: Option<moltis_gateway::network_audit::NetworkAuditBuffer>,
    #[cfg(feature = "trusted-network")]
    pub _proxy_shutdown_tx: Option<tokio::sync::watch::Sender<bool>>,
    #[cfg(feature = "tailscale")]
    pub tailscale_mode: TailscaleMode,
    #[cfg(feature = "tailscale")]
    pub tailscale_reset_on_exit: bool,
    pub app: Router,
}

#[cfg(feature = "ngrok")]
pub(super) fn attach_ngrok_controller_owner(
    app_state: &mut AppState,
    ngrok_controller: &Arc<NgrokController>,
) {
    app_state.ngrok_controller_owner = Some(Arc::clone(ngrok_controller));
}

#[cfg(feature = "mdns")]
pub(super) fn instance_slug(config: &moltis_config::MoltisConfig) -> String {
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

pub(super) async fn finalize_prepared_gateway(
    args: FinalizeGatewayArgs<'_>,
) -> crate::error::Result<PreparedGateway> {
    let FinalizeGatewayArgs {
        bind,
        port,
        tls_enabled_for_gateway,
        state,
        browser_for_lifecycle,
        browser_tool_for_warmup,
        sandbox_router,
        cron_service,
        log_buffer,
        config,
        data_dir,
        provider_summary,
        mcp_configured_count,
        method_count,
        openclaw_startup_status,
        setup_code_display,
        webauthn_registry,
        #[cfg(feature = "ngrok")]
        ngrok_controller,
        #[cfg(feature = "trusted-network")]
        audit_buffer_for_broadcast,
        #[cfg(feature = "trusted-network")]
        _proxy_shutdown_tx,
        #[cfg(feature = "tailscale")]
        tailscale_mode,
        #[cfg(feature = "tailscale")]
        tailscale_reset_on_exit,
        app,
    } = args;
    #[cfg(not(feature = "tls"))]
    let _ = bind;
    #[cfg(feature = "tls")]
    let mut app = app;
    #[cfg(not(feature = "tls"))]
    let app = app;

    // Resolve TLS configuration (only when compiled with the `tls` feature).
    #[cfg_attr(not(feature = "tls"), allow(unused_variables))]
    let tls_active = tls_enabled_for_gateway;

    #[cfg(feature = "tls")]
    if tls_active {
        let tls_config = &config.tls;
        let (ca_path, _cert_path, _key_path) = if let (Some(cert_str), Some(key_str)) =
            (&tls_config.cert_path, &tls_config.key_path)
        {
            // User-provided certs.
            let cert = PathBuf::from(cert_str);
            let key = PathBuf::from(key_str);
            let ca = tls_config.ca_cert_path.as_ref().map(PathBuf::from);
            (ca, cert, key)
        } else if tls_config.auto_generate {
            // Auto-generate certificates.
            let mgr =
                moltis_tls::FsCertManager::new().map_err(|e| crate::Error::Tls(e.to_string()))?;
            let runtime_sans = tls_runtime_sans(bind);
            let (ca, cert, key) = mgr
                .ensure_certs(&runtime_sans)
                .map_err(|e| crate::Error::Tls(e.to_string()))?;
            (Some(ca), cert, key)
        } else {
            return Err(crate::Error::Tls(
                "TLS is enabled but no certificates configured and auto_generate is false".into(),
            ));
        };

        // Add /certs/ca.pem route to the main HTTPS app if we have a CA cert.
        if let Some(ref ca) = ca_path {
            let ca_bytes = Arc::new(std::fs::read(ca)?);
            let ca_clone = Arc::clone(&ca_bytes);
            app = app.route(
                "/certs/ca.pem",
                axum::routing::get(move || {
                    let data = Arc::clone(&ca_clone);
                    async move {
                        (
                            [
                                ("content-type", "application/x-pem-file"),
                                (
                                    "content-disposition",
                                    "attachment; filename=\"moltis-ca.pem\"",
                                ),
                            ],
                            data.as_ref().clone(),
                        )
                    }
                }),
            );
        }
    }

    // NOTE: the startup banner and GatewayStart hook dispatch are handled
    // by start_gateway (the CLI entry point) after prepare_gateway returns.
    // prepare_gateway only spawns background tasks and returns PreparedGateway.

    // Spawn periodic browser cleanup task (every 30s, removes idle instances).
    {
        let browser_for_cleanup = Arc::clone(&browser_for_lifecycle);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            interval.tick().await; // skip immediate first tick
            loop {
                interval.tick().await;
                browser_for_cleanup.cleanup_idle().await;
            }
        });
    }

    // Spawn tick timer.
    let tick_state = Arc::clone(&state);
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_millis(TICK_INTERVAL_MS));
        let mut sys = sysinfo::System::new();
        let pid = sysinfo::get_current_pid().ok();
        loop {
            interval.tick().await;
            sys.refresh_memory();
            if let Some(pid) = pid {
                sys.refresh_processes_specifics(
                    sysinfo::ProcessesToUpdate::Some(&[pid]),
                    false,
                    sysinfo::ProcessRefreshKind::nothing().with_memory(),
                );
            }
            let process_mem = pid
                .and_then(|p| sys.process(p))
                .map(|p| p.memory())
                .unwrap_or(0);
            let total = sys.total_memory();
            let available = match sys.available_memory() {
                0 => total.saturating_sub(sys.used_memory()),
                v => v,
            };
            let local_llama_cpp_bytes = moltis_gateway::server::local_llama_cpp_bytes_for_ui();
            broadcast_tick(
                &tick_state,
                process_mem,
                local_llama_cpp_bytes,
                available,
                total,
            )
            .await;
        }
    });

    // Spawn session event → WebSocket forwarder.
    // Events published by the swift-bridge (or any other bus producer) are
    // relayed to all connected WebSocket clients as `"session"` events,
    // enriched with full entry metadata so clients can update in-place.
    {
        let ws_state = Arc::clone(&state);
        let mut rx = state.session_event_bus.subscribe();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        let (kind, session_key) = match &event {
                            SessionEvent::Created { session_key } => {
                                ("created", session_key.as_str())
                            },
                            SessionEvent::Deleted { session_key } => {
                                ("deleted", session_key.as_str())
                            },
                            SessionEvent::Patched { session_key } => {
                                ("patched", session_key.as_str())
                            },
                        };
                        let mut payload = serde_json::json!({
                            "kind": kind,
                            "sessionKey": session_key,
                        });
                        if kind != "deleted"
                            && let Some(ref metadata) = ws_state.services.session_metadata
                            && let Some(entry) = metadata.get(session_key).await
                        {
                            let active_channel = if let Some(ref binding_json) =
                                entry.channel_binding
                            {
                                if let Ok(target) = serde_json::from_str::<
                                    moltis_channels::ChannelReplyTarget,
                                >(binding_json)
                                {
                                    metadata
                                        .get_active_session(
                                            target.channel_type.as_str(),
                                            &target.account_id,
                                            &target.chat_id,
                                            target.thread_id.as_deref(),
                                        )
                                        .await
                                        .map(|key| key == entry.key)
                                        .unwrap_or(false)
                                } else {
                                    false
                                }
                            } else {
                                false
                            };
                            let preview = entry.preview.as_deref().map(|text| {
                                let truncated = text.chars().take(200).collect::<String>();
                                if text.chars().count() > 200 {
                                    format!("{truncated}…")
                                } else {
                                    truncated
                                }
                            });
                            let agent_id = entry.agent_id.clone();
                            payload["entry"] = serde_json::json!({
                                "id": entry.id,
                                "key": entry.key,
                                "label": entry.label,
                                "model": entry.model,
                                "createdAt": entry.created_at,
                                "updatedAt": entry.updated_at,
                                "messageCount": entry.message_count,
                                "lastSeenMessageCount": entry.last_seen_message_count,
                                "projectId": entry.project_id,
                                "sandbox_enabled": entry.sandbox_enabled,
                                "sandbox_image": entry.sandbox_image,
                                "worktree_branch": entry.worktree_branch,
                                "channelBinding": entry.channel_binding,
                                "activeChannel": active_channel,
                                "parentSessionKey": entry.parent_session_key,
                                "forkPoint": entry.fork_point,
                                "mcpDisabled": entry.mcp_disabled,
                                "preview": preview,
                                "archived": entry.archived,
                                "agent_id": agent_id.clone(),
                                "agentId": agent_id,
                                "node_id": entry.node_id,
                                "version": entry.version,
                            });
                        }
                        broadcast(&ws_state, "session", payload, BroadcastOpts {
                            drop_if_slow: true,
                            ..Default::default()
                        })
                        .await;
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("session event WS forwarder lagged, skipped {n} events");
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    // Spawn periodic update check against the releases manifest.
    let update_state = Arc::clone(&state);
    let releases_url = resolve_releases_url(config.server.update_releases_url.as_deref());
    tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .user_agent(format!("moltis-gateway/{}", update_state.version))
            .timeout(std::time::Duration::from_secs(12))
            .build()
        {
            Ok(client) => client,
            Err(e) => {
                warn!("failed to initialize update checker HTTP client: {e}");
                return;
            },
        };

        let mut interval = tokio::time::interval(UPDATE_CHECK_INTERVAL);
        loop {
            interval.tick().await;
            let next =
                fetch_update_availability(&client, &releases_url, &update_state.version).await;
            let changed = {
                let mut inner = update_state.inner.write().await;
                let update = &mut inner.update;
                if *update == next {
                    false
                } else {
                    *update = next.clone();
                    true
                }
            };
            if changed && let Ok(payload) = serde_json::to_value(&next) {
                broadcast(&update_state, "update.available", payload, BroadcastOpts {
                    drop_if_slow: true,
                    ..Default::default()
                })
                .await;
            }
        }
    });

    // Spawn metrics history collection and broadcast task (every 10 seconds).
    #[cfg(feature = "metrics")]
    {
        let metrics_state = Arc::clone(&state);
        let server_start = std::time::Instant::now();
        tokio::spawn(async move {
            enum MetricsPersistJob {
                Save(moltis_gateway::state::MetricsHistoryPoint),
                CleanupBefore(u64),
            }

            // Load history from persistent store on startup.
            if let Some(ref store) = metrics_state.metrics_store {
                let max_points = metrics_state.inner.read().await.metrics_history.capacity();
                // Load enough history to fill the in-memory buffer.
                let window_secs = max_points as u64 * 30; // 30-second intervals
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let since = now_ms.saturating_sub(window_secs * 1000);
                match store.load_history(since, max_points).await {
                    Ok(points) => {
                        let mut inner = metrics_state.inner.write().await;
                        for point in points {
                            inner.metrics_history.push(point);
                        }
                        let loaded = inner.metrics_history.iter().count();
                        drop(inner);
                        info!("Loaded {loaded} historical metrics points from store");
                    },
                    Err(e) => {
                        warn!("Failed to load metrics history: {e}");
                    },
                }
            }

            // Serialize all metrics DB writes through one background writer task.
            let metrics_persist_tx = metrics_state.metrics_store.as_ref().map(|store| {
                let store = Arc::clone(store);
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<MetricsPersistJob>();
                tokio::spawn(async move {
                    while let Some(job) = rx.recv().await {
                        match job {
                            MetricsPersistJob::Save(point) => {
                                if let Err(e) = store.save_point(&point).await {
                                    warn!("Failed to persist metrics point: {e}");
                                }
                            },
                            MetricsPersistJob::CleanupBefore(cutoff) => {
                                match store.cleanup_before(cutoff).await {
                                    Ok(deleted) if deleted > 0 => {
                                        info!("Cleaned up {} old metrics points", deleted);
                                    },
                                    Err(e) => {
                                        warn!("Failed to cleanup old metrics: {e}");
                                    },
                                    _ => {},
                                }
                            },
                        }
                    }
                });
                tx
            });

            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            let mut cleanup_counter = 0u32;
            let mut sys = sysinfo::System::new();
            let pid = sysinfo::get_current_pid().ok();
            loop {
                interval.tick().await;
                if let Some(ref handle) = metrics_state.metrics_handle {
                    // Update gauges that are derived from server state, not events.
                    moltis_metrics::gauge!(moltis_metrics::system::UPTIME_SECONDS)
                        .set(server_start.elapsed().as_secs_f64());
                    let session_count =
                        metrics_state.inner.read().await.active_sessions.len() as f64;
                    moltis_metrics::gauge!(moltis_metrics::session::ACTIVE).set(session_count);

                    let prometheus_text = handle.render();
                    let snapshot =
                        moltis_metrics::MetricsSnapshot::from_prometheus_text(&prometheus_text);
                    sys.refresh_memory();
                    if let Some(pid) = pid {
                        sys.refresh_processes_specifics(
                            sysinfo::ProcessesToUpdate::Some(&[pid]),
                            false,
                            sysinfo::ProcessRefreshKind::nothing().with_memory(),
                        );
                    }
                    let process_memory_bytes = pid
                        .and_then(|p| sys.process(p))
                        .map(|p| p.memory())
                        .unwrap_or(0);
                    let local_llama_cpp_bytes =
                        moltis_gateway::server::local_llama_cpp_bytes_for_ui();
                    // Convert per-provider metrics to history format.
                    let by_provider = snapshot
                        .categories
                        .llm
                        .by_provider
                        .iter()
                        .map(|(name, metrics)| {
                            (name.clone(), moltis_metrics::ProviderTokens {
                                input_tokens: metrics.input_tokens,
                                output_tokens: metrics.output_tokens,
                                completions: metrics.completions,
                                errors: metrics.errors,
                            })
                        })
                        .collect();

                    let point = moltis_gateway::state::MetricsHistoryPoint {
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64,
                        llm_completions: snapshot.categories.llm.completions_total,
                        llm_input_tokens: snapshot.categories.llm.input_tokens,
                        llm_output_tokens: snapshot.categories.llm.output_tokens,
                        llm_errors: snapshot.categories.llm.errors,
                        by_provider,
                        http_requests: snapshot.categories.http.total,
                        http_active: snapshot.categories.http.active,
                        ws_connections: snapshot.categories.websocket.total,
                        ws_active: snapshot.categories.websocket.active,
                        tool_executions: snapshot.categories.tools.total,
                        tool_errors: snapshot.categories.tools.errors,
                        mcp_calls: snapshot.categories.mcp.total,
                        active_sessions: snapshot.categories.system.active_sessions,
                        process_memory_bytes,
                        local_llama_cpp_bytes,
                    };

                    // Push to in-memory history.
                    metrics_state
                        .inner
                        .write()
                        .await
                        .metrics_history
                        .push(point.clone());

                    // Persist via the dedicated writer, without stalling collection.
                    if let Some(tx) = metrics_persist_tx.as_ref()
                        && tx.send(MetricsPersistJob::Save(point.clone())).is_err()
                    {
                        warn!("metrics persistence writer task is unavailable");
                    }

                    // Broadcast metrics update to all connected clients.
                    let payload = moltis_gateway::state::MetricsUpdatePayload { snapshot, point };
                    if let Ok(payload_json) = serde_json::to_value(&payload) {
                        broadcast(
                            &metrics_state,
                            "metrics.update",
                            payload_json,
                            BroadcastOpts {
                                drop_if_slow: true,
                                ..Default::default()
                            },
                        )
                        .await;
                    }

                    // Cleanup old data once per hour (120 ticks at 30s interval).
                    cleanup_counter += 1;
                    if cleanup_counter >= 120 {
                        cleanup_counter = 0;
                        if let Some(tx) = metrics_persist_tx.as_ref() {
                            // Keep 7 days of history.
                            let cutoff = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as u64
                                - (7 * 24 * 60 * 60 * 1000);
                            if tx.send(MetricsPersistJob::CleanupBefore(cutoff)).is_err() {
                                warn!("metrics persistence writer task is unavailable");
                            }
                        }
                    }
                }
            }
        });
    }

    // Spawn sandbox event broadcast task: forwards sandbox lifecycle events to WS clients.
    {
        let event_state = Arc::clone(&state);
        let mut event_rx = sandbox_router.subscribe_events();
        tokio::spawn(async move {
            loop {
                match event_rx.recv().await {
                    Ok(event) => {
                        let (event_name, payload) = match event {
                            moltis_tools::sandbox::SandboxEvent::Preparing {
                                session_key,
                                backend,
                                image,
                            } => (
                                "sandbox.prepare",
                                serde_json::json!({
                                    "phase": "start",
                                    "session_key": session_key,
                                    "backend": backend,
                                    "image": image,
                                }),
                            ),
                            moltis_tools::sandbox::SandboxEvent::Prepared {
                                session_key,
                                backend,
                                image,
                            } => (
                                "sandbox.prepare",
                                serde_json::json!({
                                    "phase": "done",
                                    "session_key": session_key,
                                    "backend": backend,
                                    "image": image,
                                }),
                            ),
                            moltis_tools::sandbox::SandboxEvent::PrepareFailed {
                                session_key,
                                backend,
                                image,
                                error,
                            } => (
                                "sandbox.prepare",
                                serde_json::json!({
                                    "phase": "error",
                                    "session_key": session_key,
                                    "backend": backend,
                                    "image": image,
                                    "error": error,
                                }),
                            ),
                            moltis_tools::sandbox::SandboxEvent::Provisioning {
                                container,
                                packages,
                            } => (
                                "sandbox.image.provision",
                                serde_json::json!({
                                    "phase": "start",
                                    "container": container,
                                    "packages": packages,
                                }),
                            ),
                            moltis_tools::sandbox::SandboxEvent::Provisioned { container } => (
                                "sandbox.image.provision",
                                serde_json::json!({
                                    "phase": "done",
                                    "container": container,
                                }),
                            ),
                            moltis_tools::sandbox::SandboxEvent::ProvisionFailed {
                                container,
                                error,
                            } => (
                                "sandbox.image.provision",
                                serde_json::json!({
                                    "phase": "error",
                                    "container": container,
                                    "error": error,
                                }),
                            ),
                        };
                        broadcast(&event_state, event_name, payload, BroadcastOpts {
                            drop_if_slow: true,
                            ..Default::default()
                        })
                        .await;
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        });
    }

    // Spawn network audit broadcast task: forwards audit entries to WS clients.
    #[cfg(feature = "trusted-network")]
    if let Some(ref audit_buf) = audit_buffer_for_broadcast {
        let audit_state = Arc::clone(&state);
        let mut audit_rx = audit_buf.subscribe();
        tokio::spawn(async move {
            loop {
                match audit_rx.recv().await {
                    Ok(entry) => {
                        if let Ok(payload) = serde_json::to_value(&entry) {
                            broadcast(
                                &audit_state,
                                "network.audit.entry",
                                payload,
                                BroadcastOpts {
                                    drop_if_slow: true,
                                    ..Default::default()
                                },
                            )
                            .await;
                        }
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        });
    }

    // Spawn log broadcast task: forwards captured tracing events to WS clients.
    if let Some(buf) = log_buffer {
        let log_state = Arc::clone(&state);
        tokio::spawn(async move {
            let mut rx = buf.subscribe();
            loop {
                match rx.recv().await {
                    Ok(entry) => {
                        if let Ok(payload) = serde_json::to_value(&entry) {
                            broadcast(&log_state, "logs.entry", payload, BroadcastOpts {
                                drop_if_slow: true,
                                ..Default::default()
                            })
                            .await;
                        }
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        });
    }

    // Start the cron scheduler (loads persisted jobs, arms the timer).
    if let Err(e) = cron_service.start().await {
        tracing::warn!("failed to start cron scheduler: {e}");
    }

    // Upsert the built-in heartbeat job from config.
    // Use a fixed ID so run history persists across restarts.
    {
        use moltis_cron::{
            heartbeat::{
                DEFAULT_INTERVAL_MS, HeartbeatPromptSource, parse_interval_ms,
                resolve_heartbeat_prompt,
            },
            types::{CronJobCreate, CronJobPatch, CronPayload, CronSchedule, SessionTarget},
        };
        const HEARTBEAT_JOB_ID: &str = "__heartbeat__";

        let hb = &config.heartbeat;
        let interval_ms = parse_interval_ms(&hb.every).unwrap_or(DEFAULT_INTERVAL_MS);
        let heartbeat_md = moltis_config::load_heartbeat_md();
        let (prompt, prompt_source) =
            resolve_heartbeat_prompt(hb.prompt.as_deref(), heartbeat_md.as_deref());
        if prompt_source == HeartbeatPromptSource::HeartbeatMd {
            tracing::info!("loaded heartbeat prompt from HEARTBEAT.md");
        }
        if hb.prompt.as_deref().is_some_and(|p| !p.trim().is_empty())
            && heartbeat_md
                .as_deref()
                .is_some_and(|p| !p.trim().is_empty())
            && prompt_source == HeartbeatPromptSource::Config
        {
            tracing::warn!(
                "heartbeat prompt source conflict: config heartbeat.prompt overrides HEARTBEAT.md"
            );
        }

        // Check if heartbeat job already exists.
        let existing = cron_service.list().await;
        let existing_job = existing.iter().find(|j| j.id == HEARTBEAT_JOB_ID);

        // Skip heartbeat when there is no meaningful prompt (no config prompt,
        // no HEARTBEAT.md content). The built-in default prompt is generic and
        // wastes LLM calls when the user hasn't configured anything.
        let has_prompt = prompt_source != HeartbeatPromptSource::Default;

        if hb.enabled && has_prompt {
            if existing_job.is_some() {
                // Update existing job to match config.
                let patch = CronJobPatch {
                    schedule: Some(CronSchedule::Every {
                        every_ms: interval_ms,
                        anchor_ms: None,
                    }),
                    payload: Some(CronPayload::AgentTurn {
                        message: prompt,
                        model: hb.model.clone(),
                        timeout_secs: None,
                        deliver: hb.deliver,
                        channel: hb.channel.clone(),
                        to: hb.to.clone(),
                    }),
                    enabled: Some(true),
                    sandbox: Some(moltis_cron::types::CronSandboxConfig {
                        enabled: hb.sandbox_enabled,
                        image: hb.sandbox_image.clone(),
                        auto_prune_container: None,
                    }),
                    ..Default::default()
                };
                match cron_service.update(HEARTBEAT_JOB_ID, patch).await {
                    Ok(job) => tracing::info!(id = %job.id, "heartbeat job updated"),
                    Err(e) => tracing::warn!("failed to update heartbeat job: {e}"),
                }
            } else {
                // Create new job with fixed ID.
                let create = CronJobCreate {
                    id: Some(HEARTBEAT_JOB_ID.into()),
                    name: "__heartbeat__".into(),
                    schedule: CronSchedule::Every {
                        every_ms: interval_ms,
                        anchor_ms: None,
                    },
                    payload: CronPayload::AgentTurn {
                        message: prompt,
                        model: hb.model.clone(),
                        timeout_secs: None,
                        deliver: hb.deliver,
                        channel: hb.channel.clone(),
                        to: hb.to.clone(),
                    },
                    session_target: SessionTarget::Named("heartbeat".into()),
                    delete_after_run: false,
                    enabled: true,
                    system: true,
                    sandbox: moltis_cron::types::CronSandboxConfig {
                        enabled: hb.sandbox_enabled,
                        image: hb.sandbox_image.clone(),
                        auto_prune_container: None,
                    },
                    wake_mode: moltis_cron::types::CronWakeMode::default(),
                };
                match cron_service.add(create).await {
                    Ok(job) => tracing::info!(id = %job.id, "heartbeat job created"),
                    Err(e) => tracing::warn!("failed to create heartbeat job: {e}"),
                }
            }
        } else if existing_job.is_some() {
            // Heartbeat is disabled or has no prompt content — remove the job.
            let _ = cron_service.remove(HEARTBEAT_JOB_ID).await;
            if !hb.enabled {
                tracing::info!("heartbeat job removed (disabled)");
            } else {
                tracing::info!("heartbeat job removed (no prompt configured)");
            }
        } else if hb.enabled && !has_prompt {
            tracing::info!("heartbeat skipped: no prompt in config and HEARTBEAT.md is empty");
        }
    }

    #[cfg(feature = "ngrok")]
    ngrok_controller.configure_app(app.clone()).await;

    Ok(PreparedGateway {
        app,
        state: Arc::clone(&state),
        port,
        banner: BannerMeta {
            provider_summary,
            mcp_configured_count,
            method_count,
            sandbox_backend_name: sandbox_router.backend_name().to_owned(),
            data_dir,
            openclaw_status: openclaw_startup_status,
            setup_code_display,
            webauthn_registry,
            #[cfg(feature = "ngrok")]
            ngrok_controller,
            browser_for_lifecycle,
            browser_tool_for_warmup,
            config,
            #[cfg(feature = "tailscale")]
            tailscale_mode,
            #[cfg(feature = "tailscale")]
            tailscale_reset_on_exit,
        },
        #[cfg(feature = "trusted-network")]
        audit_buffer: audit_buffer_for_broadcast,
        #[cfg(feature = "trusted-network")]
        _proxy_shutdown_tx,
    })
}

/// Prepare the full gateway for embedded callers (for example swift-bridge)
/// using a feature-stable argument list.
///
/// This wrapper intentionally hides `tailscale_opts`, which only exists when
/// the `tailscale` feature is enabled.
#[allow(clippy::expect_used)]
pub async fn prepare_gateway_embedded(
    bind: &str,
    port: u16,
    no_tls: bool,
    log_buffer: Option<moltis_gateway::logs::LogBuffer>,
    config_dir: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    extra_routes: Option<RouteEnhancer>,
    session_event_bus: Option<SessionEventBus>,
) -> crate::error::Result<PreparedGateway> {
    let prepared = prepare_gateway(
        bind,
        port,
        no_tls,
        log_buffer,
        config_dir,
        data_dir,
        #[cfg(feature = "tailscale")]
        None,
        extra_routes,
        session_event_bus,
    )
    .await?;
    // Embedded callers own the listener lifecycle, but still need non-blocking
    // OpenClaw startup tasks.
    moltis_gateway::server::start_openclaw_background_tasks(prepared.banner.data_dir.clone());
    Ok(prepared)
}

/// Alias for [`prepare_gateway_embedded`] used by swift-bridge consumers.
pub use prepare_gateway_embedded as prepare_httpd_embedded;

#[cfg(feature = "ngrok")]
pub(super) fn ngrok_loopback_has_proxy_headers(headers: &axum::http::HeaderMap) -> bool {
    moltis_auth::locality::has_proxy_headers(headers)
}

#[cfg(feature = "ngrok")]
pub(super) async fn require_ngrok_proxy_headers(
    request: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if !ngrok_loopback_has_proxy_headers(request.headers()) {
        warn!("rejecting ngrok loopback request without proxy headers");
        return StatusCode::FORBIDDEN.into_response();
    }

    next.run(request).await
}

#[cfg(feature = "ngrok")]
pub(super) async fn start_ngrok_tunnel(
    app: Router,
    gateway: Arc<GatewayState>,
    webauthn_registry: Option<SharedWebAuthnRegistry>,
    ngrok_config: &moltis_config::NgrokConfig,
) -> crate::error::Result<NgrokActiveTunnel> {
    use ::ngrok::prelude::{EndpointInfo, ForwarderBuilder};

    let internal_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let internal_addr = internal_listener.local_addr()?;
    let internal_app = app
        .clone()
        .layer(axum::middleware::from_fn(require_ngrok_proxy_headers));
    let loopback_shutdown = CancellationToken::new();
    let loopback_cancel = loopback_shutdown.clone();
    let loopback_task = tokio::spawn(async move {
        let server = axum::serve(
            internal_listener,
            internal_app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            loopback_cancel.cancelled().await;
        });

        if let Err(error) = server.await {
            warn!(%error, "ngrok loopback forward server exited");
        }
    });

    let forward_to = format!("http://{internal_addr}")
        .parse()
        .map_err(|error| crate::Error::Ngrok(format!("invalid ngrok forward target: {error}")))?;

    let mut session_builder = ::ngrok::Session::builder();
    if let Some(authtoken) = ngrok_config.authtoken.as_ref() {
        session_builder.authtoken(authtoken.expose_secret());
    } else {
        session_builder.authtoken_from_env();
    }
    let mut session = match session_builder.connect().await {
        Ok(session) => session,
        Err(error) => {
            loopback_shutdown.cancel();
            if let Err(join_error) = loopback_task.await {
                warn!(%join_error, "ngrok loopback server task join failed during startup");
            }
            return Err(crate::Error::Ngrok(error.to_string()));
        },
    };

    let mut endpoint = session.http_endpoint();
    if let Some(domain) = ngrok_config.domain.as_deref() {
        endpoint.domain(domain);
    }
    endpoint.forwards_to(format!("moltis://{internal_addr}"));
    let forwarder = match endpoint.listen_and_forward(forward_to).await {
        Ok(forwarder) => forwarder,
        Err(error) => {
            if let Err(close_error) = session.close().await {
                warn!(%close_error, "failed to close ngrok session after startup error");
            }
            loopback_shutdown.cancel();
            if let Err(join_error) = loopback_task.await {
                warn!(%join_error, "ngrok loopback server task join failed during startup");
            }
            return Err(crate::Error::Ngrok(error.to_string()));
        },
    };
    let public_url = forwarder.url().to_string();
    let public_host = url::Url::parse(&public_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string));

    let passkey_warning = moltis_gateway::server::sync_runtime_webauthn_host_and_notice(
        &gateway,
        webauthn_registry.as_ref(),
        public_host.as_deref(),
        Some(&public_url),
        "ngrok tunnel",
    )
    .await;
    let status = NgrokRuntimeStatus {
        public_url,
        passkey_warning,
    };

    Ok(NgrokActiveTunnel {
        session,
        forwarder,
        loopback_shutdown,
        loopback_task,
        status,
    })
}

/// Start the gateway HTTP + WebSocket server.
///
/// Thin wrapper around [`prepare_gateway`] that adds the startup banner,
/// ctrl-c handler, TLS termination, and blocks on `axum::serve`.
#[allow(clippy::expect_used)]
pub async fn start_gateway(
    bind: &str,
    port: u16,
    no_tls: bool,
    log_buffer: Option<moltis_gateway::logs::LogBuffer>,
    config_dir: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    #[cfg(feature = "tailscale")] tailscale_opts: Option<TailscaleOpts>,
    extra_routes: Option<RouteEnhancer>,
) -> crate::error::Result<()> {
    let prepared = prepare_gateway(
        bind,
        port,
        no_tls,
        log_buffer,
        config_dir,
        data_dir,
        #[cfg(feature = "tailscale")]
        tailscale_opts,
        extra_routes,
        None, // session_event_bus — CLI creates its own
    )
    .await?;

    let state = &prepared.state;
    let banner = &prepared.banner;
    #[cfg_attr(not(feature = "tls"), allow(unused_variables))]
    let config = &banner.config;

    let ip: std::net::IpAddr = bind
        .parse()
        .map_err(|e| crate::Error::Config(format!("invalid bind address '{bind}': {e}")))?;
    let addr = SocketAddr::new(ip, port);

    #[cfg_attr(not(feature = "tls"), allow(unused_variables))]
    let is_localhost =
        matches!(bind, "127.0.0.1" | "::1" | "localhost") || bind.ends_with(".localhost");

    // Register the gateway as a Bonjour/mDNS service so LAN clients can
    // discover it without typing the URL manually.
    #[cfg(feature = "mdns")]
    let _mdns_daemon = {
        let host = hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "moltis".to_string());
        let instance = format!("Moltis on {host}");
        match moltis_gateway::mdns::register(
            &instance,
            port,
            env!("CARGO_PKG_VERSION"),
            Some(&instance_slug(config)),
        ) {
            Ok(daemon) => Some(daemon),
            Err(e) => {
                tracing::warn!("mDNS registration failed: {e}");
                None
            },
        }
    };

    // Resolve TLS configuration (only when compiled with the `tls` feature).
    #[cfg(feature = "tls")]
    let tls_active = config.tls.enabled;
    #[cfg(not(feature = "tls"))]
    let tls_active = false;

    #[cfg(feature = "tls")]
    let mut ca_cert_path: Option<PathBuf> = None;
    #[cfg(feature = "tls")]
    let mut rustls_config: Option<rustls::ServerConfig> = None;

    let app = prepared.app;
    let browser_for_warmup = Arc::clone(&banner.browser_for_lifecycle);
    let browser_tool_for_warmup = banner.browser_tool_for_warmup.clone();
    #[cfg(feature = "ngrok")]
    let (ngrok_status, ngrok_startup_error) =
        match banner.ngrok_controller.apply(&config.ngrok).await {
            Ok(status) => (status, None),
            Err(error) => {
                warn!(%error, "ngrok tunnel failed to start; gateway will continue without it");
                (None, Some(error.to_string()))
            },
        };

    #[cfg(feature = "tls")]
    if tls_active {
        let tls_config = &config.tls;
        let (ca_path, cert_path, key_path) = if let (Some(cert_str), Some(key_str)) =
            (&tls_config.cert_path, &tls_config.key_path)
        {
            // User-provided certs.
            let cert = PathBuf::from(cert_str);
            let key = PathBuf::from(key_str);
            let ca = tls_config.ca_cert_path.as_ref().map(PathBuf::from);
            (ca, cert, key)
        } else if tls_config.auto_generate {
            // Auto-generate certificates.
            let mgr =
                moltis_tls::FsCertManager::new().map_err(|e| crate::Error::Tls(e.to_string()))?;
            let runtime_sans = tls_runtime_sans(bind);
            let (ca, cert, key) = mgr
                .ensure_certs(&runtime_sans)
                .map_err(|e| crate::Error::Tls(e.to_string()))?;
            (Some(ca), cert, key)
        } else {
            return Err(crate::Error::Tls(
                "TLS is enabled but no certificates configured and auto_generate is false".into(),
            ));
        };

        ca_cert_path = ca_path.clone();

        let mgr = moltis_tls::FsCertManager::new().map_err(|e| crate::Error::Tls(e.to_string()))?;
        rustls_config = Some(
            mgr.build_rustls_config(&cert_path, &key_path)
                .map_err(|e| crate::Error::Tls(e.to_string()))?,
        );
        // Note: /certs/ca.pem route is already registered by prepare_gateway.
    }

    // Count enabled skills and repos for startup banner.
    let (skill_count, repo_count) = {
        use moltis_skills::discover::{FsSkillDiscoverer, SkillDiscoverer};
        let discoverer = FsSkillDiscoverer::new(FsSkillDiscoverer::default_paths());
        let sc = discoverer.discover().await.map(|s| s.len()).unwrap_or(0);
        let rc = moltis_skills::manifest::ManifestStore::default_path()
            .ok()
            .map(|p| {
                let store = moltis_skills::manifest::ManifestStore::new(p);
                store.load().map(|m| m.repos.len()).unwrap_or(0)
            })
            .unwrap_or(0);
        (sc, rc)
    };

    // Startup banner.
    let scheme = if tls_active {
        "https"
    } else {
        "http"
    };
    // When bound to an unspecified address (0.0.0.0 / ::), resolve the
    // machine's outbound IP so the printed URL is clickable.
    let display_ip = if addr.ip().is_unspecified() {
        resolve_outbound_ip(addr.ip().is_ipv6())
            .map(|ip| SocketAddr::new(ip, port))
            .unwrap_or(addr)
    } else {
        addr
    };
    // Use plain localhost for display URLs when bound to loopback with TLS.
    #[cfg(feature = "tls")]
    let display_host = if is_localhost && tls_active {
        format!("localhost:{port}")
    } else {
        display_ip.to_string()
    };
    #[cfg(not(feature = "tls"))]
    let display_host = display_ip.to_string();
    let passkey_origins = if let Some(registry) = banner.webauthn_registry.as_ref() {
        registry.read().await.get_all_origins()
    } else {
        Vec::new()
    };
    #[cfg_attr(not(feature = "tls"), allow(unused_mut))]
    let mut lines = vec![
        format!("moltis gateway v{}", state.version),
        format!(
            "protocol v{}, listening on {}://{} ({})",
            moltis_protocol::PROTOCOL_VERSION,
            scheme,
            display_host,
            if tls_active {
                "HTTP/2 + HTTP/1.1"
            } else {
                "HTTP/1.1"
            },
        ),
        startup_bind_line(addr),
        format!("{} methods registered", banner.method_count),
        format!("llm: {}", banner.provider_summary),
        format!(
            "skills: {} enabled, {} repo{}",
            skill_count,
            repo_count,
            if repo_count == 1 {
                ""
            } else {
                "s"
            }
        ),
        format!(
            "mcp: {} configured{}",
            banner.mcp_configured_count,
            if banner.mcp_configured_count > 0 {
                " (starting in background)"
            } else {
                ""
            }
        ),
        format!("sandbox: {} backend", banner.sandbox_backend_name),
        format!(
            "config: {}",
            moltis_config::find_or_default_config_path().display()
        ),
        format!("data: {}", banner.data_dir.display()),
        format!("openclaw: {}", banner.openclaw_status),
    ];
    lines.extend(startup_passkey_origin_lines(&passkey_origins));
    #[cfg(feature = "ngrok")]
    if let Some(status) = ngrok_status.as_ref() {
        lines.push(format!("ngrok: {}", status.public_url));
        if let Some(passkey_warning) = status.passkey_warning.as_ref() {
            lines.push(format!("ngrok note: {passkey_warning}"));
        }
    } else if let Some(error) = ngrok_startup_error.as_deref() {
        lines.push(format!("ngrok: failed to start ({error})"));
    }
    #[cfg(not(feature = "ngrok"))]
    if config.ngrok.enabled {
        lines.push(
            "ngrok: enabled in config but this build does not include the ngrok feature".into(),
        );
    }
    // Hint about Apple Container on macOS when using Docker or Podman.
    #[cfg(target_os = "macos")]
    if banner.sandbox_backend_name == "docker" || banner.sandbox_backend_name == "podman" {
        lines.push(
            "hint: install Apple Container for VM-isolated sandboxing (see docs/sandbox.md)".into(),
        );
    }
    // Warn when no sandbox backend is available.
    if banner.sandbox_backend_name == "none" {
        lines.push("⚠ no container runtime found; commands run on host".into());
    }
    // Warn when TLS is off and the server is not localhost-only.
    if !tls_active && !is_localhost {
        lines.push(
            "⚠ TLS is disabled on a non-localhost bind address; session cookies will be sent over unencrypted HTTP".into(),
        );
    }
    // Display setup code if one was generated.
    if let Some(ref code) = banner.setup_code_display {
        lines.extend(startup_setup_code_lines(code));
    }
    #[cfg(feature = "tls")]
    if tls_active {
        if let Some(ref ca) = ca_cert_path {
            let http_port = config.tls.http_redirect_port.unwrap_or(port + 1);
            let ca_host = if is_localhost {
                "localhost"
            } else {
                bind
            };
            lines.push(format!(
                "CA cert: http://{}:{}/certs/ca.pem",
                ca_host, http_port
            ));
            lines.push(format!("  or: {}", ca.display()));
        }
        lines.push("run `moltis trust-ca` to remove browser warnings".into());
    }
    // Tailscale: enable serve/funnel and show in banner.
    #[cfg(feature = "tailscale")]
    {
        let tailscale_mode = banner.tailscale_mode;
        if tailscale_mode != TailscaleMode::Off {
            let manager = CliTailscaleManager::new();
            let ts_result = match tailscale_mode {
                TailscaleMode::Serve => manager.enable_serve(port, tls_active).await,
                TailscaleMode::Funnel => manager.enable_funnel(port, tls_active).await,
                TailscaleMode::Off => unreachable!(),
            };
            match ts_result {
                Ok(()) => {
                    if let Ok(Some(hostname)) = manager.hostname().await {
                        lines.push(format!("tailscale {tailscale_mode}: https://{hostname}"));
                    } else {
                        lines.push(format!("tailscale {tailscale_mode}: enabled"));
                    }
                },
                Err(e) => {
                    warn!("failed to enable tailscale {tailscale_mode}: {e}");
                    lines.push(format!("tailscale {tailscale_mode}: FAILED ({e})"));
                },
            }
        }
    }
    let width = lines.iter().map(|l| l.len()).max().unwrap_or(0) + 4;
    info!("┌{}┐", "─".repeat(width));
    for line in &lines {
        info!("│  {:<w$}│", line, w = width - 2);
    }
    info!("└{}┘", "─".repeat(width));

    // Dispatch GatewayStart hook.
    if let Some(ref hooks) = state.inner.read().await.hook_registry {
        let payload = moltis_common::hooks::HookPayload::GatewayStart {
            address: addr.to_string(),
        };
        if let Err(e) = hooks.dispatch(&payload).await {
            tracing::warn!("GatewayStart hook dispatch failed: {e}");
        }
    }

    // Spawn shutdown handler:
    // - unregister mDNS service (when configured)
    // - reset tailscale state on exit (when configured)
    // - give browser pool 5s to shut down gracefully
    // - force process exit to avoid hanging after ctrl-c
    {
        let browser_for_shutdown = Arc::clone(&banner.browser_for_lifecycle);
        #[cfg(feature = "tailscale")]
        let reset_tailscale_on_exit =
            banner.tailscale_mode != TailscaleMode::Off && banner.tailscale_reset_on_exit;
        #[cfg(feature = "tailscale")]
        let ts_mode = banner.tailscale_mode;
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_err() {
                return;
            }

            #[cfg(feature = "mdns")]
            if let Some(ref daemon) = _mdns_daemon {
                moltis_gateway::mdns::shutdown(daemon);
            }

            #[cfg(feature = "tailscale")]
            if reset_tailscale_on_exit {
                info!("shutting down tailscale {ts_mode}");
                let manager = CliTailscaleManager::new();
                if let Err(e) = manager.disable().await {
                    warn!("failed to reset tailscale on exit: {e}");
                }
            }

            let shutdown_grace = std::time::Duration::from_secs(5);
            info!(
                grace_secs = shutdown_grace.as_secs(),
                "shutting down browser pool"
            );
            if browser_for_shutdown
                .shutdown_with_grace(shutdown_grace)
                .await
            {
                info!(
                    grace_secs = shutdown_grace.as_secs(),
                    "browser pool shut down"
                );
            } else {
                warn!(
                    grace_secs = shutdown_grace.as_secs(),
                    "browser pool shutdown exceeded grace period, forcing process exit"
                );
            }

            std::process::exit(0);
        });
    }

    #[cfg(feature = "tls")]
    if tls_active {
        // Spawn HTTP redirect server on secondary port (serves CA cert download).
        if let Some(ref ca) = ca_cert_path {
            let http_port = config.tls.http_redirect_port.unwrap_or(port + 1);
            let bind_clone = bind.to_string();
            let ca_clone = ca.clone();
            tokio::spawn(async move {
                if let Err(e) =
                    moltis_tls::start_http_redirect_server(&bind_clone, http_port, port, &ca_clone)
                        .await
                {
                    tracing::error!("HTTP redirect server failed: {e}");
                }
            });
        }

        // Run HTTPS server with automatic HTTP-to-HTTPS redirect on the same port.
        // Plain HTTP requests to this port get a 301 redirect instead of a TLS error.
        let tls_cfg = rustls_config.expect("rustls config must be set when TLS is active");
        let tcp_listener = tokio::net::TcpListener::bind(addr).await?;
        moltis_gateway::server::start_openclaw_background_tasks(banner.data_dir.clone());
        moltis_gateway::server::start_browser_warmup_after_listener(
            Arc::clone(&browser_for_warmup),
            browser_tool_for_warmup.clone(),
        );
        moltis_tls::serve_tls_with_http_redirect(tcp_listener, Arc::new(tls_cfg), app, port, bind)
            .await
            .map_err(|e| crate::Error::Tls(e.to_string()))?;
        return Ok(());
    }

    // Plain HTTP server (existing behavior, or TLS feature disabled).
    let listener = tokio::net::TcpListener::bind(addr).await?;
    moltis_gateway::server::start_openclaw_background_tasks(banner.data_dir.clone());
    moltis_gateway::server::start_browser_warmup_after_listener(
        Arc::clone(&browser_for_warmup),
        browser_tool_for_warmup,
    );
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}
