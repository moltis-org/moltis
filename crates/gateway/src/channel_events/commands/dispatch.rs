use std::sync::Arc;

use moltis_channels::{ChannelReplyTarget, Error as ChannelError, Result as ChannelResult};

use crate::state::GatewayState;

use super::{super::resolve_channel_session, control_handlers, quick_actions, session_handlers};

pub(in crate::channel_events) async fn dispatch_interaction(
    state: &Arc<tokio::sync::OnceCell<Arc<GatewayState>>>,
    callback_data: &str,
    reply_to: ChannelReplyTarget,
) -> ChannelResult<String> {
    // Map callback_data prefixes to slash-command text, following the same
    // convention used by Telegram's handle_callback_query.
    let cmd_text = if let Some(n) = callback_data.strip_prefix("sessions_switch:") {
        format!("sessions {n}")
    } else if let Some(n) = callback_data.strip_prefix("agent_switch:") {
        format!("agent {n}")
    } else if let Some(n) = callback_data.strip_prefix("model_switch:") {
        format!("model {n}")
    } else if let Some(val) = callback_data.strip_prefix("sandbox_toggle:") {
        format!("sandbox {val}")
    } else if let Some(n) = callback_data.strip_prefix("sandbox_image:") {
        format!("sandbox image {n}")
    } else if let Some(provider) = callback_data.strip_prefix("model_provider:") {
        format!("model provider:{provider}")
    } else {
        return Err(ChannelError::invalid_input(format!(
            "unknown interaction callback: {callback_data}"
        )));
    };

    dispatch_command(state, &cmd_text, reply_to, None).await
}

pub(in crate::channel_events) async fn dispatch_command(
    state: &Arc<tokio::sync::OnceCell<Arc<GatewayState>>>,
    command: &str,
    reply_to: ChannelReplyTarget,
    sender_id: Option<&str>,
) -> ChannelResult<String> {
    let state = state
        .get()
        .ok_or_else(|| ChannelError::unavailable("gateway not ready"))?;
    let session_metadata = state
        .services
        .session_metadata
        .as_ref()
        .ok_or_else(|| ChannelError::unavailable("session metadata not available"))?;
    let session_key = resolve_channel_session(&reply_to, session_metadata).await;

    // Strip leading slash — some channels (e.g. Slack) include it in the
    // command field, others (Telegram, Discord) strip it before calling.
    let command = command.strip_prefix('/').unwrap_or(command);

    // Extract the command name (first word) and args (rest).
    let cmd = command.split_whitespace().next().unwrap_or("");
    let args = command[cmd.len()..].trim();

    match cmd {
        // Session management commands
        "new" => {
            session_handlers::handle_new(
                state,
                session_metadata,
                &session_key,
                &reply_to,
                sender_id,
            )
            .await
        },
        "fork" => session_handlers::handle_fork(state, &session_key, args).await,
        "clear" => session_handlers::handle_clear(state, &session_key).await,
        "compact" => session_handlers::handle_compact(state, &session_key).await,
        "title" => session_handlers::handle_title(state, &session_key).await,
        "context" => session_handlers::handle_context(state, &session_key).await,
        "sessions" => {
            session_handlers::handle_sessions(
                state,
                session_metadata,
                &session_key,
                &reply_to,
                args,
            )
            .await
        },
        "attach" => {
            session_handlers::handle_attach(state, session_metadata, &session_key, &reply_to, args)
                .await
        },

        // Control commands
        "approvals" => control_handlers::handle_approvals(state, &session_key).await,
        "approve" | "deny" => {
            control_handlers::handle_approve_deny(
                state,
                &session_key,
                &reply_to,
                sender_id,
                cmd,
                args,
            )
            .await
        },
        "agent" => {
            control_handlers::handle_agent(state, session_metadata, &session_key, args).await
        },
        "mode" => control_handlers::handle_mode(state, session_metadata, &session_key, args).await,
        "model" => {
            control_handlers::handle_model(state, session_metadata, &session_key, args).await
        },
        "sandbox" => {
            control_handlers::handle_sandbox(state, session_metadata, &session_key, args).await
        },
        "sh" => control_handlers::handle_sh(state, &session_key, args).await,
        "stop" => control_handlers::handle_stop(state, &session_key).await,
        "peek" => control_handlers::handle_peek(state, &session_key).await,
        "tts" => control_handlers::handle_tts(state, &session_key, args).await,
        "update" => control_handlers::handle_update(state, &reply_to, sender_id, args).await,

        "rollback" => quick_actions::handle_rollback(state, &session_key, args).await,

        // Quick actions
        "btw" => quick_actions::handle_btw(state, &session_key, args).await,
        "fast" => quick_actions::handle_fast(state, session_metadata, &session_key, args).await,
        "insights" => quick_actions::handle_insights(state, args).await,
        "steer" => quick_actions::handle_steer(state, &session_key, args).await,
        "queue" => quick_actions::handle_queue(state, &session_key, args).await,
        _ => Err(ChannelError::invalid_input(format!(
            "unknown command: /{cmd}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    /// Verify that every command in the centralized registry (except `"help"`,
    /// which channels handle locally) has a matching arm in `dispatch_command`.
    ///
    /// This is a compile/test-time safety net: if a new command is added to the
    /// registry but not wired into gateway dispatch, this test will fail.
    #[test]
    fn all_registered_commands_have_dispatch_arms() {
        // The set of commands that channels handle locally and never reach
        // gateway dispatch.
        let locally_handled = ["help"];

        // The set of commands that dispatch_command handles.  Keep this list
        // in sync with the match arms above.
        let dispatched = [
            "new",
            "fork",
            "clear",
            "compact",
            "title",
            "context",
            "sessions",
            "attach",
            "approvals",
            "approve",
            "deny",
            "agent",
            "mode",
            "model",
            "sandbox",
            "sh",
            "stop",
            "peek",
            "update",
            "rollback",
            "btw",
            "fast",
            "insights",
            "steer",
            "queue",
        ];

        for cmd in moltis_channels::commands::all_commands() {
            if locally_handled.contains(&cmd.name) {
                continue;
            }
            assert!(
                dispatched.contains(&cmd.name),
                "command `/{name}` is registered in moltis_channels::commands but has no \
                 dispatch arm in gateway dispatch_command. Add a match arm or update this test.",
                name = cmd.name,
            );
        }

        // Reverse check: every dispatch arm should be in the registry.
        let registry_names: Vec<&str> = moltis_channels::commands::all_commands()
            .iter()
            .map(|c| c.name)
            .collect();
        for name in &dispatched {
            assert!(
                registry_names.contains(name),
                "dispatch arm `/{name}` exists but is not in the centralized command registry. \
                 Add it to moltis_channels::commands::all_commands().",
            );
        }
    }
}
