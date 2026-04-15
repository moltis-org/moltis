use std::sync::Arc;

use tracing::info;

use {
    moltis_channels::{ChannelReplyTarget, Error as ChannelError, Result as ChannelResult},
    moltis_sessions::metadata::SqliteSessionMetadata,
};

use crate::{
    broadcast::{BroadcastOpts, broadcast},
    state::GatewayState,
};

use super::super::{
    format_attachable_sessions_list, format_channel_sessions_list, is_attachable_session,
    parse_numbered_selection, resolve_channel_session_defaults, session_list_label,
};

// ── Session management command handlers ──────────────────────────

pub(in crate::channel_events) async fn handle_new(
    state: &Arc<GatewayState>,
    session_metadata: &SqliteSessionMetadata,
    session_key: &str,
    reply_to: &ChannelReplyTarget,
    sender_id: Option<&str>,
) -> ChannelResult<String> {
    // Create a new session with a fresh UUID key.
    let new_key = format!("session:{}", uuid::Uuid::new_v4());
    let binding_json = serde_json::to_string(reply_to)
        .map_err(|e| ChannelError::external("serialize channel binding", e))?;

    // Sequential label: count existing sessions for this chat.
    let existing = session_metadata
        .list_channel_sessions(
            reply_to.channel_type.as_str(),
            &reply_to.account_id,
            &reply_to.chat_id,
        )
        .await;
    let n = existing.len() + 1;

    // Create the new session entry with channel binding.
    session_metadata
        .upsert(
            &new_key,
            Some(format!("{} {n}", reply_to.channel_type.display_name())),
        )
        .await
        .map_err(|e| ChannelError::external("create channel session", e))?;
    session_metadata
        .set_channel_binding(&new_key, Some(binding_json.clone()))
        .await;

    // Ensure the old session also has a channel binding (for listing).
    let old_entry = session_metadata.get(session_key).await;
    let channel_defaults = resolve_channel_session_defaults(state, reply_to, sender_id).await;
    if old_entry
        .as_ref()
        .and_then(|e| e.channel_binding.as_ref())
        .is_none()
    {
        session_metadata
            .set_channel_binding(session_key, Some(binding_json))
            .await;
    }

    let inherited_agent = old_entry
        .as_ref()
        .and_then(|entry| entry.agent_id.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let target_agent = if let Some(agent_id) = inherited_agent {
        agent_id
    } else if let Some(agent_id) = channel_defaults.agent_id.clone() {
        agent_id
    } else if let Some(ref store) = state.services.agent_persona_store {
        store
            .default_id()
            .await
            .unwrap_or_else(|_| "main".to_string())
    } else {
        "main".to_string()
    };
    let _ = session_metadata
        .set_agent_id(&new_key, Some(&target_agent))
        .await;

    // Update forward mapping.
    session_metadata
        .set_active_session(
            reply_to.channel_type.as_str(),
            &reply_to.account_id,
            &reply_to.chat_id,
            reply_to.thread_id.as_deref(),
            &new_key,
        )
        .await;

    info!(
        old_session = %session_key,
        new_session = %new_key,
        "channel /new: created new session"
    );

    // Assign a model to the new session: prefer the channel's
    // configured model, fall back to the first registered model.
    let models_val = state.services.model.list().await.ok();
    let models = models_val.as_ref().and_then(|v| v.as_array());

    let (model_id, model_display): (Option<String>, String) =
        if let Some(ref cm) = channel_defaults.model {
            let d = models
                .and_then(|ms| {
                    ms.iter()
                        .find(|m| m.get("id").and_then(|v| v.as_str()) == Some(cm.as_str()))
                        .and_then(|m| m.get("displayName").and_then(|v| v.as_str()))
                })
                .unwrap_or(cm.as_str());
            (Some(cm.clone()), d.to_string())
        } else if let Some(ms) = models
            && let Some(first) = ms.first()
            && let Some(id) = first.get("id").and_then(|v| v.as_str())
        {
            let d = first
                .get("displayName")
                .and_then(|v| v.as_str())
                .unwrap_or(id);
            (Some(id.to_string()), d.to_string())
        } else {
            (None, String::new())
        };

    if let Some(ref mid) = model_id {
        let _ = state
            .services
            .session
            .patch(serde_json::json!({
                "key": &new_key,
                "model": mid,
            }))
            .await;
    }

    // Notify web UI so the session list refreshes.
    broadcast(
        state,
        "session",
        serde_json::json!({
            "kind": "created",
            "sessionKey": &new_key,
        }),
        BroadcastOpts {
            drop_if_slow: true,
            ..Default::default()
        },
    )
    .await;

    if model_display.is_empty() {
        Ok("New session started.".to_string())
    } else {
        Ok(format!(
            "New session started. Using *{model_display}*. Use /model to change."
        ))
    }
}

pub(in crate::channel_events) async fn handle_clear(
    state: &Arc<GatewayState>,
    session_key: &str,
) -> ChannelResult<String> {
    let chat = state.chat().await;
    let params = serde_json::json!({ "_session_key": session_key });
    chat.clear(params)
        .await
        .map_err(ChannelError::unavailable)?;
    Ok("Session cleared.".to_string())
}

pub(in crate::channel_events) async fn handle_compact(
    state: &Arc<GatewayState>,
    session_key: &str,
) -> ChannelResult<String> {
    let chat = state.chat().await;
    let params = serde_json::json!({ "_session_key": session_key });
    chat.compact(params)
        .await
        .map_err(ChannelError::unavailable)?;
    Ok("Session compacted.".to_string())
}

pub(in crate::channel_events) async fn handle_context(
    state: &Arc<GatewayState>,
    session_key: &str,
) -> ChannelResult<String> {
    let chat = state.chat().await;
    let params = serde_json::json!({ "_session_key": session_key });
    let res = chat
        .context(params)
        .await
        .map_err(ChannelError::unavailable)?;

    let session_info = res.get("session").cloned().unwrap_or_default();
    let msg_count = session_info
        .get("messageCount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let provider = session_info
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let model = session_info
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let tokens = res.get("tokenUsage").cloned().unwrap_or_default();
    let total = tokens.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
    let context_window = tokens
        .get("contextWindow")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    // Sandbox section
    let sandbox = res.get("sandbox").cloned().unwrap_or_default();
    let sandbox_enabled = sandbox
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let sandbox_line = if sandbox_enabled {
        let image = sandbox
            .get("image")
            .and_then(|v| v.as_str())
            .unwrap_or("default");
        format!("**Sandbox:** on \u{00b7} `{image}`")
    } else {
        "**Sandbox:** off".to_string()
    };

    // Skills/plugins section
    let skills = res
        .get("skills")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let skills_line = if skills.is_empty() {
        "**Plugins:** none".to_string()
    } else {
        let names: Vec<_> = skills
            .iter()
            .filter_map(|s| s.get("name").and_then(|v| v.as_str()))
            .collect();
        format!("**Plugins:** {}", names.join(", "))
    };

    Ok(format!(
        "**Session:** `{session_key}`\n**Messages:** {msg_count}\n**Provider:** {provider}\n**Model:** `{model}`\n{sandbox_line}\n{skills_line}\n**Tokens:** ~{total}/{context_window}"
    ))
}

pub(in crate::channel_events) async fn handle_sessions(
    state: &Arc<GatewayState>,
    session_metadata: &SqliteSessionMetadata,
    session_key: &str,
    reply_to: &ChannelReplyTarget,
    args: &str,
) -> ChannelResult<String> {
    let sessions = session_metadata
        .list_channel_sessions(
            reply_to.channel_type.as_str(),
            &reply_to.account_id,
            &reply_to.chat_id,
        )
        .await;

    if sessions.is_empty() {
        return Ok("No sessions found. Send a message to start one.".to_string());
    }

    if args.is_empty() {
        Ok(format_channel_sessions_list(&sessions, session_key))
    } else {
        // Switch mode.
        let n = parse_numbered_selection(args, "sessions")?;
        if n == 0 || n > sessions.len() {
            return Err(ChannelError::invalid_input(format!(
                "invalid session number. Use 1\u{2013}{}.",
                sessions.len()
            )));
        }
        let target_session = &sessions[n - 1];

        // Update forward mapping.
        session_metadata
            .set_active_session(
                reply_to.channel_type.as_str(),
                &reply_to.account_id,
                &reply_to.chat_id,
                reply_to.thread_id.as_deref(),
                &target_session.key,
            )
            .await;

        let label = target_session
            .label
            .as_deref()
            .unwrap_or(&target_session.key);
        info!(
            session = %target_session.key,
            "channel /sessions: switched session"
        );

        broadcast(
            state,
            "session",
            serde_json::json!({
                "kind": "switched",
                "sessionKey": &target_session.key,
            }),
            BroadcastOpts {
                drop_if_slow: true,
                ..Default::default()
            },
        )
        .await;

        Ok(format!("Switched to: {label}"))
    }
}

pub(in crate::channel_events) async fn handle_attach(
    state: &Arc<GatewayState>,
    session_metadata: &SqliteSessionMetadata,
    session_key: &str,
    reply_to: &ChannelReplyTarget,
    args: &str,
) -> ChannelResult<String> {
    let sessions: Vec<_> = session_metadata
        .list_account_sessions(reply_to.channel_type.as_str(), &reply_to.account_id)
        .await
        .into_iter()
        .filter(is_attachable_session)
        .collect();

    if sessions.is_empty() {
        return Ok("No attachable sessions found yet.".to_string());
    }

    if args.is_empty() {
        return Ok(format_attachable_sessions_list(&sessions, session_key));
    }

    let n = parse_numbered_selection(args, "attach")?;
    if n == 0 || n > sessions.len() {
        return Err(ChannelError::invalid_input(format!(
            "invalid session number. Use 1\u{2013}{}.",
            sessions.len()
        )));
    }

    let target_session = &sessions[n - 1];
    let binding_json = serde_json::to_string(reply_to)
        .map_err(|e| ChannelError::external("serialize channel binding", e))?;

    session_metadata
        .clear_active_session_mappings(&target_session.key)
        .await;
    session_metadata
        .set_channel_binding(&target_session.key, Some(binding_json))
        .await;
    session_metadata
        .set_active_session(
            reply_to.channel_type.as_str(),
            &reply_to.account_id,
            &reply_to.chat_id,
            reply_to.thread_id.as_deref(),
            &target_session.key,
        )
        .await;

    let label = session_list_label(target_session);
    info!(
        session = %target_session.key,
        "channel /attach: rebound existing session to current chat"
    );

    broadcast(
        state,
        "session",
        serde_json::json!({
            "kind": "switched",
            "sessionKey": &target_session.key,
        }),
        BroadcastOpts {
            drop_if_slow: true,
            ..Default::default()
        },
    )
    .await;

    Ok(format!("Attached here: {label}"))
}
