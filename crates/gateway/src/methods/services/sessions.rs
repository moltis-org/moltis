use super::*;

pub(super) fn register(reg: &mut MethodRegistry) {
    // Sessions
    reg.register(
        "sessions.list",
        Box::new(|ctx| {
            Box::pin(async move {
                let mut result = ctx
                    .state
                    .services
                    .session
                    .list()
                    .await
                    .map_err(ErrorShape::from)?;

                // Inject replying state so the frontend can restore the
                // thinking indicator after a full page reload.
                let active_keys = ctx.state.chat().await.active_session_keys().await;
                if let Some(arr) = result.as_array_mut() {
                    for entry in arr {
                        let key_str = entry.get("key").and_then(|v| v.as_str()).map(String::from);
                        if let (Some(key), Some(obj)) = (key_str, entry.as_object_mut()) {
                            obj.insert(
                                "replying".to_string(),
                                serde_json::Value::Bool(active_keys.iter().any(|k| k == &key)),
                            );
                        }
                    }
                }
                Ok(result)
            })
        }),
    );
    reg.register(
        "sessions.preview",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .session
                    .preview(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "sessions.search",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .session
                    .search(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "sessions.resolve",
        Box::new(|ctx| {
            Box::pin(async move {
                let result = ctx
                    .state
                    .services
                    .session
                    .resolve(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)?;

                // Newly created sessions have an empty history array.
                let is_new = result
                    .get("history")
                    .and_then(|h| h.as_array())
                    .is_some_and(|a| a.is_empty());
                if is_new
                    && let Some(key) = result
                        .get("entry")
                        .and_then(|e| e.get("key"))
                        .and_then(|k| k.as_str())
                {
                    broadcast(
                        &ctx.state,
                        "session",
                        serde_json::json!({
                            "kind": "created",
                            "sessionKey": key,
                        }),
                        BroadcastOpts {
                            drop_if_slow: true,
                            ..Default::default()
                        },
                    )
                    .await;
                }
                Ok(result)
            })
        }),
    );
    reg.register(
        "sessions.patch",
        Box::new(|ctx| {
            Box::pin(async move {
                let key = ctx
                    .params
                    .get("key")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let sandbox_toggled = ctx.params.get("sandboxEnabled").is_some();
                let result = ctx
                    .state
                    .services
                    .session
                    .patch(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)?;
                let version = result.get("version").and_then(|v| v.as_u64()).unwrap_or(0);
                broadcast(
                    &ctx.state,
                    "session",
                    serde_json::json!({
                        "kind": "patched",
                        "sessionKey": key,
                        "version": version,
                    }),
                    BroadcastOpts::default(),
                )
                .await;
                if sandbox_toggled {
                    let enabled = result
                        .get("sandbox_enabled")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let message = if enabled {
                        "Sandbox enabled — commands now run in container."
                    } else {
                        "Sandbox disabled — commands now run on host."
                    };
                    broadcast(
                        &ctx.state,
                        "chat",
                        serde_json::json!({
                            "sessionKey": key,
                            "state": "notice",
                            "title": "Sandbox",
                            "message": message,
                        }),
                        BroadcastOpts::default(),
                    )
                    .await;
                }
                Ok(result)
            })
        }),
    );
    reg.register(
        "sessions.voice.generate",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .session
                    .voice_generate(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "sessions.reset",
        Box::new(|ctx| {
            Box::pin(async move {
                // Run session-end memory summary before clearing, if enabled.
                let key = ctx.params.get("key").and_then(|v| v.as_str()).unwrap_or("");
                if !key.is_empty() {
                    crate::session::summary::run_session_summary_if_enabled(&ctx.state, key).await;
                }

                ctx.state
                    .services
                    .session
                    .reset(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "sessions.delete",
        Box::new(|ctx| {
            Box::pin(async move {
                let key = ctx
                    .params
                    .get("key")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let result = ctx
                    .state
                    .services
                    .session
                    .delete(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)?;
                if !key.is_empty() {
                    broadcast(
                        &ctx.state,
                        "session",
                        serde_json::json!({
                            "kind": "deleted",
                            "sessionKey": key,
                        }),
                        BroadcastOpts {
                            drop_if_slow: true,
                            ..Default::default()
                        },
                    )
                    .await;
                }
                Ok(result)
            })
        }),
    );
    reg.register(
        "sessions.clear_all",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .session
                    .clear_all()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "sessions.compact",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .session
                    .compact(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    reg.register(
        "sessions.fork",
        Box::new(|ctx| {
            Box::pin(async move {
                let result = ctx
                    .state
                    .services
                    .session
                    .fork(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)?;
                if let Some(key) = result.get("key").and_then(|k| k.as_str()) {
                    broadcast(
                        &ctx.state,
                        "session",
                        serde_json::json!({
                            "kind": "created",
                            "sessionKey": key,
                        }),
                        BroadcastOpts {
                            drop_if_slow: true,
                            ..Default::default()
                        },
                    )
                    .await;
                }
                Ok(result)
            })
        }),
    );
    reg.register(
        "sessions.branches",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .session
                    .branches(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "sessions.run_detail",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .session
                    .run_detail(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "sessions.share.create",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .session
                    .share_create(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "sessions.share.list",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .session
                    .share_list(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "sessions.share.revoke",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .session
                    .share_revoke(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
}
