use super::*;

pub(super) fn register(reg: &mut MethodRegistry) {
    // Agent
    reg.register(
        "agent",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .agent
                    .run(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "agent.wait",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .agent
                    .run_wait(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "agent.identity.get",
        Box::new(|ctx| {
            Box::pin(async move {
                let agent_id = resolve_session_agent_id_for_ctx(&ctx).await;
                Ok(read_identity_payload_for_agent(&agent_id))
            })
        }),
    );
    reg.register(
        "agent.identity.update",
        Box::new(|ctx| {
            Box::pin(async move {
                let agent_id = resolve_session_agent_id_for_ctx(&ctx).await;
                let identity = moltis_config::schema::AgentIdentity {
                    name: ctx
                        .params
                        .get("name")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    emoji: ctx
                        .params
                        .get("emoji")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    theme: ctx
                        .params
                        .get("theme")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                };
                moltis_config::save_identity_for_agent(&agent_id, &identity)
                    .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))?;
                // Handle soul if present.
                if let Some(soul_val) = ctx.params.get("soul") {
                    let soul = if soul_val.is_null() {
                        None
                    } else {
                        soul_val.as_str().map(str::to_string)
                    };
                    write_soul_for_agent(&agent_id, soul)?;
                }
                // Handle user profile fields (user_name, user_timezone, user_location).
                save_user_profile_fields(&ctx.params)?;
                // Mark onboarding complete when both agent name and user name are present
                // (mirrors the old onboarding.identity_update behavior).
                mark_onboarded_if_ready(&identity, &ctx.params);
                // Sync persona DB row if persona store is available.
                if let Some(ref store) = ctx.state.services.agent_persona_store {
                    let _ = store
                        .update(&agent_id, crate::agent_persona::UpdateAgentParams {
                            name: identity.name.clone(),
                            emoji: identity.emoji.clone(),
                            theme: identity.theme.clone(),
                            description: None,
                            voice_persona_id: None,
                        })
                        .await;
                }
                Ok(read_identity_payload_for_agent(&agent_id))
            })
        }),
    );
    reg.register(
        "agent.identity.update_soul",
        Box::new(|ctx| {
            Box::pin(async move {
                let soul = ctx
                    .params
                    .get("soul")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let agent_id = resolve_session_agent_id_for_ctx(&ctx).await;
                write_soul_for_agent(&agent_id, soul)?;
                Ok(serde_json::json!({ "ok": true }))
            })
        }),
    );
    reg.register(
        "agents.list",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .agent
                    .list()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    #[cfg(feature = "agent")]
    {
        reg.register(
            "agents.list",
            Box::new(|ctx| {
                Box::pin(async move {
                    let Some(ref store) = ctx.state.services.agent_persona_store else {
                        return Err(ErrorShape::new(
                            error_codes::UNAVAILABLE,
                            "agent personas not available",
                        ));
                    };
                    let default_id = store.default_id().await.map_err(ErrorShape::from)?;
                    let limit_chars = workspace_file_limit_chars(&ctx);
                    let agents = store
                        .list()
                        .await
                        .map_err(ErrorShape::from)?
                        .into_iter()
                        .map(|agent| {
                            let agent_id = agent.id.clone();
                            let mut value = serde_json::to_value(agent)
                                .unwrap_or_else(|_| serde_json::json!({}));
                            if let Some(obj) = value.as_object_mut() {
                                obj.insert(
                                    "workspace_prompt_files".to_string(),
                                    serde_json::Value::Array(workspace_prompt_files_status(
                                        &agent_id,
                                        limit_chars,
                                    )),
                                );
                            }
                            value
                        })
                        .collect::<Vec<_>>();
                    Ok(serde_json::json!({
                        "default_id": default_id,
                        "agents": agents,
                    }))
                })
            }),
        );
        reg.register(
            "agents.get",
            Box::new(|ctx| {
                Box::pin(async move {
                    let id = parse_agent_id_param(&ctx.params).ok_or_else(|| {
                        ErrorShape::new(
                            error_codes::INVALID_REQUEST,
                            "missing 'id' or 'agent_id' parameter",
                        )
                    })?;
                    let Some(ref store) = ctx.state.services.agent_persona_store else {
                        return Err(ErrorShape::new(
                            error_codes::UNAVAILABLE,
                            "agent personas not available",
                        ));
                    };
                    let Some(agent) = store.get(&id).await.map_err(ErrorShape::from)? else {
                        return Err(ErrorShape::new(
                            error_codes::INVALID_REQUEST,
                            "agent not found",
                        ));
                    };

                    let mut payload = serde_json::to_value(agent)
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))?;
                    let limit_chars = workspace_file_limit_chars(&ctx);
                    if let Some(obj) = payload.as_object_mut() {
                        obj.insert(
                            "identity_fields".to_string(),
                            serde_json::json!(
                                moltis_config::load_identity_for_agent(&id).unwrap_or_default()
                            ),
                        );
                        obj.insert(
                            "soul".to_string(),
                            serde_json::json!(moltis_config::load_soul_for_agent(&id)),
                        );
                        obj.insert(
                            "default_id".to_string(),
                            serde_json::json!(
                                store
                                    .default_id()
                                    .await
                                    .unwrap_or_else(|_| "main".to_string())
                            ),
                        );
                        obj.insert(
                            "workspace_prompt_files".to_string(),
                            serde_json::Value::Array(workspace_prompt_files_status(
                                &id,
                                limit_chars,
                            )),
                        );
                    }
                    Ok(payload)
                })
            }),
        );
        reg.register(
            "agents.create",
            Box::new(|ctx| {
                Box::pin(async move {
                    let Some(ref store) = ctx.state.services.agent_persona_store else {
                        return Err(ErrorShape::new(
                            error_codes::UNAVAILABLE,
                            "agent personas not available",
                        ));
                    };
                    let params: crate::agent_persona::CreateAgentParams =
                        serde_json::from_value(ctx.params).map_err(|e| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, e.to_string())
                        })?;
                    let agent = store.create(params).await.map_err(ErrorShape::from)?;
                    // Sync persona into shared agents_config presets.
                    if let Some(ref agents_config) = ctx.state.services.agents_config {
                        let mut guard = agents_config.write().await;
                        crate::server::sync_persona_into_preset(&mut guard, &agent);
                    }
                    serde_json::to_value(&agent)
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))
                })
            }),
        );
        reg.register(
            "agents.update",
            Box::new(|ctx| {
                Box::pin(async move {
                    let id = parse_agent_id_param(&ctx.params).ok_or_else(|| {
                        ErrorShape::new(
                            error_codes::INVALID_REQUEST,
                            "missing 'id' or 'agent_id' parameter",
                        )
                    })?;
                    let Some(ref store) = ctx.state.services.agent_persona_store else {
                        return Err(ErrorShape::new(
                            error_codes::UNAVAILABLE,
                            "agent personas not available",
                        ));
                    };
                    let params: crate::agent_persona::UpdateAgentParams =
                        serde_json::from_value(ctx.params).map_err(|e| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, e.to_string())
                        })?;
                    let agent = store.update(&id, params).await.map_err(ErrorShape::from)?;
                    // Sync updated persona into shared agents_config presets.
                    if let Some(ref agents_config) = ctx.state.services.agents_config {
                        let mut guard = agents_config.write().await;
                        crate::server::sync_persona_into_preset(&mut guard, &agent);
                    }
                    serde_json::to_value(&agent)
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))
                })
            }),
        );
        reg.register(
            "agents.delete",
            Box::new(|ctx| {
                Box::pin(async move {
                    let id = parse_agent_id_param(&ctx.params).ok_or_else(|| {
                        ErrorShape::new(
                            error_codes::INVALID_REQUEST,
                            "missing 'id' or 'agent_id' parameter",
                        )
                    })?;
                    let Some(ref store) = ctx.state.services.agent_persona_store else {
                        return Err(ErrorShape::new(
                            error_codes::UNAVAILABLE,
                            "agent personas not available",
                        ));
                    };
                    let fallback_default_id = store.default_id().await.map_err(ErrorShape::from)?;
                    let mut reassigned_sessions = 0_u64;
                    if let Some(ref meta) = ctx.state.services.session_metadata {
                        let sessions = meta.list_by_agent_id(&id).await.map_err(|e| {
                            ErrorShape::new(error_codes::UNAVAILABLE, e.to_string())
                        })?;
                        for session in sessions {
                            meta.set_agent_id(&session.key, Some(&fallback_default_id))
                                .await
                                .map_err(|e| {
                                    ErrorShape::new(error_codes::UNAVAILABLE, e.to_string())
                                })?;
                            reassigned_sessions = reassigned_sessions.saturating_add(1);
                        }
                    }
                    store.delete(&id).await.map_err(ErrorShape::from)?;
                    // Remove preset for deleted persona from shared agents_config.
                    if let Some(ref agents_config) = ctx.state.services.agents_config {
                        let mut guard = agents_config.write().await;
                        guard.presets.remove(&id);
                    }
                    Ok(serde_json::json!({
                        "deleted": true,
                        "reassigned_sessions": reassigned_sessions,
                        "default_id": fallback_default_id,
                    }))
                })
            }),
        );
        reg.register(
            "agents.set_default",
            Box::new(|ctx| {
                Box::pin(async move {
                    let id = parse_agent_id_param(&ctx.params).ok_or_else(|| {
                        ErrorShape::new(
                            error_codes::INVALID_REQUEST,
                            "missing 'id' or 'agent_id' parameter",
                        )
                    })?;
                    let Some(ref store) = ctx.state.services.agent_persona_store else {
                        return Err(ErrorShape::new(
                            error_codes::UNAVAILABLE,
                            "agent personas not available",
                        ));
                    };
                    let default_id = store.set_default(&id).await.map_err(ErrorShape::from)?;
                    Ok(serde_json::json!({
                        "ok": true,
                        "default_id": default_id,
                    }))
                })
            }),
        );
        reg.register(
            "agents.set_session",
            Box::new(|ctx| {
                Box::pin(async move {
                    let session_key = ctx
                        .params
                        .get("session_key")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(
                                error_codes::INVALID_REQUEST,
                                "missing 'session_key' parameter",
                            )
                        })?;
                    let agent_id = if let Some(agent_id) = parse_agent_id_param(&ctx.params) {
                        if !agent_exists_for_ctx(&ctx, &agent_id).await {
                            return Err(ErrorShape::new(
                                error_codes::INVALID_REQUEST,
                                format!("agent '{agent_id}' not found"),
                            ));
                        }
                        agent_id
                    } else {
                        default_agent_id_for_ctx(&ctx).await
                    };
                    let Some(ref meta) = ctx.state.services.session_metadata else {
                        return Err(ErrorShape::new(
                            error_codes::UNAVAILABLE,
                            "session metadata not available",
                        ));
                    };
                    meta.upsert(session_key, None)
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))?;
                    meta.set_agent_id(session_key, Some(&agent_id))
                        .await
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))?;
                    Ok(serde_json::json!({ "ok": true, "agent_id": agent_id }))
                })
            }),
        );
        reg.register(
            "agents.identity.get",
            Box::new(|ctx| {
                Box::pin(async move {
                    let agent_id = resolve_requested_agent_id(&ctx, &ctx.params).await?;
                    Ok(read_identity_payload_for_agent(&agent_id))
                })
            }),
        );
        reg.register(
            "agents.identity.update",
            Box::new(|ctx| {
                Box::pin(async move {
                    let agent_id = resolve_requested_agent_id(&ctx, &ctx.params).await?;
                    let identity = moltis_config::schema::AgentIdentity {
                        name: ctx
                            .params
                            .get("name")
                            .and_then(|v| v.as_str())
                            .map(String::from),
                        emoji: ctx
                            .params
                            .get("emoji")
                            .and_then(|v| v.as_str())
                            .map(String::from),
                        theme: ctx
                            .params
                            .get("theme")
                            .and_then(|v| v.as_str())
                            .map(String::from),
                    };
                    moltis_config::save_identity_for_agent(&agent_id, &identity)
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))?;
                    // Handle soul if present.
                    if let Some(soul_val) = ctx.params.get("soul") {
                        let soul = if soul_val.is_null() {
                            None
                        } else {
                            soul_val.as_str().map(str::to_string)
                        };
                        write_soul_for_agent(&agent_id, soul)?;
                    }
                    // Handle user profile fields.
                    save_user_profile_fields(&ctx.params)?;
                    // Mark onboarding complete when both names are present.
                    mark_onboarded_if_ready(&identity, &ctx.params);
                    // Sync persona DB row.
                    if let Some(ref store) = ctx.state.services.agent_persona_store {
                        let _ = store
                            .update(&agent_id, crate::agent_persona::UpdateAgentParams {
                                name: identity.name.clone(),
                                emoji: identity.emoji.clone(),
                                theme: identity.theme.clone(),
                                description: None,
                                voice_persona_id: None,
                            })
                            .await;
                    }
                    // Sync identity into preset.
                    if let Some(ref agents_config) = ctx.state.services.agents_config {
                        let mut guard = agents_config.write().await;
                        if let Some(entry) = guard.presets.get_mut(&agent_id) {
                            entry.identity = identity;
                        }
                    }
                    Ok(read_identity_payload_for_agent(&agent_id))
                })
            }),
        );
        reg.register(
            "agents.identity.update_soul",
            Box::new(|ctx| {
                Box::pin(async move {
                    let agent_id = resolve_requested_agent_id(&ctx, &ctx.params).await?;
                    let soul = ctx
                        .params
                        .get("soul")
                        .and_then(|v| v.as_str())
                        .map(str::to_string);
                    write_soul_for_agent(&agent_id, soul.clone())?;
                    // Sync soul into preset's system_prompt_suffix.
                    if let Some(ref agents_config) = ctx.state.services.agents_config {
                        let mut guard = agents_config.write().await;
                        if let Some(entry) = guard.presets.get_mut(&agent_id) {
                            entry.system_prompt_suffix = soul.filter(|s| !s.trim().is_empty());
                        }
                    }
                    Ok(serde_json::json!({ "ok": true }))
                })
            }),
        );
        reg.register(
            "agents.files.list",
            Box::new(|ctx| {
                Box::pin(async move {
                    let agent_id = resolve_requested_agent_id(&ctx, &ctx.params).await?;
                    let limit_chars = workspace_file_limit_chars(&ctx);
                    let mut files: Vec<serde_json::Value> = Vec::new();
                    let root = moltis_config::agent_workspace_dir(&agent_id);
                    let root_exists = root.exists();
                    if root_exists {
                        list_agent_workspace_files_recursively(&root, &root, &mut files);
                    }
                    for file_name in &[
                        "IDENTITY.md",
                        "SOUL.md",
                        "MEMORY.md",
                        "AGENTS.md",
                        "TOOLS.md",
                    ] {
                        let relative_path = Path::new(file_name);
                        if !should_fallback_agent_file_to_root(&agent_id, relative_path) {
                            continue;
                        }
                        let agent_path = root.join(file_name);
                        let root_path = moltis_config::data_dir().join(file_name);
                        if !agent_path.exists() && root_path.exists() {
                            let mut entry = serde_json::json!({
                                "path": file_name,
                                "source": "root",
                                "size": std::fs::metadata(&root_path).ok().map(|m| m.len()),
                            });
                            if matches!(*file_name, "AGENTS.md" | "TOOLS.md")
                                && let Some(obj) = entry.as_object_mut()
                                && let Some(status) =
                                    workspace_prompt_file_status(&agent_id, file_name, limit_chars)
                                && let Ok(status_value) = serde_json::to_value(status)
                                && let Some(status_obj) = status_value.as_object()
                            {
                                for (key, value) in status_obj {
                                    if key != "path" && key != "source" && key != "size" {
                                        obj.insert(key.clone(), value.clone());
                                    }
                                }
                            }
                            files.push(entry);
                        }
                    }
                    files.sort_by(|left, right| {
                        let left_path = left
                            .get("path")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default();
                        let right_path = right
                            .get("path")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default();
                        left_path.cmp(right_path)
                    });
                    Ok(serde_json::json!({
                        "agent_id": agent_id,
                        "files": files,
                    }))
                })
            }),
        );
        reg.register(
            "agents.files.get",
            Box::new(|ctx| {
                Box::pin(async move {
                    let agent_id = resolve_requested_agent_id(&ctx, &ctx.params).await?;
                    let relative_path = normalize_relative_agent_path(
                        ctx.params
                            .get("path")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                ErrorShape::new(
                                    error_codes::INVALID_REQUEST,
                                    "missing 'path' parameter",
                                )
                            })?,
                    )?;
                    let content = read_agent_file(&agent_id, &relative_path)?;
                    Ok(serde_json::json!({
                        "agent_id": agent_id,
                        "path": relative_path.to_string_lossy(),
                        "content": content,
                    }))
                })
            }),
        );
        reg.register(
            "agents.files.set",
            Box::new(|ctx| {
                Box::pin(async move {
                    let agent_id = resolve_requested_agent_id(&ctx, &ctx.params).await?;
                    let relative_path = normalize_relative_agent_path(
                        ctx.params
                            .get("path")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                ErrorShape::new(
                                    error_codes::INVALID_REQUEST,
                                    "missing 'path' parameter",
                                )
                            })?,
                    )?;
                    let content = ctx
                        .params
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    let full_path =
                        moltis_config::agent_workspace_dir(&agent_id).join(&relative_path);
                    if let Some(parent) = full_path.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| {
                            ErrorShape::new(error_codes::UNAVAILABLE, e.to_string())
                        })?;
                    }
                    std::fs::write(&full_path, content)
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))?;

                    Ok(serde_json::json!({
                        "ok": true,
                        "agent_id": agent_id,
                        "path": relative_path.to_string_lossy(),
                    }))
                })
            }),
        );
        reg.register(
            "agents.preset.get",
            Box::new(|ctx| {
                Box::pin(async move {
                    let id = parse_agent_id_param(&ctx.params).ok_or_else(|| {
                        ErrorShape::new(
                            error_codes::INVALID_REQUEST,
                            "missing 'id' or 'agent_id' parameter",
                        )
                    })?;
                    let config = moltis_config::discover_and_load_readonly();
                    let toml_str = match config.agents.presets.get(&id) {
                        Some(preset) => toml::to_string_pretty(preset).unwrap_or_default(),
                        None => String::new(),
                    };
                    let provenance =
                        moltis_config::defaults::compute_preset_provenance(&config.agents);
                    let source = provenance
                        .iter()
                        .find(|p| p.id == id)
                        .map(|p| p.source)
                        .unwrap_or(moltis_config::defaults::ConfigSource::Custom);
                    // Return structured fields alongside TOML for UI controls.
                    let preset_fields = config.agents.presets.get(&id).map(|p| {
                        let mcp = match &p.mcp {
                            moltis_config::schema::PresetMcpPolicy::All => serde_json::json!({
                                "mode": "all"
                            }),
                            moltis_config::schema::PresetMcpPolicy::Allow(servers) => serde_json::json!({
                                "mode": "allow",
                                "servers": servers.iter().map(|s| s.as_str()).collect::<Vec<&str>>()
                            }),
                            moltis_config::schema::PresetMcpPolicy::Deny(servers) => serde_json::json!({
                                "mode": "deny",
                                "servers": servers.iter().map(|s| s.as_str()).collect::<Vec<&str>>()
                            }),
                        };
                        serde_json::json!({
                            "model": p.model,
                            "mcp": mcp,
                            "sandbox": {
                                "mode": p.sandbox.mode,
                            },
                            "skills": {
                                "allow": p.skills.allow,
                                "deny": p.skills.deny,
                            },
                        })
                    });
                    Ok(serde_json::json!({
                        "id": id,
                        "toml": toml_str,
                        "exists": !toml_str.is_empty(),
                        "provenance": source,
                        "fields": preset_fields,
                    }))
                })
            }),
        );
        reg.register(
            "agents.preset.update",
            Box::new(|ctx| {
                Box::pin(async move {
                    let id = parse_agent_id_param(&ctx.params).ok_or_else(|| {
                        ErrorShape::new(
                            error_codes::INVALID_REQUEST,
                            "missing 'id' or 'agent_id' parameter",
                        )
                    })?;
                    validate_preset_id(&id)?;
                    reject_toml_backed_preset_update(&id)?;
                    let config = moltis_config::discover_and_load_readonly();
                    let preset =
                        preset_from_rpc_params(&id, &ctx.params, config.agents.presets.get(&id))?;
                    let path = moltis_config::agent_defs::write_user_agent_def(&id, &preset)
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))?;
                    refresh_agents_config(&ctx).await;
                    Ok(serde_json::json!({
                        "ok": true,
                        "id": id,
                        "path": path.to_string_lossy(),
                    }))
                })
            }),
        );
        reg.register(
            "agents.preset.create",
            Box::new(|ctx| {
                Box::pin(async move {
                    let id = parse_agent_id_param(&ctx.params).ok_or_else(|| {
                        ErrorShape::new(
                            error_codes::INVALID_REQUEST,
                            "missing 'id' or 'agent_id' parameter",
                        )
                    })?;
                    validate_preset_id(&id)?;
                    let config = moltis_config::discover_and_load_readonly();
                    if let Some(existing) = config.agents.presets.get(&id)
                        && !moltis_config::schema::is_default_agent_preset(&id, existing)
                    {
                        return Err(ErrorShape::new(
                            error_codes::INVALID_REQUEST,
                            format!("preset '{id}' already exists"),
                        ));
                    }
                    let preset = preset_from_rpc_params(&id, &ctx.params, None)?;
                    let path = moltis_config::agent_defs::write_user_agent_def(&id, &preset)
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))?;
                    refresh_agents_config(&ctx).await;
                    Ok(serde_json::json!({
                        "ok": true,
                        "id": id,
                        "path": path.to_string_lossy(),
                    }))
                })
            }),
        );
        reg.register(
            "agents.preset.delete",
            Box::new(|ctx| {
                Box::pin(async move {
                    let id = parse_agent_id_param(&ctx.params).ok_or_else(|| {
                        ErrorShape::new(
                            error_codes::INVALID_REQUEST,
                            "missing 'id' or 'agent_id' parameter",
                        )
                    })?;
                    validate_preset_id(&id)?;
                    let deleted = moltis_config::agent_defs::delete_user_agent_def(&id)
                        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))?;
                    refresh_agents_config(&ctx).await;
                    Ok(serde_json::json!({ "ok": true, "id": id, "deleted": deleted }))
                })
            }),
        );
        reg.register(
            "agents.preset.save",
            Box::new(|ctx| {
                Box::pin(async move {
                    let id = parse_agent_id_param(&ctx.params).ok_or_else(|| {
                        ErrorShape::new(
                            error_codes::INVALID_REQUEST,
                            "missing 'id' or 'agent_id' parameter",
                        )
                    })?;
                    let toml_str = ctx
                        .params
                        .get("toml")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    // Parse the TOML as a partial AgentPreset to validate it
                    let partial: moltis_config::AgentPreset = if toml_str.trim().is_empty() {
                        moltis_config::AgentPreset::default()
                    } else {
                        toml::from_str(&toml_str).map_err(|e| {
                            ErrorShape::new(
                                error_codes::INVALID_REQUEST,
                                format!("invalid TOML: {e}"),
                            )
                        })?
                    };

                    // Write to moltis.toml using update_config
                    moltis_config::update_config(|cfg| {
                        if toml_str.trim().is_empty() {
                            cfg.agents.presets.remove(&id);
                        } else {
                            // Merge: keep existing identity fields from persona if present,
                            // let TOML fields override everything else.
                            if let Some(existing) = cfg.agents.presets.get(&id) {
                                let mut merged = partial.clone();
                                // Preserve persona identity if TOML didn't set it
                                if merged.identity.name.is_none() {
                                    merged.identity.name = existing.identity.name.clone();
                                }
                                if merged.identity.emoji.is_none() {
                                    merged.identity.emoji = existing.identity.emoji.clone();
                                }
                                if merged.identity.theme.is_none() {
                                    merged.identity.theme = existing.identity.theme.clone();
                                }
                                cfg.agents.presets.insert(id.clone(), merged);
                            } else {
                                cfg.agents.presets.insert(id.clone(), partial);
                            }
                        }
                    })
                    .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))?;

                    // Refresh in-memory agents_config if available
                    if let Some(ref agents_config) = ctx.state.services.agents_config {
                        let fresh = moltis_config::discover_and_load();
                        let mut guard = agents_config.write().await;
                        *guard = fresh.agents;
                    }

                    Ok(serde_json::json!({ "ok": true, "id": id }))
                })
            }),
        );
        reg.register(
            "agents.presets_list",
            Box::new(|ctx| {
                Box::pin(async move {
                    let config = moltis_config::discover_and_load_readonly();
                    let toml_config =
                        moltis_config::discover_and_load_readonly_without_agent_defs();
                    let persona_ids: std::collections::HashSet<String> =
                        if let Some(ref store) = ctx.state.services.agent_persona_store {
                            store
                                .list()
                                .await
                                .map_err(ErrorShape::from)?
                                .into_iter()
                                .map(|a| a.id)
                                .collect()
                        } else {
                            std::collections::HashSet::new()
                        };

                    let all_provenance =
                        moltis_config::defaults::compute_preset_provenance(&config.agents);
                    let config_only: Vec<serde_json::Value> = config
                        .agents
                        .presets
                        .iter()
                        .filter(|(name, _)| !persona_ids.contains(*name))
                        .map(|(name, preset)| {
                            let toml_str = toml::to_string_pretty(preset).unwrap_or_default();
                            let markdown_path = moltis_config::data_dir()
                                .join("agents")
                                .join(format!("{name}.md"));
                            let markdown_backed = markdown_path.exists();
                            let toml_backed = toml_config.agents.presets.get(name).is_some_and(|existing| {
                                !moltis_config::schema::is_default_agent_preset(name, existing)
                            });
                            let provenance = all_provenance
                                .iter()
                                .find(|p| &p.id == name)
                                .map(|p| p.source)
                                .unwrap_or(moltis_config::defaults::ConfigSource::Custom);
                            serde_json::json!({
                                "id": name,
                                "name": preset.identity.name.as_deref().unwrap_or(name),
                                "emoji": preset.identity.emoji,
                                "theme": preset.identity.theme,
                                "model": preset.model,
                                "system_prompt_suffix": preset.system_prompt_suffix,
                                "tools_allow": preset.tools.allow,
                                "tools_deny": preset.tools.deny,
                                "delegate_only": preset.delegate_only,
                                "toml": toml_str,
                                "provenance": provenance,
                                "deletable": markdown_backed && !toml_backed,
                                "toml_backed": toml_backed,
                                "path": markdown_backed.then(|| markdown_path.to_string_lossy().to_string()),
                            })
                        })
                        .collect();

                    Ok(serde_json::json!({ "presets": config_only }))
                })
            }),
        );
    }
}

#[cfg(feature = "agent")]
fn validate_preset_id(id: &str) -> Result<(), ErrorShape> {
    let valid = !id.is_empty()
        && id.len() <= 80
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if valid {
        Ok(())
    } else {
        Err(ErrorShape::new(
            error_codes::INVALID_REQUEST,
            "preset id must use lowercase letters, numbers, and hyphens",
        ))
    }
}

#[cfg(feature = "agent")]
fn preset_from_rpc_params(
    id: &str,
    params: &serde_json::Value,
    base: Option<&moltis_config::AgentPreset>,
) -> Result<moltis_config::AgentPreset, ErrorShape> {
    if let Some(toml_str) = params.get("toml").and_then(|value| value.as_str())
        && !toml_str.trim().is_empty()
    {
        return toml::from_str(toml_str).map_err(|e| {
            ErrorShape::new(error_codes::INVALID_REQUEST, format!("invalid TOML: {e}"))
        });
    }

    let mut preset = base.cloned().unwrap_or_default();
    if params.get("name").is_some() {
        preset.identity.name = optional_string(params, "name").or_else(|| Some(id.to_string()));
    } else if preset.identity.name.is_none() {
        preset.identity.name = Some(id.to_string());
    }
    if params.get("emoji").is_some() {
        preset.identity.emoji = optional_string(params, "emoji");
    }
    if params.get("theme").is_some() {
        preset.identity.theme = optional_string(params, "theme");
    }
    if params.get("model").is_some() {
        preset.model = optional_string(params, "model");
    }
    if params.get("system_prompt_suffix").is_some() || params.get("soul").is_some() {
        preset.system_prompt_suffix = optional_string(params, "system_prompt_suffix")
            .or_else(|| optional_string(params, "soul"));
    }
    if let Some(delegate_only) = params
        .get("delegate_only")
        .and_then(serde_json::Value::as_bool)
    {
        preset.delegate_only = delegate_only;
    }
    if params.get("tools_allow").is_some() {
        preset.tools.allow = string_list_param(params, "tools_allow");
    }
    if params.get("tools_deny").is_some() {
        preset.tools.deny = string_list_param(params, "tools_deny");
    }
    if params.get("max_iterations").is_some() {
        preset.max_iterations = params
            .get("max_iterations")
            .and_then(serde_json::Value::as_u64);
    }
    if params.get("timeout_secs").is_some() {
        preset.timeout_secs = params
            .get("timeout_secs")
            .and_then(serde_json::Value::as_u64);
    }
    if let Some(re) = optional_string(params, "reasoning_effort") {
        preset.reasoning_effort = Some(re.as_str().try_into().map_err(parse_preset_param_error)?);
    }
    if params.get("mcp_mode").is_some() || params.get("mcp_servers").is_some() {
        preset.mcp = parse_mcp_policy_param(params);
    }
    if let Some(mode) = optional_string(params, "sandbox_mode") {
        preset.sandbox.mode = Some(mode.as_str().try_into().map_err(parse_preset_param_error)?);
    }
    if params.get("skills_allow").is_some() {
        preset.skills.allow = Some(string_list_param(params, "skills_allow"));
    }
    if params.get("skills_deny").is_some() {
        let skills_deny = string_list_param(params, "skills_deny");
        preset.skills.deny = if skills_deny.is_empty() {
            None
        } else {
            Some(skills_deny)
        };
    }
    Ok(preset)
}

#[cfg(feature = "agent")]
fn reject_toml_backed_preset_update(id: &str) -> Result<(), ErrorShape> {
    let config = moltis_config::discover_and_load_readonly_without_agent_defs();
    if let Some(existing) = config.agents.presets.get(id)
        && !moltis_config::schema::is_default_agent_preset(id, existing)
    {
        return Err(ErrorShape::new(
            error_codes::INVALID_REQUEST,
            format!(
                "preset '{id}' is defined in moltis.toml; edit moltis.toml or remove that preset before using Web UI markdown overrides"
            ),
        ));
    }

    let markdown_path = moltis_config::data_dir()
        .join("agents")
        .join(format!("{id}.md"));
    if markdown_path.exists() {
        return Ok(());
    }
    Ok(())
}

#[cfg(feature = "agent")]
fn optional_string(params: &serde_json::Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(feature = "agent")]
fn string_list_param(params: &serde_json::Value, key: &str) -> Vec<String> {
    params
        .get(key)
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(feature = "agent")]
fn parse_preset_param_error(message: String) -> ErrorShape {
    ErrorShape::new(error_codes::INVALID_REQUEST, message)
}

#[cfg(feature = "agent")]
fn parse_mcp_policy_param(params: &serde_json::Value) -> moltis_config::schema::PresetMcpPolicy {
    use moltis_config::schema::{McpServerId, PresetMcpPolicy};
    let mode = params
        .get("mcp_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("all");
    let servers: Vec<McpServerId> = string_list_param(params, "mcp_servers")
        .into_iter()
        .map(McpServerId::from)
        .collect();
    match mode {
        "allow" => PresetMcpPolicy::Allow(servers),
        "deny" => PresetMcpPolicy::Deny(servers),
        _ => PresetMcpPolicy::All,
    }
}

#[cfg(feature = "agent")]
async fn refresh_agents_config(ctx: &MethodContext) {
    if let Some(ref agents_config) = ctx.state.services.agents_config {
        let fresh = moltis_config::discover_and_load();
        let mut guard = agents_config.write().await;
        *guard = fresh.agents;
    }
}
