use std::sync::Arc;

use moltis_channels::{ChannelReplyTarget, Error as ChannelError, Result as ChannelResult};

use crate::state::GatewayState;

use super::resolve_channel_session;

mod attachments;
mod control_handlers;
pub(in crate::channel_events) mod formatting;
mod location;
mod media;
mod session_handlers;

// Re-export everything that `channel_events.rs` uses via `commands::*`.
pub(super) use {
    attachments::dispatch_to_chat_with_attachments,
    location::{resolve_pending_location, update_location},
    media::{
        request_sender_approval, save_channel_attachment, save_channel_voice, transcribe_voice,
        voice_stt_available,
    },
};

pub(super) async fn dispatch_interaction(
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

pub(super) async fn dispatch_command(
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
        "clear" => session_handlers::handle_clear(state, &session_key).await,
        "compact" => session_handlers::handle_compact(state, &session_key).await,
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
        "model" => {
            control_handlers::handle_model(state, session_metadata, &session_key, args).await
        },
        "sandbox" => {
            control_handlers::handle_sandbox(state, session_metadata, &session_key, args).await
        },
        "sh" => control_handlers::handle_sh(state, &session_key, args).await,
        "stop" => control_handlers::handle_stop(state, &session_key).await,
        "peek" => control_handlers::handle_peek(state, &session_key).await,
        _ => Err(ChannelError::invalid_input(format!(
            "unknown command: /{cmd}"
        ))),
    }
}
