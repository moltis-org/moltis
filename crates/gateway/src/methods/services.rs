use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use tracing::warn;

use {
    moltis_config::VoiceSttProvider,
    moltis_protocol::{ErrorShape, error_codes},
};

use crate::broadcast::{BroadcastOpts, broadcast};

use super::{MethodContext, MethodRegistry};

pub(super) fn model_probe_params(provider: Option<&str>) -> serde_json::Value {
    let mut params = serde_json::json!({
        "background": true,
        "reason": "provider_connected",
    });
    if let Some(provider) = provider
        && !provider.trim().is_empty()
    {
        params["provider"] = serde_json::json!(provider);
    }
    params
}

async fn active_session_key_for_ctx(ctx: &MethodContext) -> Option<String> {
    if let Some(session_key) = ctx
        .params
        .get("_session_key")
        .and_then(|v| v.as_str())
        .filter(|value| !value.trim().is_empty())
    {
        return Some(session_key.to_string());
    }
    let inner = ctx.state.inner.read().await;
    inner.active_sessions.get(&ctx.client_conn_id).cloned()
}

async fn default_agent_id_for_ctx(ctx: &MethodContext) -> String {
    if let Some(ref store) = ctx.state.services.agent_persona_store {
        return store
            .default_id()
            .await
            .unwrap_or_else(|_| "main".to_string());
    }
    "main".to_string()
}

async fn agent_exists_for_ctx(ctx: &MethodContext, agent_id: &str) -> bool {
    if agent_id == "main" {
        return true;
    }
    if let Some(ref store) = ctx.state.services.agent_persona_store {
        return store.get(agent_id).await.ok().flatten().is_some();
    }
    false
}

async fn resolve_session_agent_id_for_ctx(ctx: &MethodContext) -> String {
    let default_id = default_agent_id_for_ctx(ctx).await;
    let Some(session_key) = active_session_key_for_ctx(ctx).await else {
        return default_id;
    };
    let Some(ref metadata) = ctx.state.services.session_metadata else {
        return default_id;
    };
    let Some(entry) = metadata.get(&session_key).await else {
        return default_id;
    };
    let Some(agent_id) = entry
        .agent_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return default_id;
    };
    if agent_exists_for_ctx(ctx, agent_id).await {
        return agent_id.to_string();
    }
    warn!(
        session = %session_key,
        agent_id,
        fallback = %default_id,
        "session references unknown agent, falling back to default"
    );
    let _ = metadata.set_agent_id(&session_key, Some(&default_id)).await;
    default_id
}

fn parse_agent_id_param(params: &serde_json::Value) -> Option<String> {
    params
        .get("agent_id")
        .or_else(|| params.get("id"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

async fn resolve_requested_agent_id(
    ctx: &MethodContext,
    params: &serde_json::Value,
) -> Result<String, ErrorShape> {
    if let Some(id) = parse_agent_id_param(params) {
        if agent_exists_for_ctx(ctx, &id).await {
            return Ok(id);
        }
        return Err(ErrorShape::new(
            error_codes::INVALID_REQUEST,
            format!("agent '{id}' not found"),
        ));
    }
    Ok(default_agent_id_for_ctx(ctx).await)
}

fn read_identity_payload_for_agent(agent_id: &str) -> serde_json::Value {
    let config = moltis_config::discover_and_load();
    let mut identity = config.identity.clone();
    if let Some(file_identity) = moltis_config::load_identity_for_agent(agent_id) {
        if file_identity.name.is_some() {
            identity.name = file_identity.name;
        }
        if file_identity.emoji.is_some() {
            identity.emoji = file_identity.emoji;
        }
        if file_identity.theme.is_some() {
            identity.theme = file_identity.theme;
        }
    }
    let mut user = config.user;
    if let Some(file_user) = moltis_config::load_user() {
        if file_user.name.is_some() {
            user.name = file_user.name;
        }
        if file_user.timezone.is_some() {
            user.timezone = file_user.timezone;
        }
    }
    let resolved_name = identity
        .name
        .clone()
        .unwrap_or_else(|| "moltis".to_string());
    let identity_path = if agent_id == "main" {
        let main_path = moltis_config::agent_workspace_dir("main").join("IDENTITY.md");
        if main_path.exists() {
            main_path
        } else {
            moltis_config::identity_path()
        }
    } else {
        moltis_config::agent_workspace_dir(agent_id).join("IDENTITY.md")
    };
    let identity_text = std::fs::read_to_string(identity_path)
        .ok()
        .and_then(|content| moltis_config::extract_yaml_frontmatter(&content).map(str::to_string));
    let soul = moltis_config::load_soul_for_agent(agent_id);
    let identity_name = identity.name.clone();
    let identity_emoji = identity.emoji.clone();
    let identity_theme = identity.theme.clone();
    let user_name = user.name.clone();
    let user_timezone = user.timezone.as_ref().map(|tz| tz.name().to_string());
    serde_json::json!({
        "name": resolved_name,
        "emoji": identity_emoji.clone(),
        "theme": identity_theme.clone(),
        "user_name": user_name,
        "user_timezone": user_timezone,
        "identity": identity_text,
        "identity_fields": {
            "name": identity_name,
            "emoji": identity_emoji,
            "theme": identity_theme,
        },
        "soul": soul,
    })
}

fn write_soul_for_agent(agent_id: &str, soul: Option<String>) -> Result<(), ErrorShape> {
    if agent_id == "main" {
        moltis_config::save_soul(soul.as_deref())
            .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))?;
        return Ok(());
    }
    let dir = moltis_config::agent_workspace_dir(agent_id);
    std::fs::create_dir_all(&dir)
        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))?;
    let soul_path = dir.join("SOUL.md");
    match soul.as_deref().map(str::trim) {
        Some(content) if !content.is_empty() => {
            std::fs::write(&soul_path, content)
                .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))?;
        },
        _ => {
            std::fs::write(&soul_path, "")
                .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))?;
        },
    }
    Ok(())
}

fn normalize_relative_agent_path(path: &str) -> Result<PathBuf, ErrorShape> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(ErrorShape::new(
            error_codes::INVALID_REQUEST,
            "missing 'path' parameter",
        ));
    }
    let candidate = Path::new(trimmed);
    if candidate.is_absolute() {
        return Err(ErrorShape::new(
            error_codes::INVALID_REQUEST,
            "path must be relative",
        ));
    }
    for component in candidate.components() {
        if matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            return Err(ErrorShape::new(
                error_codes::INVALID_REQUEST,
                "path traversal is not allowed",
            ));
        }
    }
    Ok(candidate.to_path_buf())
}

fn read_agent_file(agent_id: &str, relative_path: &Path) -> Result<String, ErrorShape> {
    let primary = moltis_config::agent_workspace_dir(agent_id).join(relative_path);
    let fallback = (agent_id == "main").then(|| moltis_config::data_dir().join(relative_path));

    let target = if primary.exists() {
        Some(primary)
    } else {
        fallback.filter(|path| path.exists())
    }
    .ok_or_else(|| ErrorShape::new(error_codes::INVALID_REQUEST, "file not found"))?;

    std::fs::read_to_string(target)
        .map_err(|e| ErrorShape::new(error_codes::UNAVAILABLE, e.to_string()))
}

fn list_agent_workspace_files_recursively(
    root: &Path,
    base: &Path,
    files: &mut Vec<serde_json::Value>,
) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            list_agent_workspace_files_recursively(&path, base, files);
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        if let Ok(relative) = path.strip_prefix(base) {
            files.push(serde_json::json!({
                "path": relative.to_string_lossy(),
                "size": entry.metadata().ok().map(|m| m.len()),
            }));
        }
    }
}

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
                    let agents = store.list().await.map_err(ErrorShape::from)?;
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
                    write_soul_for_agent(&agent_id, soul)?;
                    Ok(serde_json::json!({ "ok": true }))
                })
            }),
        );
        reg.register(
            "agents.files.list",
            Box::new(|ctx| {
                Box::pin(async move {
                    let agent_id = resolve_requested_agent_id(&ctx, &ctx.params).await?;
                    let mut files: Vec<serde_json::Value> = Vec::new();
                    let root = moltis_config::agent_workspace_dir(&agent_id);
                    let root_exists = root.exists();
                    if root_exists {
                        list_agent_workspace_files_recursively(&root, &root, &mut files);
                    }
                    if agent_id == "main" {
                        for file_name in &[
                            "IDENTITY.md",
                            "SOUL.md",
                            "MEMORY.md",
                            "AGENTS.md",
                            "TOOLS.md",
                        ] {
                            let agent_path = root.join(file_name);
                            let root_path = moltis_config::data_dir().join(file_name);
                            if !agent_path.exists() && root_path.exists() {
                                files.push(serde_json::json!({
                                    "path": file_name,
                                    "source": "root",
                                    "size": std::fs::metadata(root_path).ok().map(|m| m.len()),
                                }));
                            }
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
    }

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

    // Channels
    reg.register(
        "channels.status",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .channel
                    .status()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    // channels.list is an alias for channels.status (used by the UI)
    reg.register(
        "channels.list",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .channel
                    .status()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "channels.add",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .channel
                    .add(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "channels.remove",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .channel
                    .remove(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "channels.update",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .channel
                    .update(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "channels.logout",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .channel
                    .logout(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "channels.senders.list",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .channel
                    .senders_list(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "channels.senders.approve",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .channel
                    .sender_approve(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "channels.senders.deny",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .channel
                    .sender_deny(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "send",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .channel
                    .send(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

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
                    if let Some(hb_job) = jobs.iter().find(|j| j.name == "__heartbeat__") {
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
                        let job_patch = moltis_cron::types::CronJobPatch {
                            schedule: Some(moltis_cron::types::CronSchedule::Every {
                                every_ms: interval_ms,
                                anchor_ms: None,
                            }),
                            payload: Some(moltis_cron::types::CronPayload::AgentTurn {
                                message: prompt,
                                model: patch.model.clone(),
                                timeout_secs: None,
                                deliver: false,
                                channel: None,
                                to: None,
                            }),
                            enabled: Some(effective_enabled),
                            sandbox: Some(moltis_cron::types::CronSandboxConfig {
                                enabled: patch.sandbox_enabled,
                                image: patch.sandbox_image.clone(),
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
                let mut resolve_params = serde_json::json!({ "key": key });
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

                // Inject replying state so frontend restores thinking
                // indicator and voice-pending state after page reload.
                let chat = ctx.state.chat().await;
                let active_keys = chat.active_session_keys().await;
                let replying = active_keys.iter().any(|k| k == key);
                let mut result = result;
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
                                    Duration::from_secs(3),
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
                let provider_name = ctx
                    .params
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned);

                let result = ctx
                    .state
                    .services
                    .provider_setup
                    .save_key(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)?;

                // Kick off background model detection after saving provider
                // credentials, matching the behaviour of oauth.complete.
                let model_service = Arc::clone(&ctx.state.services.model);
                tokio::spawn(async move {
                    let _ = model_service
                        .detect_supported(model_probe_params(provider_name.as_deref()))
                        .await;
                });

                Ok(result)
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
                let provider_name = ctx
                    .params
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned);
                let result = ctx
                    .state
                    .services
                    .provider_setup
                    .oauth_start(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)?;

                // If oauth.start short-circuited because valid tokens already
                // existed, trigger a provider-scoped background probe now.
                if result
                    .get("alreadyAuthenticated")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    let model_service = Arc::clone(&ctx.state.services.model);
                    tokio::spawn(async move {
                        let _ = model_service
                            .detect_supported(model_probe_params(provider_name.as_deref()))
                            .await;
                    });
                }

                Ok(result)
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
                let result = ctx
                    .state
                    .services
                    .provider_setup
                    .oauth_complete(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)?;

                let provider_name = result
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned);

                // Kick off background support probing after OAuth provider connect.
                let model_service = Arc::clone(&ctx.state.services.model);
                tokio::spawn(async move {
                    let _ = model_service
                        .detect_supported(model_probe_params(provider_name.as_deref()))
                        .await;
                });

                Ok(result)
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
                let provider_name = ctx
                    .params
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned);

                let result = ctx
                    .state
                    .services
                    .provider_setup
                    .save_models(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)?;

                // Kick off background support probing after saving preferred models.
                let model_service = Arc::clone(&ctx.state.services.model);
                tokio::spawn(async move {
                    let _ = model_service
                        .detect_supported(model_probe_params(provider_name.as_deref()))
                        .await;
                });

                Ok(result)
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

    // Update
    reg.register(
        "update.run",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .update
                    .run(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    // Onboarding / Wizard
    reg.register(
        "wizard.start",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .onboarding
                    .wizard_start(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "wizard.next",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .onboarding
                    .wizard_next(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "wizard.cancel",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .onboarding
                    .wizard_cancel()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "wizard.status",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .onboarding
                    .wizard_status()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    // Web login
    reg.register(
        "web.login.start",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .web_login
                    .start(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "web.login.wait",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .web_login
                    .wait(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    // ── Projects ────────────────────────────────────────────────────

    reg.register(
        "projects.list",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .project
                    .list()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "projects.get",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .project
                    .get(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "projects.upsert",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .project
                    .upsert(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "projects.delete",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .project
                    .delete(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "projects.detect",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .project
                    .detect(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "projects.complete_path",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .project
                    .complete_path(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "projects.context",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .project
                    .context(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    // ── Voice Config ───────────────────────────────────────────────
    #[cfg(feature = "voice")]
    {
        reg.register(
                "voice.config.get",
                Box::new(|_ctx| {
                    Box::pin(async move {
                        let config = moltis_config::discover_and_load();
                        Ok(serde_json::json!({
                            "tts": {
                                "enabled": config.voice.tts.enabled,
                                "provider": config.voice.tts.provider,
                                "elevenlabs_configured": config.voice.tts.elevenlabs.api_key.is_some(),
                                "openai_configured": config.voice.tts.openai.api_key.is_some(),
                            },
                            "stt": {
                                "enabled": config.voice.stt.enabled,
                                "provider": config.voice.stt.provider,
                                "whisper_configured": config.voice.stt.whisper.api_key.is_some(),
                                "groq_configured": config.voice.stt.groq.api_key.is_some(),
                                "deepgram_configured": config.voice.stt.deepgram.api_key.is_some(),
                                "google_configured": config.voice.stt.google.api_key.is_some(),
                                "elevenlabs_configured": config.voice.stt.elevenlabs.api_key.is_some(),
                                "whisper_cli_configured": config.voice.stt.whisper_cli.model_path.is_some(),
                                "sherpa_onnx_configured": config.voice.stt.sherpa_onnx.model_dir.is_some(),
                            },
                        }))
                    })
                }),
            );
        // Comprehensive provider listing with availability detection
        reg.register(
            "voice.providers.all",
            Box::new(|_ctx| {
                Box::pin(async move {
                    let config = moltis_config::discover_and_load();
                    let providers = super::voice::detect_voice_providers(&config).await;
                    Ok(serde_json::json!(providers))
                })
            }),
        );
        reg.register(
            "voice.elevenlabs.catalog",
            Box::new(|_ctx| {
                Box::pin(async move {
                    let config = moltis_config::discover_and_load();
                    Ok(super::voice::fetch_elevenlabs_catalog(&config).await)
                })
            }),
        );
        // Enable/disable a voice provider (updates config file)
        reg.register(
            "voice.provider.toggle",
            Box::new(|ctx| {
                Box::pin(async move {
                    let provider = ctx
                        .params
                        .get("provider")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing provider")
                        })?;
                    let enabled = ctx
                        .params
                        .get("enabled")
                        .and_then(|v| v.as_bool())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing enabled")
                        })?;
                    let provider_type = ctx
                        .params
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("stt");

                    super::voice::toggle_voice_provider(provider, enabled, provider_type).map_err(
                        |e| {
                            ErrorShape::new(
                                error_codes::UNAVAILABLE,
                                format!("failed to toggle provider: {}", e),
                            )
                        },
                    )?;

                    // Broadcast change
                    broadcast(
                        &ctx.state,
                        "voice.config.changed",
                        serde_json::json!({ "provider": provider, "enabled": enabled }),
                        BroadcastOpts::default(),
                    )
                    .await;

                    Ok(serde_json::json!({ "ok": true, "provider": provider, "enabled": enabled }))
                })
            }),
        );
        reg.register(
            "voice.override.session.set",
            Box::new(|ctx| {
                Box::pin(async move {
                    let session_key = ctx
                        .params
                        .get("sessionKey")
                        .or_else(|| ctx.params.get("session_key"))
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing sessionKey")
                        })?
                        .to_string();

                    let override_cfg = crate::state::TtsRuntimeOverride {
                        provider: ctx
                            .params
                            .get("provider")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        voice_id: ctx
                            .params
                            .get("voiceId")
                            .or_else(|| ctx.params.get("voice_id"))
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        model: ctx
                            .params
                            .get("model")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                    };

                    ctx.state
                        .inner
                        .write()
                        .await
                        .tts_session_overrides
                        .insert(session_key.clone(), override_cfg.clone());

                    Ok(serde_json::to_value(override_cfg).unwrap_or_else(
                        |_| serde_json::json!({ "ok": true, "sessionKey": session_key }),
                    ))
                })
            }),
        );
        reg.register(
            "voice.override.session.clear",
            Box::new(|ctx| {
                Box::pin(async move {
                    let session_key = ctx
                        .params
                        .get("sessionKey")
                        .or_else(|| ctx.params.get("session_key"))
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing sessionKey")
                        })?
                        .to_string();

                    ctx.state
                        .inner
                        .write()
                        .await
                        .tts_session_overrides
                        .remove(&session_key);
                    Ok(serde_json::json!({ "ok": true, "sessionKey": session_key }))
                })
            }),
        );
        reg.register(
            "voice.override.channel.set",
            Box::new(|ctx| {
                Box::pin(async move {
                    let channel_type = ctx
                        .params
                        .get("channelType")
                        .or_else(|| ctx.params.get("channel_type"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("telegram");
                    let account_id = ctx
                        .params
                        .get("accountId")
                        .or_else(|| ctx.params.get("account_id"))
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing accountId")
                        })?;

                    let key = format!("{}:{}", channel_type, account_id);
                    let override_cfg = crate::state::TtsRuntimeOverride {
                        provider: ctx
                            .params
                            .get("provider")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        voice_id: ctx
                            .params
                            .get("voiceId")
                            .or_else(|| ctx.params.get("voice_id"))
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        model: ctx
                            .params
                            .get("model")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                    };

                    ctx.state
                        .inner
                        .write()
                        .await
                        .tts_channel_overrides
                        .insert(key.clone(), override_cfg.clone());

                    Ok(serde_json::json!({ "ok": true, "key": key, "override": override_cfg }))
                })
            }),
        );
        reg.register(
            "voice.override.channel.clear",
            Box::new(|ctx| {
                Box::pin(async move {
                    let channel_type = ctx
                        .params
                        .get("channelType")
                        .or_else(|| ctx.params.get("channel_type"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("telegram");
                    let account_id = ctx
                        .params
                        .get("accountId")
                        .or_else(|| ctx.params.get("account_id"))
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing accountId")
                        })?;

                    let key = format!("{}:{}", channel_type, account_id);
                    ctx.state
                        .inner
                        .write()
                        .await
                        .tts_channel_overrides
                        .remove(&key);
                    Ok(serde_json::json!({ "ok": true, "key": key }))
                })
            }),
        );
        reg.register(
            "voice.config.save_key",
            Box::new(|ctx| {
                Box::pin(async move {
                    use secrecy::Secret;

                    let provider = ctx
                        .params
                        .get("provider")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing provider")
                        })?;
                    let api_key = ctx
                        .params
                        .get("api_key")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing api_key")
                        })?;

                    moltis_config::update_config(|cfg| {
                        match provider {
                            // TTS providers
                            "elevenlabs" => {
                                // ElevenLabs shares key between TTS and STT
                                let key = Secret::new(api_key.to_string());
                                cfg.voice.tts.elevenlabs.api_key = Some(key.clone());
                                cfg.voice.stt.elevenlabs.api_key =
                                    Some(Secret::new(api_key.to_string()));
                                // Auto-enable both TTS and STT with ElevenLabs
                                cfg.voice.tts.provider = "elevenlabs".to_string();
                                cfg.voice.tts.enabled = true;
                                cfg.voice.stt.provider = Some(VoiceSttProvider::ElevenLabs);
                                cfg.voice.stt.enabled = true;
                            },
                            "openai" | "openai-tts" => {
                                cfg.voice.tts.openai.api_key =
                                    Some(Secret::new(api_key.to_string()));
                                cfg.voice.tts.provider = "openai".to_string();
                                cfg.voice.tts.enabled = true;
                            },
                            "google-tts" => {
                                // Google API key is shared - set both TTS and STT
                                let key = Secret::new(api_key.to_string());
                                cfg.voice.tts.google.api_key = Some(key.clone());
                                cfg.voice.stt.google.api_key =
                                    Some(Secret::new(api_key.to_string()));
                                // Auto-enable both TTS and STT with Google
                                cfg.voice.tts.provider = "google".to_string();
                                cfg.voice.tts.enabled = true;
                                cfg.voice.stt.provider = Some(VoiceSttProvider::Google);
                                cfg.voice.stt.enabled = true;
                            },
                            // STT providers
                            "whisper" => {
                                cfg.voice.stt.whisper.api_key =
                                    Some(Secret::new(api_key.to_string()));
                                cfg.voice.stt.provider = Some(VoiceSttProvider::Whisper);
                                cfg.voice.stt.enabled = true;
                            },
                            "groq" => {
                                cfg.voice.stt.groq.api_key = Some(Secret::new(api_key.to_string()));
                                cfg.voice.stt.provider = Some(VoiceSttProvider::Groq);
                                cfg.voice.stt.enabled = true;
                            },
                            "deepgram" => {
                                cfg.voice.stt.deepgram.api_key =
                                    Some(Secret::new(api_key.to_string()));
                                cfg.voice.stt.provider = Some(VoiceSttProvider::Deepgram);
                                cfg.voice.stt.enabled = true;
                            },
                            "google" => {
                                // Google STT key - also set TTS since they share the same key
                                let key = Secret::new(api_key.to_string());
                                cfg.voice.stt.google.api_key = Some(key.clone());
                                cfg.voice.tts.google.api_key =
                                    Some(Secret::new(api_key.to_string()));
                                // Auto-enable both STT and TTS with Google
                                cfg.voice.stt.provider = Some(VoiceSttProvider::Google);
                                cfg.voice.stt.enabled = true;
                                cfg.voice.tts.provider = "google".to_string();
                                cfg.voice.tts.enabled = true;
                            },
                            "mistral" => {
                                cfg.voice.stt.mistral.api_key =
                                    Some(Secret::new(api_key.to_string()));
                                cfg.voice.stt.provider = Some(VoiceSttProvider::Mistral);
                                cfg.voice.stt.enabled = true;
                            },
                            "elevenlabs-stt" => {
                                // ElevenLabs shares key between TTS and STT
                                let key = Secret::new(api_key.to_string());
                                cfg.voice.stt.elevenlabs.api_key = Some(key.clone());
                                cfg.voice.tts.elevenlabs.api_key =
                                    Some(Secret::new(api_key.to_string()));
                                // Auto-enable both STT and TTS with ElevenLabs
                                cfg.voice.stt.provider = Some(VoiceSttProvider::ElevenLabs);
                                cfg.voice.stt.enabled = true;
                                cfg.voice.tts.provider = "elevenlabs".to_string();
                                cfg.voice.tts.enabled = true;
                            },
                            _ => {},
                        }

                        super::voice::apply_voice_provider_settings(cfg, provider, &ctx.params);
                    })
                    .map_err(|e| {
                        ErrorShape::new(error_codes::UNAVAILABLE, format!("failed to save: {}", e))
                    })?;

                    // Broadcast voice config change event
                    broadcast(
                        &ctx.state,
                        "voice.config.changed",
                        serde_json::json!({ "provider": provider }),
                        BroadcastOpts::default(),
                    )
                    .await;

                    Ok(serde_json::json!({ "ok": true, "provider": provider }))
                })
            }),
        );
        reg.register(
            "voice.config.save_settings",
            Box::new(|ctx| {
                Box::pin(async move {
                    let provider = ctx
                        .params
                        .get("provider")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing provider")
                        })?;

                    moltis_config::update_config(|cfg| {
                        super::voice::apply_voice_provider_settings(cfg, provider, &ctx.params);
                    })
                    .map_err(|e| {
                        ErrorShape::new(
                            error_codes::UNAVAILABLE,
                            format!("failed to save settings: {}", e),
                        )
                    })?;

                    broadcast(
                        &ctx.state,
                        "voice.config.changed",
                        serde_json::json!({ "provider": provider, "settings": true }),
                        BroadcastOpts::default(),
                    )
                    .await;

                    Ok(serde_json::json!({ "ok": true, "provider": provider }))
                })
            }),
        );
        reg.register(
            "voice.config.remove_key",
            Box::new(|ctx| {
                Box::pin(async move {
                    let provider = ctx
                        .params
                        .get("provider")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing provider")
                        })?;

                    moltis_config::update_config(|cfg| match provider {
                        // TTS providers
                        "elevenlabs" => {
                            cfg.voice.tts.elevenlabs.api_key = None;
                        },
                        "openai" => {
                            cfg.voice.tts.openai.api_key = None;
                        },
                        // STT providers
                        "whisper" => {
                            cfg.voice.stt.whisper.api_key = None;
                        },
                        "groq" => {
                            cfg.voice.stt.groq.api_key = None;
                        },
                        "deepgram" => {
                            cfg.voice.stt.deepgram.api_key = None;
                        },
                        "google" => {
                            cfg.voice.stt.google.api_key = None;
                        },
                        "mistral" => {
                            cfg.voice.stt.mistral.api_key = None;
                        },
                        "elevenlabs-stt" => {
                            cfg.voice.stt.elevenlabs.api_key = None;
                        },
                        _ => {},
                    })
                    .map_err(|e| {
                        ErrorShape::new(error_codes::UNAVAILABLE, format!("failed to save: {}", e))
                    })?;

                    // Broadcast voice config change event
                    broadcast(
                        &ctx.state,
                        "voice.config.changed",
                        serde_json::json!({ "provider": provider, "removed": true }),
                        BroadcastOpts::default(),
                    )
                    .await;

                    Ok(serde_json::json!({ "ok": true, "provider": provider }))
                })
            }),
        );
        reg.register(
            "voice.config.voxtral_requirements",
            Box::new(|_ctx| {
                Box::pin(async move {
                    // Detect OS and architecture
                    let os = std::env::consts::OS;
                    let arch = std::env::consts::ARCH;

                    // Check Python version
                    let python_info = super::voice::check_python_version().await;

                    // Check CUDA availability
                    let cuda_info = super::voice::check_cuda_availability().await;

                    // Determine compatibility
                    let (compatible, reasons) = super::voice::check_voxtral_compatibility(
                        os,
                        arch,
                        &python_info,
                        &cuda_info,
                    );

                    Ok(serde_json::json!({
                        "os": os,
                        "arch": arch,
                        "python": python_info,
                        "cuda": cuda_info,
                        "compatible": compatible,
                        "reasons": reasons,
                    }))
                })
            }),
        );
    }

    #[cfg(feature = "graphql")]
    {
        reg.register(
            "graphql.config.get",
            Box::new(|ctx| {
                Box::pin(async move {
                    Ok(serde_json::json!({
                        "enabled": ctx.state.is_graphql_enabled(),
                    }))
                })
            }),
        );
        reg.register(
            "graphql.config.set",
            Box::new(|ctx| {
                Box::pin(async move {
                    let enabled = ctx
                        .params
                        .get("enabled")
                        .and_then(|v| v.as_bool())
                        .ok_or_else(|| {
                            ErrorShape::new(error_codes::INVALID_REQUEST, "missing enabled")
                        })?;

                    ctx.state.set_graphql_enabled(enabled);

                    let mut persisted = true;
                    if let Err(error) = moltis_config::update_config(|cfg| {
                        cfg.graphql.enabled = enabled;
                    }) {
                        persisted = false;
                        tracing::warn!(%error, enabled, "failed to persist graphql config");
                    }

                    Ok(serde_json::json!({
                        "ok": true,
                        "enabled": enabled,
                        "persisted": persisted,
                    }))
                })
            }),
        );
    }

    // ── Memory ─────────────────────────────────────────────────────

    reg.register(
        "memory.status",
        Box::new(|ctx| {
            Box::pin(async move {
                if let Some(ref mm) = ctx.state.memory_manager {
                    match mm.status().await {
                        Ok(status) => Ok(serde_json::json!({
                            "available": true,
                            "total_files": status.total_files,
                            "total_chunks": status.total_chunks,
                            "db_size": status.db_size_bytes,
                            "db_size_display": status.db_size_display(),
                            "embedding_model": status.embedding_model,
                            "has_embeddings": mm.has_embeddings(),
                        })),
                        Err(e) => Ok(serde_json::json!({
                            "available": false,
                            "error": e.to_string(),
                        })),
                    }
                } else {
                    Ok(serde_json::json!({
                        "available": false,
                        "error": "Memory system not initialized",
                    }))
                }
            })
        }),
    );

    reg.register(
        "memory.config.get",
        Box::new(|_ctx| {
            Box::pin(async move {
                // Read memory config from the config file
                let config = moltis_config::discover_and_load();
                let memory = &config.memory;
                Ok(serde_json::json!({
                    "backend": memory.backend.as_deref().unwrap_or("builtin"),
                    "citations": memory.citations.as_deref().unwrap_or("auto"),
                    "disable_rag": memory.disable_rag,
                    "llm_reranking": memory.llm_reranking,
                    "session_export": memory.session_export,
                    "qmd_feature_enabled": cfg!(feature = "qmd"),
                }))
            })
        }),
    );

    reg.register(
        "memory.config.update",
        Box::new(|ctx| {
            Box::pin(async move {
                let backend = ctx
                    .params
                    .get("backend")
                    .and_then(|v| v.as_str())
                    .unwrap_or("builtin");
                let citations = ctx
                    .params
                    .get("citations")
                    .and_then(|v| v.as_str())
                    .unwrap_or("auto");
                let llm_reranking = ctx
                    .params
                    .get("llm_reranking")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let disable_rag = ctx.params.get("disable_rag").and_then(|v| v.as_bool());
                let session_export = ctx
                    .params
                    .get("session_export")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                // Persist to moltis.toml so the config survives restarts.
                let backend_str = backend.to_string();
                let citations_str = citations.to_string();
                let mut effective_disable_rag =
                    moltis_config::discover_and_load().memory.disable_rag;
                if let Err(e) = moltis_config::update_config(|cfg| {
                    cfg.memory.backend = Some(backend_str.clone());
                    cfg.memory.citations = Some(citations_str.clone());
                    cfg.memory.llm_reranking = llm_reranking;
                    if let Some(value) = disable_rag {
                        cfg.memory.disable_rag = value;
                    }
                    cfg.memory.session_export = session_export;
                    effective_disable_rag = cfg.memory.disable_rag;
                }) {
                    tracing::warn!(error = %e, "failed to persist memory config");
                }

                Ok(serde_json::json!({
                    "backend": backend,
                    "citations": citations,
                    "disable_rag": effective_disable_rag,
                    "llm_reranking": llm_reranking,
                    "session_export": session_export,
                }))
            })
        }),
    );

    // QMD status check
    reg.register(
        "memory.qmd.status",
        Box::new(|_ctx| {
            Box::pin(async move {
                #[cfg(feature = "qmd")]
                {
                    use moltis_qmd::{QmdManager, QmdManagerConfig};

                    let config = moltis_config::discover_and_load();
                    let qmd_config = QmdManagerConfig {
                        command: config
                            .memory
                            .qmd
                            .command
                            .clone()
                            .unwrap_or_else(|| "qmd".into()),
                        collections: HashMap::new(),
                        max_results: config.memory.qmd.max_results.unwrap_or(10),
                        timeout_ms: config.memory.qmd.timeout_ms.unwrap_or(30_000),
                        work_dir: moltis_config::data_dir(),
                    };

                    let manager = QmdManager::new(qmd_config);
                    let status = manager.status().await;

                    Ok(serde_json::json!({
                        "feature_enabled": true,
                        "available": status.available,
                        "version": status.version,
                        "error": status.error,
                    }))
                }

                #[cfg(not(feature = "qmd"))]
                {
                    Ok(serde_json::json!({
                        "feature_enabled": false,
                        "available": false,
                        "error": "QMD feature not enabled. Rebuild with --features qmd",
                    }))
                }
            })
        }),
    );

    // ── Hooks methods ────────────────────────────────────────────────

    // hooks.list — return discovered hooks with live stats.
    reg.register(
        "hooks.list",
        Box::new(|ctx| {
            Box::pin(async move {
                let inner = ctx.state.inner.read().await;
                let mut list = inner.discovered_hooks.clone();

                // Enrich with live stats from the registry.
                if let Some(ref registry) = inner.hook_registry {
                    for hook in &mut list {
                        if let Some(stats) = registry.handler_stats(&hook.name) {
                            let calls = stats.call_count.load(std::sync::atomic::Ordering::Relaxed);
                            let failures = stats
                                .failure_count
                                .load(std::sync::atomic::Ordering::Relaxed);
                            let total_us = stats
                                .total_latency_us
                                .load(std::sync::atomic::Ordering::Relaxed);
                            hook.call_count = calls;
                            hook.failure_count = failures;
                            hook.avg_latency_ms = total_us.checked_div(calls).unwrap_or(0) / 1000;
                        }
                    }
                }

                Ok(serde_json::json!({ "hooks": list }))
            })
        }),
    );

    // hooks.enable — re-enable a previously disabled hook.
    reg.register(
        "hooks.enable",
        Box::new(|ctx| {
            Box::pin(async move {
                let name = ctx
                    .params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ErrorShape::new(error_codes::INVALID_REQUEST, "missing name"))?;

                ctx.state.inner.write().await.disabled_hooks.remove(name);

                // Persist disabled hooks list.
                persist_disabled_hooks(&ctx.state).await;

                // Rebuild hooks.
                reload_hooks(&ctx.state).await;

                Ok(serde_json::json!({ "ok": true }))
            })
        }),
    );

    // hooks.disable — disable a hook without removing its files.
    reg.register(
        "hooks.disable",
        Box::new(|ctx| {
            Box::pin(async move {
                let name = ctx
                    .params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ErrorShape::new(error_codes::INVALID_REQUEST, "missing name"))?;

                ctx.state
                    .inner
                    .write()
                    .await
                    .disabled_hooks
                    .insert(name.to_string());

                // Persist disabled hooks list.
                persist_disabled_hooks(&ctx.state).await;

                // Rebuild hooks.
                reload_hooks(&ctx.state).await;

                Ok(serde_json::json!({ "ok": true }))
            })
        }),
    );

    // hooks.save — write HOOK.md content back to disk.
    reg.register(
        "hooks.save",
        Box::new(|ctx| {
            Box::pin(async move {
                let name = ctx
                    .params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ErrorShape::new(error_codes::INVALID_REQUEST, "missing name"))?;
                let content = ctx
                    .params
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ErrorShape::new(error_codes::INVALID_REQUEST, "missing content")
                    })?;

                // Find the hook's source path.
                let source_path = {
                    let inner = ctx.state.inner.read().await;
                    inner
                        .discovered_hooks
                        .iter()
                        .find(|h| h.name == name)
                        .map(|h| h.source_path.clone())
                };

                let source_path = source_path.ok_or_else(|| {
                    ErrorShape::new(error_codes::INVALID_REQUEST, "hook not found")
                })?;

                // Write the content to HOOK.md.
                let hook_md_path = PathBuf::from(&source_path).join("HOOK.md");
                std::fs::write(&hook_md_path, content).map_err(|e| {
                    ErrorShape::new(
                        error_codes::UNAVAILABLE,
                        format!("failed to write HOOK.md: {e}"),
                    )
                })?;

                // Reload hooks to pick up the changes.
                reload_hooks(&ctx.state).await;

                Ok(serde_json::json!({ "ok": true }))
            })
        }),
    );

    // hooks.reload — re-run discovery and rebuild the registry.
    reg.register(
        "hooks.reload",
        Box::new(|ctx| {
            Box::pin(async move {
                reload_hooks(&ctx.state).await;
                Ok(serde_json::json!({ "ok": true }))
            })
        }),
    );

    // ── OpenClaw import ─────────────────────────────────────────────────

    reg.register(
        "openclaw.detect",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .onboarding
                    .openclaw_detect()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "openclaw.scan",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .onboarding
                    .openclaw_scan()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
    reg.register(
        "openclaw.import",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .onboarding
                    .openclaw_import(ctx.params.clone())
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    // ── Logs ────────────────────────────────────────────────────────────────

    reg.register(
        "logs.tail",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .logs
                    .tail(ctx.params)
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    reg.register(
        "logs.list",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .logs
                    .list(ctx.params)
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    reg.register(
        "logs.status",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .logs
                    .status()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );

    reg.register(
        "logs.ack",
        Box::new(|ctx| {
            Box::pin(async move {
                ctx.state
                    .services
                    .logs
                    .ack()
                    .await
                    .map_err(ErrorShape::from)
            })
        }),
    );
}

async fn reload_hooks(state: &Arc<crate::state::GatewayState>) {
    let disabled = state.inner.read().await.disabled_hooks.clone();
    let session_store = state.services.session_store.as_ref();
    let (new_registry, new_info) =
        crate::server::discover_and_build_hooks(&disabled, session_store).await;

    {
        let mut inner = state.inner.write().await;
        inner.hook_registry = new_registry;
        inner.discovered_hooks = new_info.clone();
    }

    // Broadcast hooks.status event so connected UIs auto-refresh.
    broadcast(
        state,
        "hooks.status",
        serde_json::json!({ "hooks": new_info }),
        BroadcastOpts::default(),
    )
    .await;
}

/// Persist the disabled hooks set to `data_dir/disabled_hooks.json`.
async fn persist_disabled_hooks(state: &Arc<crate::state::GatewayState>) {
    let disabled = state.inner.read().await.disabled_hooks.clone();
    let path = moltis_config::data_dir().join("disabled_hooks.json");
    let json = serde_json::to_string_pretty(&disabled).unwrap_or_default();
    if let Err(e) = std::fs::write(&path, json) {
        warn!("failed to persist disabled hooks: {e}");
    }
}
