use std::sync::Arc;

use {
    moltis_channels::{ChannelReplyTarget, Error as ChannelError, Result as ChannelResult},
    moltis_sessions::metadata::SqliteSessionMetadata,
    moltis_tools::image_cache::ImageBuilder,
};

use crate::{
    broadcast::{BroadcastOpts, broadcast},
    state::GatewayState,
};

use super::{
    super::{
        ApprovalListResponse, format_pending_approvals_list, is_sender_on_allowlist,
        parse_numbered_selection,
    },
    formatting::{format_model_list, unique_providers},
};

// ── Control command handlers ─────────────────────────────────────

pub(in crate::channel_events) async fn handle_approvals(
    state: &Arc<GatewayState>,
    session_key: &str,
) -> ChannelResult<String> {
    let response = state
        .services
        .exec_approval
        .request(serde_json::json!({ "sessionKey": session_key }))
        .await
        .map_err(ChannelError::unavailable)?;
    let approvals: ApprovalListResponse = serde_json::from_value(response)
        .map_err(|e| ChannelError::external("parse approval list", e))?;

    if approvals.requests.is_empty() {
        Ok("No pending approvals for this session.".to_string())
    } else {
        Ok(format_pending_approvals_list(&approvals.requests))
    }
}

pub(in crate::channel_events) async fn handle_approve_deny(
    state: &Arc<GatewayState>,
    session_key: &str,
    reply_to: &ChannelReplyTarget,
    sender_id: Option<&str>,
    cmd: &str,
    args: &str,
) -> ChannelResult<String> {
    let authorized = match sender_id {
        Some(sid) => is_sender_on_allowlist(state, &reply_to.account_id, sid).await,
        None => false,
    };
    if !authorized {
        return Err(ChannelError::invalid_input(
            "You are not authorized to manage approvals. Only users on this bot's allowlist can use /approve and /deny.",
        ));
    }
    if args.is_empty() {
        return Err(ChannelError::invalid_input(format!(
            "usage: /{cmd} [number]"
        )));
    }

    let response = state
        .services
        .exec_approval
        .request(serde_json::json!({ "sessionKey": session_key }))
        .await
        .map_err(ChannelError::unavailable)?;
    let approvals: ApprovalListResponse = serde_json::from_value(response)
        .map_err(|e| ChannelError::external("parse approval list", e))?;

    if approvals.requests.is_empty() {
        return Ok("No pending approvals for this session.".to_string());
    }

    let n = parse_numbered_selection(args, cmd)?;
    if n == 0 || n > approvals.requests.len() {
        return Err(ChannelError::invalid_input(format!(
            "invalid approval number. Use 1\u{2013}{}.",
            approvals.requests.len()
        )));
    }

    let request = &approvals.requests[n - 1];
    let decision = if cmd == "approve" {
        "approved"
    } else {
        "denied"
    };
    let mut params = serde_json::json!({
        "requestId": &request.id,
        "decision": decision,
    });
    if cmd == "approve" {
        params["command"] = serde_json::json!(&request.command);
    }

    state
        .services
        .exec_approval
        .resolve(params)
        .await
        .map_err(ChannelError::unavailable)?;

    use crate::approval::{MAX_COMMAND_PREVIEW_LEN, truncate_command_preview};
    let preview = truncate_command_preview(&request.command, MAX_COMMAND_PREVIEW_LEN);
    if cmd == "approve" {
        Ok(format!("Approved: `{preview}`"))
    } else {
        Ok(format!("Denied: `{preview}`"))
    }
}

pub(in crate::channel_events) async fn handle_agent(
    state: &Arc<GatewayState>,
    session_metadata: &SqliteSessionMetadata,
    session_key: &str,
    args: &str,
) -> ChannelResult<String> {
    let Some(ref store) = state.services.agent_persona_store else {
        return Err(ChannelError::unavailable(
            "agent personas are not available",
        ));
    };
    let default_id = store
        .default_id()
        .await
        .unwrap_or_else(|_| "main".to_string());
    let agents = store
        .list()
        .await
        .map_err(|e| ChannelError::external("listing agents", e))?;
    let current_agent = session_metadata
        .get(session_key)
        .await
        .and_then(|entry| entry.agent_id)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(default_id.clone());

    if args.is_empty() {
        let mut lines = Vec::new();
        for (i, agent) in agents.iter().enumerate() {
            let marker = if agent.id == current_agent {
                " *"
            } else {
                ""
            };
            let default_badge = if agent.id == default_id {
                " (default)"
            } else {
                ""
            };
            let emoji = agent.emoji.clone().unwrap_or_default();
            let label = if emoji.is_empty() {
                agent.name.clone()
            } else {
                format!("{emoji} {}", agent.name)
            };
            lines.push(format!(
                "{}. {} [{}]{}{}",
                i + 1,
                label,
                agent.id,
                default_badge,
                marker,
            ));
        }
        lines.push("\nUse /agent N to switch.".to_string());
        Ok(lines.join("\n"))
    } else {
        let n: usize = args
            .parse()
            .map_err(|_| ChannelError::invalid_input("usage: /agent [number]"))?;
        if n == 0 || n > agents.len() {
            return Err(ChannelError::invalid_input(format!(
                "invalid agent number. Use 1\u{2013}{}.",
                agents.len()
            )));
        }
        let chosen = &agents[n - 1];
        session_metadata
            .set_agent_id(session_key, Some(&chosen.id))
            .await
            .map_err(|e| ChannelError::external("setting session agent", e))?;

        broadcast(
            state,
            "session",
            serde_json::json!({
                "kind": "patched",
                "sessionKey": session_key,
            }),
            BroadcastOpts {
                drop_if_slow: true,
                ..Default::default()
            },
        )
        .await;

        let emoji = chosen.emoji.clone().unwrap_or_default();
        if emoji.is_empty() {
            Ok(format!("Agent switched to: {}", chosen.name))
        } else {
            Ok(format!("Agent switched to: {} {}", emoji, chosen.name))
        }
    }
}

pub(in crate::channel_events) async fn handle_model(
    state: &Arc<GatewayState>,
    session_metadata: &SqliteSessionMetadata,
    session_key: &str,
    args: &str,
) -> ChannelResult<String> {
    let models_val = state
        .services
        .model
        .list()
        .await
        .map_err(ChannelError::unavailable)?;
    let models = models_val
        .as_array()
        .ok_or_else(|| ChannelError::invalid_input("bad model list"))?;

    let current_model = {
        let entry = session_metadata.get(session_key).await;
        entry.and_then(|e| e.model.clone())
    };

    if args.is_empty() {
        // List unique providers (sorted, deduplicated).
        let providers = unique_providers(models);

        if providers.len() <= 1 {
            // Single provider -- list models directly.
            return Ok(format_model_list(models, current_model.as_deref(), None));
        }

        // Multiple providers -- list them for selection.
        // Prefix with "providers:" so Telegram handler knows.
        let current_provider = current_model.as_deref().and_then(|cm| {
            models.iter().find_map(|m| {
                let id = m.get("id").and_then(|v| v.as_str())?;
                if id == cm {
                    m.get("provider").and_then(|v| v.as_str()).map(String::from)
                } else {
                    None
                }
            })
        });
        let mut lines = vec!["providers:".to_string()];
        for (i, p) in providers.iter().enumerate() {
            let count = models
                .iter()
                .filter(|m| m.get("provider").and_then(|v| v.as_str()) == Some(p))
                .count();
            let marker = if current_provider.as_deref() == Some(p) {
                " *"
            } else {
                ""
            };
            lines.push(format!("{}. {} ({} models){}", i + 1, p, count, marker));
        }
        Ok(lines.join("\n"))
    } else if let Some(provider) = args.strip_prefix("provider:") {
        // List models for a specific provider.
        Ok(format_model_list(
            models,
            current_model.as_deref(),
            Some(provider),
        ))
    } else {
        // Switch mode -- arg is a 1-based global index.
        let n: usize = args
            .parse()
            .map_err(|_| ChannelError::invalid_input("usage: /model [number]"))?;
        if n == 0 || n > models.len() {
            return Err(ChannelError::invalid_input(format!(
                "invalid model number. Use 1\u{2013}{}.",
                models.len()
            )));
        }
        let chosen = &models[n - 1];
        let model_id = chosen
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ChannelError::invalid_input("model has no id"))?;
        let display = chosen
            .get("displayName")
            .and_then(|v| v.as_str())
            .unwrap_or(model_id);

        let patch_res = state
            .services
            .session
            .patch(serde_json::json!({
                "key": session_key,
                "model": model_id,
            }))
            .await
            .map_err(ChannelError::unavailable)?;
        let version = patch_res
            .get("version")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        broadcast(
            state,
            "session",
            serde_json::json!({
                "kind": "patched",
                "sessionKey": session_key,
                "version": version,
            }),
            BroadcastOpts {
                drop_if_slow: true,
                ..Default::default()
            },
        )
        .await;

        Ok(format!("Model switched to: {display}"))
    }
}

pub(in crate::channel_events) async fn handle_sandbox(
    state: &Arc<GatewayState>,
    session_metadata: &SqliteSessionMetadata,
    session_key: &str,
    args: &str,
) -> ChannelResult<String> {
    let is_enabled = if let Some(ref router) = state.sandbox_router {
        router.is_sandboxed(session_key).await
    } else {
        false
    };

    if args.is_empty() {
        // Show current status and image list.
        let current_image = {
            let entry = session_metadata.get(session_key).await;
            let session_img = entry.and_then(|e| e.sandbox_image.clone());
            match session_img {
                Some(img) if !img.is_empty() => img,
                _ => {
                    if let Some(ref router) = state.sandbox_router {
                        router.default_image().await
                    } else {
                        moltis_tools::sandbox::DEFAULT_SANDBOX_IMAGE.to_string()
                    }
                },
            }
        };

        let status = if is_enabled {
            "on"
        } else {
            "off"
        };

        // List available images.
        let builder = moltis_tools::image_cache::DockerImageBuilder::new();
        let cached = builder.list_cached().await.unwrap_or_default();

        let default_img = moltis_tools::sandbox::DEFAULT_SANDBOX_IMAGE.to_string();
        let mut images: Vec<(String, Option<String>)> = vec![(default_img.clone(), None)];
        for img in &cached {
            images.push((
                img.tag.clone(),
                Some(format!("{} ({})", img.skill_name, img.size)),
            ));
        }

        let mut lines = vec![format!("status:{status}")];
        for (i, (tag, subtitle)) in images.iter().enumerate() {
            let marker = if *tag == current_image {
                " *"
            } else {
                ""
            };
            let label = if let Some(sub) = subtitle {
                format!("{}. {} \u{2014} {}{}", i + 1, tag, sub, marker)
            } else {
                format!("{}. {}{}", i + 1, tag, marker)
            };
            lines.push(label);
        }
        Ok(lines.join("\n"))
    } else if args == "on" || args == "off" {
        let new_val = args == "on";
        let patch_res = state
            .services
            .session
            .patch(serde_json::json!({
                "key": session_key,
                "sandbox_enabled": new_val,
            }))
            .await
            .map_err(ChannelError::unavailable)?;
        let version = patch_res
            .get("version")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        broadcast(
            state,
            "session",
            serde_json::json!({
                "kind": "patched",
                "sessionKey": session_key,
                "version": version,
            }),
            BroadcastOpts {
                drop_if_slow: true,
                ..Default::default()
            },
        )
        .await;
        let label = if new_val {
            "enabled"
        } else {
            "disabled"
        };
        Ok(format!("Sandbox {label}."))
    } else if let Some(rest) = args.strip_prefix("image ") {
        let n: usize = rest
            .parse()
            .map_err(|_| ChannelError::invalid_input("usage: /sandbox image [number]"))?;

        let default_img = moltis_tools::sandbox::DEFAULT_SANDBOX_IMAGE.to_string();
        let builder = moltis_tools::image_cache::DockerImageBuilder::new();
        let cached = builder.list_cached().await.unwrap_or_default();
        let mut images: Vec<String> = vec![default_img];
        for img in &cached {
            images.push(img.tag.clone());
        }

        if n == 0 || n > images.len() {
            return Err(ChannelError::invalid_input(format!(
                "invalid image number. Use 1\u{2013}{}.",
                images.len()
            )));
        }
        let chosen = &images[n - 1];

        // If choosing the default image, clear the session override.
        let patch_value = if n == 1 {
            ""
        } else {
            chosen.as_str()
        };
        let patch_res = state
            .services
            .session
            .patch(serde_json::json!({
                "key": session_key,
                "sandbox_image": patch_value,
            }))
            .await
            .map_err(ChannelError::unavailable)?;
        let version = patch_res
            .get("version")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        broadcast(
            state,
            "session",
            serde_json::json!({
                "kind": "patched",
                "sessionKey": session_key,
                "version": version,
            }),
            BroadcastOpts {
                drop_if_slow: true,
                ..Default::default()
            },
        )
        .await;

        Ok(format!("Image set to: {chosen}"))
    } else {
        Err(ChannelError::invalid_input(
            "usage: /sandbox [on|off|image N]",
        ))
    }
}

pub(in crate::channel_events) async fn handle_sh(
    state: &Arc<GatewayState>,
    session_key: &str,
    args: &str,
) -> ChannelResult<String> {
    let route = if let Some(ref router) = state.sandbox_router {
        if router.is_sandboxed(session_key).await {
            "sandboxed"
        } else {
            "host"
        }
    } else {
        "host"
    };

    match args {
        "" | "on" => {
            state.set_channel_command_mode(session_key, true).await;
            Ok(format!(
                "Command mode enabled ({route}). Send commands as plain messages. Use /sh off (or /sh exit) to leave."
            ))
        },
        "off" | "exit" => {
            state.set_channel_command_mode(session_key, false).await;
            Ok("Command mode disabled. Back to normal chat mode.".to_string())
        },
        "status" => {
            let enabled = state.is_channel_command_mode_enabled(session_key).await;
            if enabled {
                Ok(format!(
                    "Command mode is enabled ({route}). Use /sh off (or /sh exit) to leave."
                ))
            } else {
                Ok(format!(
                    "Command mode is disabled ({route}). Use /sh to enable."
                ))
            }
        },
        _ => Err(ChannelError::invalid_input(
            "usage: /sh [on|off|exit|status]",
        )),
    }
}

pub(in crate::channel_events) async fn handle_stop(
    state: &Arc<GatewayState>,
    session_key: &str,
) -> ChannelResult<String> {
    let chat = state.chat().await;
    let params = serde_json::json!({ "sessionKey": session_key });
    match chat.abort(params).await {
        Ok(res) => {
            let aborted = res
                .get("aborted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if aborted {
                Ok("Stopped.".to_string())
            } else {
                Ok("Nothing to stop.".to_string())
            }
        },
        Err(e) => Err(ChannelError::external("abort", e)),
    }
}

pub(in crate::channel_events) async fn handle_peek(
    state: &Arc<GatewayState>,
    session_key: &str,
) -> ChannelResult<String> {
    let chat = state.chat().await;
    let params = serde_json::json!({ "sessionKey": session_key });
    match chat.peek(params).await {
        Ok(res) => {
            let active = res.get("active").and_then(|v| v.as_bool()).unwrap_or(false);
            if !active {
                return Ok("Idle \u{2014} nothing running.".to_string());
            }
            let mut lines = Vec::new();
            if let Some(text) = res.get("thinkingText").and_then(|v| v.as_str()) {
                lines.push(format!("Thinking: {text}"));
            }
            if let Some(tools) = res.get("toolCalls").and_then(|v| v.as_array()) {
                for tc in tools {
                    let name = tc.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                    lines.push(format!("  Running: {name}"));
                }
            }
            if lines.is_empty() {
                lines.push("Active (thinking\u{2026})".to_string());
            }
            Ok(lines.join("\n"))
        },
        Err(e) => Err(ChannelError::external("peek", e)),
    }
}
