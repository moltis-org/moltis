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
                    // The new agent may force its sandbox where the old one did
                    // not, or the other way round. The caller is holding a
                    // toggle whose enabled-ness is the old agent's answer, and
                    // this reply is the only one it gets for the switch, so it
                    // carries the new answer rather than making the caller
                    // guess or re-list.
                    let sandbox_forced =
                        crate::sandbox_policy::agent_sandbox_forced(Some(&agent_id));
                    Ok(serde_json::json!({
                        "ok": true,
                        "agent_id": agent_id,
                        "sandbox_forced": sandbox_forced,
                    }))
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
                            "sandbox": preset_sandbox_fields(&p.sandbox),
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
    // The flat spelling of `[sandbox] force`, matching `sandbox_mode` and
    // `sandbox_mounts`. A present-but-not-boolean value is a typo, not a
    // request to force the sandbox, and a silent `false` would be the one
    // direction this field must never fail in.
    if let Some(force) = params.get("sandbox_force") {
        preset.sandbox.force = match force {
            serde_json::Value::Null => false,
            value => value.as_bool().ok_or_else(|| {
                parse_preset_param_error("sandbox_force must be a boolean or null".to_string())
            })?,
        };
    }
    if params.get("sandbox_mounts").is_some() {
        preset.sandbox.mounts = parse_sandbox_mounts_param(params)?;
    }
    if params.get("run_as").is_some() {
        preset.sandbox.run_as = parse_run_as_param(params)?;
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

/// The `sandbox` block of an `agents.preset.get` response.
///
/// Mounts come back in the same array-of-triples spelling the write surface
/// accepts, so `get` after `update` reads back what was written.
#[cfg(feature = "agent")]
fn preset_sandbox_fields(
    sandbox: &moltis_config::schema::PresetSandboxPolicy,
) -> serde_json::Value {
    serde_json::json!({
        "mode": sandbox.mode,
        "force": sandbox.force,
        "run_as": sandbox.run_as,
        "mounts": sandbox
            .mounts
            .iter()
            .map(moltis_config::schema::SandboxMountConfig::to_triple)
            .collect::<Vec<_>>(),
    })
}

/// Read `run_as` with its own parse rather than [`optional_string`].
///
/// `optional_string` drops an empty or non-string value silently, and a dropped
/// `run_as` means the container runs as root - the exact outcome this field
/// exists to prevent. So anything present that is not a well-formed `uid:gid`
/// is a hard error here, with two deliberate exceptions that both mean the
/// same thing: an explicit empty string and an explicit JSON `null` clear the
/// field, the way an empty `sandbox_mounts` array clears the mounts. Both are
/// an operator removing the setting, not a silent fallback - and `null` is
/// what a client that models the field as an optional string sends, so
/// rejecting it while accepting `""` was a shape rule, not a safety one.
#[cfg(feature = "agent")]
fn parse_run_as_param(params: &serde_json::Value) -> Result<Option<String>, ErrorShape> {
    if params.get("run_as").is_none_or(serde_json::Value::is_null) {
        return Ok(None);
    }
    let Some(raw) = params.get("run_as").and_then(serde_json::Value::as_str) else {
        return Err(parse_preset_param_error(
            "run_as must be a \"uid:gid\" string or null".to_string(),
        ));
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    moltis_config::schema::check_run_as(raw).map_err(parse_preset_param_error)?;
    Ok(Some(raw.to_string()))
}

/// Read `sandbox_mounts` with its own parse rather than [`string_list_param`].
///
/// `string_list_param` does `.as_array()` followed by `unwrap_or_default()`, so
/// a bare JSON string yields an empty `Vec` while the key is still present: the
/// preset would be written with zero mounts and the call would report success.
/// Present-and-non-empty must never parse to empty, so anything that is not an
/// array of well-formed triples is a hard error here, and the parsed set then
/// goes through the same `check_mount_set` the config file path uses.
#[cfg(feature = "agent")]
fn parse_sandbox_mounts_param(
    params: &serde_json::Value,
) -> Result<Vec<moltis_config::schema::SandboxMountConfig>, ErrorShape> {
    let Some(items) = params
        .get("sandbox_mounts")
        .and_then(serde_json::Value::as_array)
    else {
        return Err(parse_preset_param_error(
            "sandbox_mounts must be an array of \"source:target:access\" strings".to_string(),
        ));
    };
    let mounts = items
        .iter()
        .map(|item| {
            let triple = item.as_str().ok_or_else(|| {
                parse_preset_param_error(
                    "sandbox_mounts entries must be \"source:target:access\" strings".to_string(),
                )
            })?;
            moltis_config::schema::SandboxMountConfig::parse_triple(triple.trim())
                .map_err(parse_preset_param_error)
        })
        .collect::<Result<Vec<_>, ErrorShape>>()?;
    moltis_config::schema::check_mount_set(&mounts)
        .map_err(|problems| parse_preset_param_error(problems.join("; ")))?;
    Ok(mounts)
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

#[cfg(all(test, feature = "agent"))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn preset_from_rpc_params_accepts_an_array_of_mount_triples() {
        let preset = preset_from_rpc_params(
            "walter",
            &serde_json::json!({
                "id": "walter",
                "sandbox_mounts": ["/srv/vault:/srv/vault:rw", "/srv/notes:/srv/notes:ro"],
            }),
            None,
        )
        .expect("a well-formed array must parse");

        let mounts = &preset.sandbox.mounts;
        assert_eq!(mounts.len(), 2);
        assert_eq!(mounts[0].source, "/srv/vault");
        assert_eq!(mounts[0].target, "/srv/vault");
        assert!(mounts[0].access.is_writable());
        assert!(!mounts[1].access.is_writable());
    }

    #[test]
    fn preset_from_rpc_params_rejects_a_bare_string_for_sandbox_mounts() {
        // `string_list_param` does `.as_array()` then `unwrap_or_default()`, so a
        // bare string would write a preset with zero mounts and report success.
        // Present-and-non-empty must never parse to empty.
        let error = preset_from_rpc_params(
            "walter",
            &serde_json::json!({
                "id": "walter",
                "sandbox_mounts": "/srv/vault:/srv/vault:rw",
            }),
            None,
        )
        .expect_err("a present-but-not-an-array value must be a hard error");
        assert!(
            error.message.contains("array"),
            "the error should say an array is expected, got: {}",
            error.message
        );
    }

    #[test]
    fn preset_from_rpc_params_rejects_a_malformed_mount_triple() {
        let error = preset_from_rpc_params(
            "walter",
            &serde_json::json!({
                "id": "walter",
                "sandbox_mounts": ["/srv/vault:/srv/vault"],
            }),
            None,
        )
        .expect_err("a two-field triple must be a hard error");
        assert!(
            error.message.contains("source:target:access"),
            "the error should name the expected shape, got: {}",
            error.message
        );
    }

    #[test]
    fn preset_from_rpc_params_rejects_a_relative_mount_source() {
        let error = preset_from_rpc_params(
            "walter",
            &serde_json::json!({
                "id": "walter",
                "sandbox_mounts": ["vault:/srv/vault:rw"],
            }),
            None,
        )
        .expect_err("a relative source must be a hard error");
        assert!(
            error.message.contains("absolute"),
            "the error should name the rule, got: {}",
            error.message
        );
    }

    #[test]
    fn preset_from_rpc_params_rejects_a_mount_source_that_resolves_to_root() {
        // `value == "/"` was the whole rule, and Docker resolves `/.` and `//`
        // to `/` before it sees them - so either spelling bound the entire host
        // filesystem into the container and passed validation.
        for source in ["/", "//", "/."] {
            let error = preset_from_rpc_params(
                "walter",
                &serde_json::json!({
                    "id": "walter",
                    "sandbox_mounts": [format!("{source}:/srv/root:ro")],
                }),
                None,
            )
            .expect_err("a source that resolves to / must be a hard error");
            assert!(
                error.message.contains("must not resolve to"),
                "source {source:?} must be refused, got: {}",
                error.message
            );
        }
    }

    #[test]
    fn preset_from_rpc_params_rejects_two_mounts_sharing_a_target() {
        let error = preset_from_rpc_params(
            "walter",
            &serde_json::json!({
                "id": "walter",
                "sandbox_mounts": ["/srv/a:/srv/shared:rw", "/srv/b:/srv/shared:ro"],
            }),
            None,
        )
        .expect_err("a duplicate target must be a hard error");
        assert!(
            error.message.contains("share the target"),
            "the error should name the rule, got: {}",
            error.message
        );
    }

    #[test]
    fn preset_from_rpc_params_mounts_read_back_through_preset_get_fields() {
        let preset = preset_from_rpc_params(
            "walter",
            &serde_json::json!({
                "id": "walter",
                "sandbox_mounts": ["/srv/vault:/srv/vault:rw"],
            }),
            None,
        )
        .expect("a well-formed array must parse");

        let fields = preset_sandbox_fields(&preset.sandbox);
        assert_eq!(
            fields["mounts"],
            serde_json::json!(["/srv/vault:/srv/vault:rw"]),
            "agents.preset.get must read the stored mounts back"
        );
    }

    #[test]
    fn preset_from_rpc_params_accepts_a_well_formed_run_as() {
        let preset = preset_from_rpc_params(
            "walter",
            &serde_json::json!({ "id": "walter", "run_as": "1000:1000" }),
            None,
        )
        .expect("a well-formed uid:gid must parse");

        assert_eq!(preset.sandbox.run_as.as_deref(), Some("1000:1000"));
        assert_eq!(
            preset_sandbox_fields(&preset.sandbox)["run_as"],
            serde_json::json!("1000:1000"),
            "agents.preset.get must read the stored run_as back"
        );
    }

    #[test]
    fn preset_from_rpc_params_rejects_a_malformed_run_as() {
        for value in ["1000", "1000:1000:1000", "1000:", ":1000", "walter:walter"] {
            let result = preset_from_rpc_params(
                "walter",
                &serde_json::json!({ "id": "walter", "run_as": value }),
                None,
            );
            let error = match result {
                Ok(preset) => panic!(
                    "run_as {value:?} must be rejected, parsed as {:?}",
                    preset.sandbox.run_as
                ),
                Err(error) => error,
            };
            assert!(
                error.message.contains("run_as"),
                "the error should name the field, got: {}",
                error.message
            );
        }
    }

    #[test]
    fn preset_from_rpc_params_rejects_a_root_run_as() {
        let error = preset_from_rpc_params(
            "walter",
            &serde_json::json!({ "id": "walter", "run_as": "0:0" }),
            None,
        )
        .expect_err("uid 0 must be a hard error");
        assert!(
            error.message.contains("uid 0"),
            "the error should name the rule, got: {}",
            error.message
        );
    }

    #[test]
    fn preset_from_rpc_params_rejects_a_root_gid() {
        // The uid was the only half refused, so `1000:0` wrote cleanly and put
        // the container in the root group - group-write on every root-owned
        // path the agent's mounts expose.
        let error = preset_from_rpc_params(
            "walter",
            &serde_json::json!({ "id": "walter", "run_as": "1000:0" }),
            None,
        )
        .expect_err("gid 0 must be a hard error");
        assert!(
            error.message.contains("gid 0"),
            "the error should name the rule, got: {}",
            error.message
        );
    }

    #[test]
    fn preset_from_rpc_params_rejects_a_non_string_run_as() {
        // `optional_string` would drop this silently, and a dropped run_as
        // means the container runs as root.
        let error = preset_from_rpc_params(
            "walter",
            &serde_json::json!({ "id": "walter", "run_as": 1000 }),
            None,
        )
        .expect_err("a non-string run_as must be a hard error");
        assert!(
            error.message.contains("uid:gid"),
            "the error should name the expected shape, got: {}",
            error.message
        );
    }

    #[test]
    fn preset_from_rpc_params_treats_a_null_run_as_as_a_clear() {
        // `""` cleared the field and `null` was a hard error, which is a shape
        // rule rather than a safety one: a client that models an optional
        // string as `null` was told its request was malformed with no way to
        // remove the setting.
        let preset = preset_from_rpc_params(
            "walter",
            &serde_json::json!({ "id": "walter", "run_as": serde_json::Value::Null }),
            None,
        )
        .expect("an explicit null must clear run_as, not fail");
        assert_eq!(preset.sandbox.run_as, None);
    }
}
