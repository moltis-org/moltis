use super::*;

pub(super) fn register(reg: &mut MethodRegistry) {
    // Skills
    reg.register(
        "skills.list",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .skills
                    .list()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "skills.status",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .skills
                    .status()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "skills.bins",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .skills
                    .bins()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "skills.install",
        Box::new(|ctx| {
            Box::pin(async move {
                let source = ctx
                    .params
                    .get("source")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let op_id = ctx
                    .params
                    .get("op_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or(ctx.request_id.as_str())
                    .to_string();

                broadcast(
                    &ctx.state,
                    "skills.install.progress",
                    serde_json::json!({
                        "phase": "start",
                        "source": source,
                        "op_id": op_id,
                    }),
                    BroadcastOpts::default(),
                )
                .await;

                match ctx.state.services.skills.install(ctx.params.clone()).await {
                    Ok(payload) => {
                        broadcast(
                            &ctx.state,
                            "skills.install.progress",
                            serde_json::json!({
                                "phase": "done",
                                "source": source,
                                "op_id": op_id,
                            }),
                            BroadcastOpts::default(),
                        )
                        .await;
                        Ok(payload)
                    },
                    Err(e) => {
                        broadcast(
                            &ctx.state,
                            "skills.install.progress",
                            serde_json::json!({
                                "phase": "error",
                                "source": source,
                                "op_id": op_id,
                                "error": e.to_string(),
                            }),
                            BroadcastOpts::default(),
                        )
                        .await;
                        Err(ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))
                    },
                }
            })
        }),
    );
    reg.register(
        "skills.remove",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .skills
                    .remove(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "skills.update",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .skills
                    .update(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "skills.repos.list",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .skills
                    .repos_list()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "skills.repos.remove",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .skills
                    .repos_remove(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "skills.repos.export",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .skills
                    .repos_export(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "skills.repos.import",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .skills
                    .repos_import(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "skills.repos.unquarantine",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .skills
                    .repos_unquarantine(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "skills.emergency_disable",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .skills
                    .emergency_disable()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "skills.skill.trust",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .skills
                    .skill_trust(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "skills.skill.enable",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .skills
                    .skill_enable(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "skills.skill.disable",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .skills
                    .skill_disable(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "skills.skill.detail",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .skills
                    .skill_detail(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "skills.install_dep",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .skills
                    .install_dep(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "skills.skill.save",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .skills
                    .skill_save(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "skills.bundled.categories",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .skills
                    .bundled_categories()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "skills.bundled.toggle_category",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .skills
                    .bundled_toggle_category(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    // MCP
    reg.register(
        "mcp.list",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .mcp
                    .list()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "mcp.add",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .mcp
                    .add(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "mcp.remove",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .mcp
                    .remove(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "mcp.enable",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .mcp
                    .enable(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "mcp.disable",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .mcp
                    .disable(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "mcp.status",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .mcp
                    .status(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "mcp.tools",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .mcp
                    .tools(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "mcp.restart",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .mcp
                    .restart(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "mcp.reauth",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .mcp
                    .reauth(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "mcp.oauth.start",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .mcp
                    .oauth_start(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "mcp.oauth.complete",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .mcp
                    .oauth_complete(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "mcp.update",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .mcp
                    .update(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "mcp.config.get",
        Box::new(|_ctx| {
            Box::pin(async move {
                let config = moltis_config::discover_and_load();
                Ok(serde_json::json!({
                    "request_timeout_secs": config.mcp.request_timeout_secs,
                }))
            })
        }),
    );
    reg.register(
        "mcp.config.update",
        Box::new(|ctx| {
            Box::pin(async move {
                let request_timeout_secs = match ctx.params.get("request_timeout_secs") {
                    None => {
                        return Err(ServiceError::message(
                            "missing 'request_timeout_secs' parameter",
                        )
                        .into());
                    },
                    Some(value) => value.as_u64().ok_or_else(|| {
                        ServiceError::message(
                            "invalid 'request_timeout_secs' parameter: expected a positive integer",
                        )
                    })?,
                };

                if request_timeout_secs == 0 {
                    return Err(ServiceError::message(
                        "request_timeout_secs must be greater than 0",
                    )
                    .into());
                }

                // Update in-memory first (infallible atomic store), then persist
                // to disk.  This ordering means a crash between the two steps
                // leaves the runtime correct and only the file stale — the next
                // restart reads the file anyway.
                ctx.state
                    .services
                    .mcp
                    .update_request_timeout(request_timeout_secs)
                    .await
                    .map_err(ErrorShape::from)?;

                if let Err(e) = moltis_config::update_config(|cfg| {
                    cfg.mcp.request_timeout_secs = request_timeout_secs;
                }) {
                    tracing::warn!(error = %e, "failed to persist MCP config");
                    return Err(ServiceError::message(format!(
                        "failed to persist MCP config: {e}"
                    ))
                    .into());
                }

                Ok(serde_json::json!({
                    "request_timeout_secs": request_timeout_secs,
                    "restart_required": true,
                }))
            })
        }),
    );

    // Browser
    reg.register(
        "browser.request",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .browser
                    .request(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    // Usage
    reg.register(
        "usage.status",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .usage
                    .status()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "usage.cost",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .usage
                    .cost(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    // Exec approvals
    reg.register(
        "exec.approvals.get",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .exec_approval
                    .get()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "exec.approvals.set",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .exec_approval
                    .set(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "exec.approvals.node.get",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .exec_approval
                    .node_get(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "exec.approvals.node.set",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .exec_approval
                    .node_set(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "exec.approval.request",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .exec_approval
                    .request(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "exec.approval.resolve",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .exec_approval
                    .resolve(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    // Network audit
    reg.register(
        "network.audit.list",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .network_audit
                    .list(ctx.params.clone())
                    .await
                    .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))
            })
        }),
    );
    reg.register(
        "network.audit.tail",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .network_audit
                    .tail(ctx.params.clone())
                    .await
                    .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))
            })
        }),
    );
    reg.register(
        "network.audit.stats",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .network_audit
                    .stats()
                    .await
                    .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))
            })
        }),
    );

    // Models
    reg.register(
        "models.list",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .model
                    .list()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "models.list_all",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .model
                    .list_all()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "models.disable",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .model
                    .disable(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "models.enable",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .model
                    .enable(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "models.detect_supported",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .model
                    .detect_supported(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "models.cancel_detect",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .model
                    .cancel_detect()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "models.test",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .model
                    .test(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    // Provider setup
    reg.register(
        "providers.available",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .provider_setup
                    .available()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "providers.save_key",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .provider_setup
                    .save_key(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "providers.validate_key",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .provider_setup
                    .validate_key(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "providers.oauth.start",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .provider_setup
                    .oauth_start(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "providers.oauth.status",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .provider_setup
                    .oauth_status(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "providers.oauth.complete",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .provider_setup
                    .oauth_complete(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "providers.save_model",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .provider_setup
                    .save_model(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "providers.save_models",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .provider_setup
                    .save_models(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "providers.remove_key",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .provider_setup
                    .remove_key(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    reg.register(
        "providers.add_custom",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .provider_setup
                    .add_custom(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    // Local LLM
    reg.register(
        "providers.local.system_info",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .local_llm
                    .system_info()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "providers.local.models",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .local_llm
                    .models()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "providers.local.configure",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .local_llm
                    .configure(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "providers.local.status",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .local_llm
                    .status()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "providers.local.search_hf",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .local_llm
                    .search_hf(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "providers.local.configure_custom",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .local_llm
                    .configure_custom(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "providers.local.remove_model",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .local_llm
                    .remove_model(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    // Voicewake
    reg.register(
        "voicewake.get",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .voicewake
                    .get()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "voicewake.set",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .voicewake
                    .set(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "wake",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .voicewake
                    .wake(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "talk.mode",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .voicewake
                    .talk_mode(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
}
