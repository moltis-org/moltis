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
                if agent_id == "main" {
                    return ctx
                        .state
                        .services
                        .onboarding
                        .identity_update(ctx.params)
                        .await
                        .map_err(ErrorShape::from);
                }
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
                if agent_id == "main" {
                    return ctx
                        .state
                        .services
                        .onboarding
                        .identity_update_soul(soul)
                        .await
                        .map_err(ErrorShape::from);
                }
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
                    if agent_id == "main" {
                        return ctx
                            .state
                            .services
                            .onboarding
                            .identity_update(ctx.params)
                            .await
                            .map_err(ErrorShape::from);
                    }
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
                    // Sync identity into preset.
                    if let Some(ref agents_config) = ctx.state.services.agents_config {
                        let mut guard = agents_config.write().await;
                        if let Some(entry) = guard.presets.get_mut(&agent_id) {
                            entry.identity = identity;
                        }
                    }
                    Ok(serde_json::json!({ "ok": true }))
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
                    if agent_id != "main"
                        && let Some(ref agents_config) = ctx.state.services.agents_config
                    {
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
                    let config = moltis_config::discover_and_load();
                    let toml_str = match config.agents.presets.get(&id) {
                        Some(preset) => toml::to_string_pretty(preset).unwrap_or_default(),
                        None => String::new(),
                    };
                    Ok(serde_json::json!({
                        "id": id,
                        "toml": toml_str,
                        "exists": !toml_str.is_empty(),
                    }))
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
                    let config = moltis_config::discover_and_load();
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

                    let config_only: Vec<serde_json::Value> = config
                        .agents
                        .presets
                        .iter()
                        .filter(|(name, _)| !persona_ids.contains(*name))
                        .map(|(name, preset)| {
                            let toml_str = toml::to_string_pretty(preset).unwrap_or_default();
                            serde_json::json!({
                                "id": name,
                                "name": preset.identity.name.as_deref().unwrap_or(name),
                                "emoji": preset.identity.emoji,
                                "theme": preset.identity.theme,
                                "model": preset.model,
                                "toml": toml_str,
                                "source": "config",
                            })
                        })
                        .collect();

                    Ok(serde_json::json!({ "presets": config_only }))
                })
            }),
        );
    }
}
