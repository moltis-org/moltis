use super::*;

pub(super) fn register(reg: &mut MethodRegistry) {
    // Config
    reg.register(
        "config.get",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .config
                    .get(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "config.set",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .config
                    .set(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "config.apply",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .config
                    .apply(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "config.patch",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .config
                    .patch(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "config.schema",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .config
                    .schema()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    // Cron
    reg.register(
        "cron.list",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .cron
                    .list()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "cron.status",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .cron
                    .status()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "cron.add",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .cron
                    .add(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "cron.update",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .cron
                    .update(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "cron.remove",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .cron
                    .remove(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "cron.run",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .cron
                    .run(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "cron.runs",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .cron
                    .runs(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    // Webhooks
    reg.register(
        "webhooks.list",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .webhooks
                    .list()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "webhooks.get",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .webhooks
                    .get(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "webhooks.create",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .webhooks
                    .create(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "webhooks.update",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .webhooks
                    .update(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "webhooks.delete",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .webhooks
                    .delete(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "webhooks.deliveries",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .webhooks
                    .deliveries(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "webhooks.delivery.get",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .webhooks
                    .delivery_get(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "webhooks.delivery.payload",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .webhooks
                    .delivery_payload(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "webhooks.delivery.actions",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .webhooks
                    .delivery_actions(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "webhooks.profiles",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .webhooks
                    .profiles()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    // Heartbeat
    reg.register(
        "heartbeat.status",
        Box::new(|ctx| {
            Box::pin(async move {
                let config = ctx.state.inner.read().await.heartbeat_config.clone();
                let heartbeat_path = moltis_config::heartbeat_path();
                let heartbeat_file_exists = heartbeat_path.exists();
                let heartbeat_md = moltis_config::load_heartbeat_md();
                let (_, prompt_source) = moltis_cron::heartbeat::resolve_heartbeat_prompt(
                    config.prompt.as_deref(),
                    heartbeat_md.as_deref(),
                );
                // No meaningful prompt → heartbeat won't execute.
                let has_prompt =
                    prompt_source != moltis_cron::heartbeat::HeartbeatPromptSource::Default;
                // Find the heartbeat job to get its state.
                let jobs_val = ctx
                    .state
                    .services
                    .cron
                    .list()
                    .await
                    .map_err(ErrorShape::from)?;
                let jobs: Vec<moltis_cron::types::CronJob> =
                    serde_json::from_value(jobs_val).unwrap_or_default();
                let hb_job = jobs.iter().find(|j| j.name == "__heartbeat__");
                Ok(serde_json::json!({
                    "config": config,
                    "job": hb_job,
                    "promptSource": prompt_source.as_str(),
                    "heartbeatFileExists": heartbeat_file_exists,
                    "hasPrompt": has_prompt,
                }))
            })
        }),
    );
    reg.register(
            "heartbeat.update",
            Box::new(|ctx| {
                Box::pin(async move {
                    let patch: moltis_config::schema::HeartbeatConfig =
                        serde_json::from_value(ctx.params.clone()).map_err(|e| {
                            ErrorShape::new(
                                error_codes::INVALID_REQUEST,
                                format!("invalid heartbeat config: {e}"),
                            )
                        })?;
                    ctx.state.inner.write().await.heartbeat_config = patch.clone();

                    // Persist to moltis.toml so the config survives restarts.
                    if let Err(e) = moltis_config::update_config(|cfg| {
                        cfg.heartbeat = patch.clone();
                    }) {
                        tracing::warn!(error = %e, "failed to persist heartbeat config");
                    }

                    // Update the heartbeat cron job in-place.
                    let jobs_val = ctx
                        .state
                        .services
                        .cron
                        .list()
                        .await
                        .map_err(ErrorShape::from)?;
                    let jobs: Vec<moltis_cron::types::CronJob> =
                        serde_json::from_value(jobs_val).unwrap_or_default();
                    let interval_ms = moltis_cron::heartbeat::parse_interval_ms(&patch.every)
                        .unwrap_or(moltis_cron::heartbeat::DEFAULT_INTERVAL_MS);
                    let heartbeat_md = moltis_config::load_heartbeat_md();
                    let (prompt, prompt_source) =
                        moltis_cron::heartbeat::resolve_heartbeat_prompt(
                            patch.prompt.as_deref(),
                            heartbeat_md.as_deref(),
                        );
                    if prompt_source
                        == moltis_cron::heartbeat::HeartbeatPromptSource::HeartbeatMd
                    {
                        tracing::info!("loaded heartbeat prompt from HEARTBEAT.md");
                    }
                    if patch.prompt.as_deref().is_some_and(|p| !p.trim().is_empty())
                        && heartbeat_md.as_deref().is_some_and(|p| !p.trim().is_empty())
                        && prompt_source
                            == moltis_cron::heartbeat::HeartbeatPromptSource::Config
                    {
                        tracing::warn!(
                            "heartbeat prompt source conflict: config heartbeat.prompt overrides HEARTBEAT.md"
                        );
                    }
                    // Disable the job when there is no meaningful prompt,
                    // even if the user toggled enabled=true.
                    let has_prompt = prompt_source
                        != moltis_cron::heartbeat::HeartbeatPromptSource::Default;
                    let effective_enabled = patch.enabled && has_prompt;

                    if let Some(hb_job) = jobs.iter().find(|j| j.id == "__heartbeat__") {
                        let job_patch = moltis_cron::types::CronJobPatch {
                            schedule: Some(moltis_cron::types::CronSchedule::Every {
                                every_ms: interval_ms,
                                anchor_ms: None,
                            }),
                            payload: Some(moltis_cron::types::CronPayload::AgentTurn {
                                message: prompt,
                                model: patch.model.clone(),
                                agent_id: patch.agent_id.clone(),
                                timeout_secs: None,
                                deliver: patch.deliver,
                                channel: patch.channel.clone(),
                                to: patch.to.clone(),
                            }),
                            enabled: Some(effective_enabled),
                            sandbox: Some(moltis_cron::types::CronSandboxConfig {
                                enabled: patch.sandbox_enabled,
                                image: patch.sandbox_image.clone(),
                                auto_prune_container: None,
                            }),
                            ..Default::default()
                        };
                        ctx.state
                            .services
                            .cron
                            .update(serde_json::json!({
                                "id": hb_job.id,
                                "patch": job_patch,
                            }))
                            .await
                            .map_err(ErrorShape::from)?;
                    } else if effective_enabled {
                        // Create the heartbeat job only when enabled with a valid prompt.
                        let create = moltis_cron::types::CronJobCreate {
                            id: Some("__heartbeat__".into()),
                            name: "__heartbeat__".into(),
                            schedule: moltis_cron::types::CronSchedule::Every {
                                every_ms: interval_ms,
                                anchor_ms: None,
                            },
                            payload: moltis_cron::types::CronPayload::AgentTurn {
                                message: prompt,
                                model: patch.model.clone(),
                                agent_id: patch.agent_id.clone(),
                                timeout_secs: None,
                                deliver: patch.deliver,
                                channel: patch.channel.clone(),
                                to: patch.to.clone(),
                            },
                            session_target: moltis_cron::types::SessionTarget::Named("heartbeat".into()),
                            delete_after_run: false,
                            enabled: effective_enabled,
                            system: true,
                            sandbox: moltis_cron::types::CronSandboxConfig {
                                enabled: patch.sandbox_enabled,
                                image: patch.sandbox_image.clone(),
                                auto_prune_container: None,
                            },
                            wake_mode: moltis_cron::types::CronWakeMode::default(),
                        };
                        let create_json = serde_json::to_value(create)
                            .map_err(|e| ErrorShape::new(error_codes::INVALID_REQUEST, format!("failed to serialize job: {e}")))?;
                        ctx.state
                            .services
                            .cron
                            .add(create_json)
                            .await
                            .map_err(ErrorShape::from)?;
                    }
                    Ok(serde_json::json!({ "updated": true }))
                })
            }),
        );
    reg.register(
        "heartbeat.run",
        Box::new(|ctx| {
            Box::pin(async move {
                let jobs_val = ctx
                    .state
                    .services
                    .cron
                    .list()
                    .await
                    .map_err(ErrorShape::from)?;
                let jobs: Vec<moltis_cron::types::CronJob> =
                    serde_json::from_value(jobs_val).unwrap_or_default();
                let hb_job = jobs
                    .iter()
                    .find(|j| j.name == "__heartbeat__")
                    .ok_or_else(|| {
                        ErrorShape::new(error_codes::INVALID_REQUEST, "heartbeat job not found")
                    })?;
                ctx.state
                    .services
                    .cron
                    .run(serde_json::json!({
                        "id": hb_job.id,
                        "force": true,
                    }))
                    .await
                    .map_err(ErrorShape::from)?;
                Ok(serde_json::json!({ "triggered": true }))
            })
        }),
    );
    reg.register(
        "heartbeat.runs",
        Box::new(|ctx| {
            Box::pin(async move {
                let jobs_val = ctx
                    .state
                    .services
                    .cron
                    .list()
                    .await
                    .map_err(ErrorShape::from)?;
                let jobs: Vec<moltis_cron::types::CronJob> =
                    serde_json::from_value(jobs_val).unwrap_or_default();
                let hb_job = jobs
                    .iter()
                    .find(|j| j.name == "__heartbeat__")
                    .ok_or_else(|| {
                        ErrorShape::new(error_codes::INVALID_REQUEST, "heartbeat job not found")
                    })?;
                let limit = ctx
                    .params
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(20);
                ctx.state
                    .services
                    .cron
                    .runs(serde_json::json!({
                        "id": hb_job.id,
                        "limit": limit,
                    }))
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    // Chat (uses chat_override if set, otherwise falls back to services.chat)
    // Inject _conn_id and _accept_language so the chat service can resolve
    // the active session and forward the user's locale to web tools.
    reg.register(
        "chat.send",
        Box::new(|ctx| {
            Box::pin(async move {
                let mut params = ctx.params.clone();
                params["_conn_id"] = serde_json::json!(ctx.client_conn_id);
                // Forward client Accept-Language, public remote IP, and timezone.
                {
                    let inner = ctx.state.inner.read().await;
                    if let Some(client) = inner.clients.get(&ctx.client_conn_id) {
                        if let Some(ref lang) = client.accept_language {
                            params["_accept_language"] = serde_json::json!(lang);
                        }
                        if let Some(ref ip) = client.remote_ip {
                            params["_remote_ip"] = serde_json::json!(ip);
                        }
                        if let Some(ref tz) = client.timezone {
                            params["_timezone"] = serde_json::json!(tz);
                        }
                    }
                }
                ctx.state
                    .chat()
                    .await
                    .send(params)
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "chat.abort",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .chat()
                    .await
                    .abort(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "chat.peek",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .chat()
                    .await
                    .peek(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "chat.cancel_queued",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .chat()
                    .await
                    .cancel_queued(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "chat.history",
        Box::new(|ctx| {
            Box::pin(async move {
                let mut params = ctx.params.clone();
                params["_conn_id"] = serde_json::json!(ctx.client_conn_id);
                ctx.state
                    .chat()
                    .await
                    .history(params)
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "chat.inject",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .chat()
                    .await
                    .inject(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "chat.clear",
        Box::new(|ctx| {
            Box::pin(async move {
                let mut params = ctx.params.clone();
                params["_conn_id"] = serde_json::json!(ctx.client_conn_id);
                ctx.state
                    .chat()
                    .await
                    .clear(params)
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "chat.compact",
        Box::new(|ctx| {
            Box::pin(async move {
                let mut params = ctx.params.clone();
                params["_conn_id"] = serde_json::json!(ctx.client_conn_id);
                ctx.state
                    .chat()
                    .await
                    .compact(params)
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    reg.register(
        "chat.context",
        Box::new(|ctx| {
            Box::pin(async move {
                let mut params = ctx.params.clone();
                params["_conn_id"] = serde_json::json!(ctx.client_conn_id);
                ctx.state
                    .chat()
                    .await
                    .context(params)
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    reg.register(
        "chat.raw_prompt",
        Box::new(|ctx| {
            Box::pin(async move {
                let mut params = ctx.params.clone();
                params["_conn_id"] = serde_json::json!(ctx.client_conn_id);
                // Forward client Accept-Language, public remote IP, and timezone.
                {
                    let inner = ctx.state.inner.read().await;
                    if let Some(client) = inner.clients.get(&ctx.client_conn_id) {
                        if let Some(ref lang) = client.accept_language {
                            params["_accept_language"] = serde_json::json!(lang);
                        }
                        if let Some(ref ip) = client.remote_ip {
                            params["_remote_ip"] = serde_json::json!(ip);
                        }
                        if let Some(ref tz) = client.timezone {
                            params["_timezone"] = serde_json::json!(tz);
                        }
                    }
                }
                ctx.state
                    .chat()
                    .await
                    .raw_prompt(params)
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    reg.register(
        "chat.full_context",
        Box::new(|ctx| {
            Box::pin(async move {
                let mut params = ctx.params.clone();
                params["_conn_id"] = serde_json::json!(ctx.client_conn_id);
                // Forward client Accept-Language, public remote IP, and timezone.
                {
                    let inner = ctx.state.inner.read().await;
                    if let Some(client) = inner.clients.get(&ctx.client_conn_id) {
                        if let Some(ref lang) = client.accept_language {
                            params["_accept_language"] = serde_json::json!(lang);
                        }
                        if let Some(ref ip) = client.remote_ip {
                            params["_remote_ip"] = serde_json::json!(ip);
                        }
                        if let Some(ref tz) = client.timezone {
                            params["_timezone"] = serde_json::json!(tz);
                        }
                    }
                }
                ctx.state
                    .chat()
                    .await
                    .full_context(params)
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    reg.register(
        "chat.prompt_memory.refresh",
        Box::new(|ctx| {
            Box::pin(async move {
                let mut params = ctx.params.clone();
                params["_conn_id"] = serde_json::json!(ctx.client_conn_id);
                ctx.state
                    .chat()
                    .await
                    .refresh_prompt_memory(params)
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    // Session switching
    reg.register(
        "sessions.switch",
        Box::new(|ctx| {
            Box::pin(async move {
                let key = ctx
                    .params
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ErrorShape::new(error_codes::INVALID_REQUEST, "missing 'key' parameter")
                    })?;
                let include_history = ctx
                    .params
                    .get("include_history")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let previous_active_key = {
                    let inner = ctx.state.inner.read().await;
                    inner.active_sessions.get(&ctx.client_conn_id).cloned()
                };
                let was_existing_session =
                    if let Some(ref metadata) = ctx.state.services.session_metadata {
                        metadata.get(key).await.is_some()
                    } else {
                        false
                    };

                // Store the active session (and project if provided) for this connection.
                {
                    let mut inner = ctx.state.inner.write().await;
                    inner
                        .active_sessions
                        .insert(ctx.client_conn_id.clone(), key.to_string());

                    if let Some(project_id) = ctx.params.get("project_id").and_then(|v| v.as_str())
                    {
                        if project_id.is_empty() {
                            inner.active_projects.remove(&ctx.client_conn_id);
                        } else {
                            inner
                                .active_projects
                                .insert(ctx.client_conn_id.clone(), project_id.to_string());
                        }
                    }
                }

                // Resolve first (auto-creates session if needed), then
                // persist project_id so the entry exists when we patch.
                let mut resolve_params = serde_json::json!({
                    "key": key,
                    "include_history": include_history,
                });
                if !was_existing_session
                    && let Some(previous_key) = previous_active_key
                        .as_deref()
                        .filter(|previous_key| *previous_key != key)
                {
                    resolve_params["inherit_agent_from"] = serde_json::json!(previous_key);
                }
                let result = ctx
                    .state
                    .services
                    .session
                    .resolve(resolve_params)
                    .await
                    .map_err(|e| {
                        tracing::error!("session resolve failed: {e}");
                        ErrorShape::new(
                            error_codes::UNAVAILABLE,
                            format!("session resolve failed: {e}"),
                        )
                    })?;

                // Mark the session as seen so unread state clears.
                ctx.state.services.session.mark_seen(key).await;

                if let Some(pid) = ctx.params.get("project_id").and_then(|v| v.as_str()) {
                    let _ = ctx
                        .state
                        .services
                        .session
                        .patch(serde_json::json!({ "key": key, "project_id": pid }))
                        .await;

                    // Auto-create worktree if project has auto_worktree enabled.
                    if let Ok(proj_val) = ctx
                        .state
                        .services
                        .project
                        .get(serde_json::json!({"id": pid}))
                        .await
                        && proj_val
                            .get("auto_worktree")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                        && let Some(dir) = proj_val.get("directory").and_then(|v| v.as_str())
                    {
                        let project_dir = Path::new(dir);
                        let create_result =
                            match moltis_projects::WorktreeManager::resolve_base_branch(project_dir)
                                .await
                            {
                                Ok(base) => {
                                    moltis_projects::WorktreeManager::create_from_base(
                                        project_dir,
                                        key,
                                        &base,
                                    )
                                    .await
                                },
                                Err(_) => {
                                    moltis_projects::WorktreeManager::create(project_dir, key).await
                                },
                            };
                        match create_result {
                            Ok(wt_dir) => {
                                let prefix = proj_val
                                    .get("branch_prefix")
                                    .and_then(|v| v.as_str())
                                    .filter(|s| !s.is_empty())
                                    .unwrap_or("moltis");
                                let branch = format!("{prefix}/{key}");
                                let _ = ctx
                                    .state
                                    .services
                                    .session
                                    .patch(serde_json::json!({
                                        "key": key,
                                        "worktree_branch": branch,
                                    }))
                                    .await;

                                if let Err(e) = moltis_projects::worktree::copy_project_config(
                                    project_dir,
                                    &wt_dir,
                                ) {
                                    tracing::warn!("failed to copy project config: {e}");
                                }

                                if let Some(cmd) = proj_val
                                    .get("setup_command")
                                    .and_then(|v| v.as_str())
                                    .filter(|s| !s.is_empty())
                                    && let Err(e) = moltis_projects::WorktreeManager::run_setup(
                                        &wt_dir,
                                        cmd,
                                        project_dir,
                                        key,
                                    )
                                    .await
                                {
                                    tracing::warn!("worktree setup failed: {e}");
                                }
                            },
                            Err(e) => {
                                tracing::warn!("auto-create worktree failed: {e}");
                            },
                        }
                    }
                }

                // If the client already has a cached history with the same
                // message count, skip sending the full history to avoid
                // transferring megabytes of data on every session switch.
                let cached_count = ctx
                    .params
                    .get("cached_message_count")
                    .and_then(|v| v.as_u64());
                let mut result = result;
                if !include_history && let Some(obj) = result.as_object_mut() {
                    obj.insert("history".to_string(), serde_json::Value::Array(Vec::new()));
                    obj.insert("historyOmitted".to_string(), serde_json::Value::Bool(true));
                    obj.remove("historyTruncated");
                    obj.remove("historyDroppedCount");
                }
                if let Some(cached) = cached_count
                    && include_history
                    && let Some(obj) = result.as_object_mut()
                    && let Some(entry_obj) = obj.get("entry").and_then(|e| e.as_object())
                    && let Some(server_count) =
                        entry_obj.get("messageCount").and_then(|v| v.as_u64())
                    && cached == server_count
                {
                    obj.insert("history".to_string(), serde_json::Value::Array(Vec::new()));
                    obj.insert("historyCacheHit".to_string(), serde_json::Value::Bool(true));
                    obj.remove("historyTruncated");
                    obj.remove("historyDroppedCount");
                }

                // Inject replying state so frontend restores thinking
                // indicator and voice-pending state after page reload.
                let chat = ctx.state.chat().await;
                let active_keys = chat.active_session_keys().await;
                let replying = active_keys.iter().any(|k| k == key);
                if let Some(obj) = result.as_object_mut() {
                    obj.insert("replying".to_string(), serde_json::Value::Bool(replying));
                    if replying {
                        if let Some(text) = chat.active_thinking_text(key).await {
                            obj.insert("thinkingText".to_string(), serde_json::Value::String(text));
                        }
                        if chat.active_voice_pending(key).await {
                            obj.insert("voicePending".to_string(), serde_json::Value::Bool(true));
                        }
                    }
                }

                Ok(result)
            })
        }),
    );

    // TTS and STT (voice feature)
    #[cfg(feature = "voice")]
    {
        reg.register(
            "tts.status",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .tts
                        .status()
                        .await
                        .map_err(ErrorShape::from)
                })
            }),
        );
        reg.register(
            "tts.providers",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .tts
                        .providers()
                        .await
                        .map_err(ErrorShape::from)
                })
            }),
        );
        reg.register(
            "tts.enable",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .tts
                        .enable(ctx.params.clone())
                        .await
                        .map_err(ErrorShape::from)
                })
            }),
        );
        reg.register(
            "tts.disable",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .tts
                        .disable()
                        .await
                        .map_err(ErrorShape::from)
                })
            }),
        );
        reg.register(
            "tts.convert",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .tts
                        .convert(ctx.params.clone())
                        .await
                        .map_err(ErrorShape::from)
                })
            }),
        );
        reg.register(
                "tts.generate_phrase",
                Box::new(|ctx| {
                    Box::pin(async move {
                        let context = ctx
                            .params
                            .get("context")
                            .and_then(|v| v.as_str())
                            .unwrap_or("settings");

                        let identity = moltis_config::resolve_identity();
                        let user = identity
                            .user_name
                            .unwrap_or_else(|| "friend".into());
                        let bot = identity.name;

                        // Try LLM generation with a 3-second timeout.
                        // Clone the Arc out so we don't hold the outer RwLock across awaits.
                        let providers = ctx.state.inner.read().await.llm_providers.clone();
                        if let Some(providers) = providers {
                            let provider = providers.read().await.first();
                            if let Some(provider) = provider {
                                let system_prompt = format!(
                                    "You generate short, funny TTS test phrases for a voice assistant.\n\
                                     The user's name is {user}. The bot's name is {bot}.\n\
                                     Include SSML <break time=\"0.5s\"/> tags for natural pauses.\n\
                                     Reply with ONLY the phrase text — no quotes, no markdown. Under 200 chars."
                                );
                                let messages = vec![
                                    moltis_agents::model::ChatMessage::system(system_prompt),
                                    moltis_agents::model::ChatMessage::user(format!(
                                        "Generate a {context} TTS test phrase."
                                    )),
                                ];
                                let result = tokio::time::timeout(
                                    std::time::Duration::from_secs(3),
                                    provider.complete(&messages, &[]),
                                )
                                .await;

                                if let Ok(Ok(response)) = result
                                    && let Some(text) = response.text
                                {
                                    let text = text.trim().to_string();
                                    if !text.is_empty() {
                                        return Ok(serde_json::json!({
                                            "phrase": text,
                                            "source": "llm",
                                        }));
                                    }
                                }
                            }
                        }

                        // Fall back to static phrases with sequential picking.
                        let phrases =
                            crate::tts_phrases::static_phrases(&user, &bot, context);
                        let idx = ctx.state.next_tts_phrase_index(phrases.len());
                        let phrase = phrases
                            .into_iter()
                            .nth(idx)
                            .unwrap_or_default();

                        Ok(serde_json::json!({
                            "phrase": phrase,
                            "source": "static",
                        }))
                    })
                }),
            );
        reg.register(
            "tts.setProvider",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .tts
                        .set_provider(ctx.params.clone())
                        .await
                        .map_err(ErrorShape::from)
                })
            }),
        );
        reg.register(
            "stt.status",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .stt
                        .status()
                        .await
                        .map_err(ErrorShape::from)
                })
            }),
        );
        reg.register(
            "stt.providers",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .stt
                        .providers()
                        .await
                        .map_err(ErrorShape::from)
                })
            }),
        );
        reg.register(
            "stt.transcribe",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .stt
                        .transcribe(ctx.params.clone())
                        .await
                        .map_err(ErrorShape::from)
                })
            }),
        );
        reg.register(
            "stt.setProvider",
            Box::new(|ctx| {
                Box::pin(async move {
                    ctx.state
                        .services
                        .stt
                        .set_provider(ctx.params.clone())
                        .await
                        .map_err(ErrorShape::from)
                })
            }),
        );
    }
}
