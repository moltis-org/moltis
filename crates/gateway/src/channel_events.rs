use std::sync::Arc;

use {
    async_trait::async_trait,
    serde::Deserialize,
    tracing::{debug, error, warn},
};

use {
    moltis_channels::{
        ChannelAttachment, ChannelEvent, ChannelEventSink, ChannelMessageMeta, ChannelReplyTarget,
        Error as ChannelError, Result as ChannelResult, SavedChannelFile,
    },
    moltis_sessions::metadata::{SessionEntry, SqliteSessionMetadata},
    moltis_tools::approval::PendingApprovalView,
};

use crate::{
    broadcast::{BroadcastOpts, broadcast},
    state::GatewayState,
};

/// Default (deterministic) session key for a channel chat.
///
/// For Telegram forum topics the thread ID is appended so each topic gets its
/// own session: `telegram:bot:chat:thread`.
fn default_channel_session_key(target: &ChannelReplyTarget) -> String {
    match &target.thread_id {
        Some(tid) => format!(
            "{}:{}:{}:{}",
            target.channel_type, target.account_id, target.chat_id, tid
        ),
        None => format!(
            "{}:{}:{}",
            target.channel_type, target.account_id, target.chat_id
        ),
    }
}

/// Resolve the active session key for a channel chat.
/// Uses the forward mapping table if an override exists, otherwise falls back
/// to the deterministic key.
async fn resolve_channel_session(
    target: &ChannelReplyTarget,
    metadata: &SqliteSessionMetadata,
) -> String {
    if let Some(key) = metadata
        .get_active_session(
            target.channel_type.as_str(),
            &target.account_id,
            &target.chat_id,
            target.thread_id.as_deref(),
        )
        .await
    {
        return key;
    }
    default_channel_session_key(target)
}

fn slash_command_name(text: &str) -> Option<&str> {
    let rest = text.trim_start().strip_prefix('/')?;
    let cmd = rest.split_whitespace().next().unwrap_or("");
    if cmd.is_empty() {
        None
    } else {
        Some(cmd)
    }
}

fn is_channel_control_command_name(cmd: &str) -> bool {
    matches!(
        cmd,
        "new"
            | "clear"
            | "compact"
            | "context"
            | "model"
            | "sandbox"
            | "sessions"
            | "attach"
            | "approvals"
            | "approve"
            | "deny"
            | "agent"
            | "help"
            | "sh"
            | "peek"
            | "stop"
    )
}

fn rewrite_for_shell_mode(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(cmd) = slash_command_name(trimmed)
        && is_channel_control_command_name(cmd)
    {
        return None;
    }

    Some(format!("/sh {trimmed}"))
}

fn parse_numbered_selection(arg: &str, command_name: &str) -> ChannelResult<usize> {
    arg.parse()
        .map_err(|_| ChannelError::invalid_input(format!("usage: /{command_name} [number]")))
}

/// Check whether `sender_id` is on the channel account's DM allowlist.
///
/// The DM allowlist is the source of truth for privileged command access:
/// anyone allowed to DM the bot is trusted to run commands like `/approve`
/// and `/deny` from any context (DM or group). Users not on the allowlist
/// can still chat (if group policy permits) but cannot run privileged
/// commands.
///
/// Returns `false` when the allowlist is empty (open DM policy) because an
/// open policy means no one has been explicitly authorized.
async fn is_sender_on_allowlist(
    state: &Arc<GatewayState>,
    account_id: &str,
    sender_id: &str,
) -> bool {
    let Some(ref registry) = state.services.channel_registry else {
        return false;
    };
    let Some(config) = registry.account_config(account_id).await else {
        return false;
    };
    let allowlist = config.allowlist();
    // Empty allowlist = open policy → no explicit authorization.
    if allowlist.is_empty() {
        return false;
    }
    // Check the full sender_id first, then try the user part before '@'
    // (WhatsApp JIDs are e.g. "15551234567@s.whatsapp.net" but allowlists
    // use plain phone numbers like "15551234567").
    moltis_channels::gating::is_allowed(sender_id, allowlist)
        || sender_id
            .split_once('@')
            .is_some_and(|(user, _)| moltis_channels::gating::is_allowed(user, allowlist))
}

fn is_attachable_session(entry: &SessionEntry) -> bool {
    !entry.archived && !entry.key.starts_with("cron:")
}

fn session_list_label(entry: &SessionEntry) -> &str {
    entry.label.as_deref().unwrap_or(&entry.key)
}

fn format_channel_sessions_list(sessions: &[SessionEntry], current_session_key: &str) -> String {
    let mut lines = Vec::new();
    for (i, session) in sessions.iter().enumerate() {
        let marker = if session.key == current_session_key {
            " *"
        } else {
            ""
        };
        lines.push(format!(
            "{}. {} ({} msgs){}",
            i + 1,
            session_list_label(session),
            session.message_count,
            marker,
        ));
    }
    lines.push("\nUse /sessions N to switch.".to_string());
    lines.join("\n")
}

fn format_attachable_sessions_list(sessions: &[SessionEntry], current_session_key: &str) -> String {
    let mut lines = Vec::new();
    for (i, session) in sessions.iter().enumerate() {
        let label = session_list_label(session);
        let marker = if session.key == current_session_key {
            " *"
        } else {
            ""
        };
        let key_suffix = if label == session.key {
            String::new()
        } else {
            format!(" [{}]", session.key)
        };
        lines.push(format!(
            "{}. {}{} ({} msgs){}",
            i + 1,
            label,
            key_suffix,
            session.message_count,
            marker,
        ));
    }
    lines.push(
        "\nUse /attach N to move an existing session to this chat. This rebinds it from any previous channel chat."
            .to_string(),
    );
    lines.join("\n")
}

fn format_pending_approvals_list(requests: &[PendingApprovalView]) -> String {
    use crate::approval::{MAX_COMMAND_PREVIEW_LEN, truncate_command_preview};
    let mut lines = Vec::new();
    for (i, request) in requests.iter().enumerate() {
        let preview = truncate_command_preview(&request.command, MAX_COMMAND_PREVIEW_LEN);
        lines.push(format!("{}. `{}`", i + 1, preview));
    }
    lines.push("\nUse /approve N or /deny N.".to_string());
    lines.join("\n")
}

#[derive(Debug, Deserialize)]
struct ApprovalListResponse {
    #[serde(default)]
    requests: Vec<PendingApprovalView>,
}

#[derive(Debug, Default)]
struct ChannelSessionDefaults {
    model: Option<String>,
    agent_id: Option<String>,
}

fn config_string(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn override_map<'a>(
    config: &'a serde_json::Value,
    key: &str,
    target_id: &str,
) -> Option<&'a serde_json::Map<String, serde_json::Value>> {
    config
        .get(key)
        .and_then(serde_json::Value::as_object)
        .and_then(|overrides| overrides.get(target_id))
        .and_then(serde_json::Value::as_object)
}

async fn resolve_channel_session_defaults(
    state: &Arc<GatewayState>,
    reply_to: &ChannelReplyTarget,
    sender_id: Option<&str>,
) -> ChannelSessionDefaults {
    let Ok(status) = state.services.channel.status().await else {
        return ChannelSessionDefaults::default();
    };
    let Some(channel) = status
        .get("channels")
        .and_then(serde_json::Value::as_array)
        .and_then(|channels| {
            channels.iter().find(|channel| {
                channel
                    .get("account_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(reply_to.account_id.as_str())
                    && channel.get("type").and_then(serde_json::Value::as_str)
                        == Some(reply_to.channel_type.as_str())
            })
        })
    else {
        return ChannelSessionDefaults::default();
    };
    let Some(config) = channel.get("config") else {
        return ChannelSessionDefaults::default();
    };

    resolve_channel_session_defaults_from_config(config, &reply_to.chat_id, sender_id)
}

fn resolve_channel_session_defaults_from_config(
    config: &serde_json::Value,
    chat_id: &str,
    sender_id: Option<&str>,
) -> ChannelSessionDefaults {
    let user_override = override_map(
        config,
        "user_overrides",
        sender_id
            .filter(|sender_id| *sender_id != chat_id)
            .unwrap_or(chat_id),
    );
    let channel_override = override_map(config, "channel_overrides", chat_id);

    ChannelSessionDefaults {
        model: user_override
            .and_then(|override_value| config_string(override_value.get("model")))
            .or_else(|| {
                channel_override
                    .and_then(|override_value| config_string(override_value.get("model")))
            })
            .or_else(|| config_string(config.get("model"))),
        agent_id: user_override
            .and_then(|override_value| config_string(override_value.get("agent_id")))
            .or_else(|| {
                channel_override
                    .and_then(|override_value| config_string(override_value.get("agent_id")))
            })
            .or_else(|| config_string(config.get("agent_id"))),
    }
}

fn start_channel_typing_loop(
    state: &Arc<GatewayState>,
    reply_to: &ChannelReplyTarget,
) -> Option<tokio::sync::oneshot::Sender<()>> {
    let outbound = state.services.channel_outbound_arc()?;
    let (done_tx, mut done_rx) = tokio::sync::oneshot::channel::<()>();
    let account_id = reply_to.account_id.clone();
    let chat_id = reply_to.chat_id.clone();

    tokio::spawn(async move {
        loop {
            if let Err(e) = outbound.send_typing(&account_id, &chat_id).await {
                debug!(account_id, chat_id, "typing indicator failed: {e}");
            }
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(4)) => {},
                _ = &mut done_rx => break,
            }
        }
    });

    Some(done_tx)
}

async fn resolve_channel_agent_id(
    state: &Arc<GatewayState>,
    session_key: &str,
    requested_agent_id: Option<&str>,
) -> String {
    let fallback = if let Some(ref store) = state.services.agent_persona_store {
        store
            .default_id()
            .await
            .unwrap_or_else(|_| "main".to_string())
    } else {
        "main".to_string()
    };

    let Some(agent_id) = requested_agent_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return fallback;
    };

    if agent_id == "main" {
        return "main".to_string();
    }

    let Some(ref store) = state.services.agent_persona_store else {
        return agent_id.to_string();
    };

    match store.get(agent_id).await {
        Ok(Some(_)) => agent_id.to_string(),
        Ok(None) => {
            warn!(
                session = %session_key,
                agent_id,
                fallback = %fallback,
                "channel requested unknown agent, falling back to default"
            );
            fallback
        },
        Err(error) => {
            warn!(
                session = %session_key,
                agent_id,
                fallback = %fallback,
                %error,
                "failed to resolve channel agent, falling back to default"
            );
            fallback
        },
    }
}

mod commands;
mod control;
mod dispatch;
mod sink;
#[cfg(test)]
mod tests;

pub use sink::GatewayChannelEventSink;

#[async_trait]
impl ChannelEventSink for GatewayChannelEventSink {
    async fn emit(&self, event: ChannelEvent) {
        sink::emit(&self.state, event).await;
    }

    async fn request_sender_approval(
        &self,
        channel_type: &str,
        account_id: &str,
        identifier: &str,
    ) {
        commands::request_sender_approval(&self.state, channel_type, account_id, identifier).await;
    }

    async fn save_channel_voice(
        &self,
        audio_data: &[u8],
        filename: &str,
        reply_to: &ChannelReplyTarget,
    ) -> Option<String> {
        commands::save_channel_voice(&self.state, audio_data, filename, reply_to).await
    }

    async fn save_channel_attachment(
        &self,
        file_data: &[u8],
        filename: &str,
        reply_to: &ChannelReplyTarget,
    ) -> Option<SavedChannelFile> {
        commands::save_channel_attachment(&self.state, file_data, filename, reply_to).await
    }

    async fn transcribe_voice(&self, audio_data: &[u8], format: &str) -> ChannelResult<String> {
        commands::transcribe_voice(&self.state, audio_data, format).await
    }

    async fn voice_stt_available(&self) -> bool {
        commands::voice_stt_available(&self.state).await
    }

    async fn dispatch_interaction(
        &self,
        callback_data: &str,
        reply_to: ChannelReplyTarget,
    ) -> ChannelResult<String> {
        commands::dispatch_interaction(&self.state, callback_data, reply_to).await
    }

    async fn update_location(
        &self,
        reply_to: &ChannelReplyTarget,
        latitude: f64,
        longitude: f64,
    ) -> bool {
        commands::update_location(&self.state, reply_to, latitude, longitude).await
    }

    async fn resolve_pending_location(
        &self,
        reply_to: &ChannelReplyTarget,
        latitude: f64,
        longitude: f64,
    ) -> bool {
        commands::resolve_pending_location(&self.state, reply_to, latitude, longitude).await
    }

    async fn dispatch_to_chat_with_attachments(
        &self,
        text: &str,
        attachments: Vec<ChannelAttachment>,
        reply_to: ChannelReplyTarget,
        meta: ChannelMessageMeta,
    ) {
        commands::dispatch_to_chat_with_attachments(&self.state, text, attachments, reply_to, meta)
            .await;
    }

    async fn dispatch_to_chat(
        &self,
        text: &str,
        reply_to: ChannelReplyTarget,
        meta: ChannelMessageMeta,
    ) {
        dispatch::dispatch_to_chat(&self.state, text, reply_to, meta).await;
    }

    async fn request_disable_account(&self, channel_type: &str, account_id: &str, reason: &str) {
        control::request_disable_account(&self.state, channel_type, account_id, reason).await;
    }

    async fn dispatch_command(
        &self,
        command: &str,
        reply_to: ChannelReplyTarget,
        sender_id: Option<&str>,
    ) -> ChannelResult<String> {
        commands::dispatch_command(&self.state, command, reply_to, sender_id).await
    }
}
