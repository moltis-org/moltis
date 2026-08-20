use std::sync::Arc;

use {
    async_trait::async_trait,
    serde::Deserialize,
    tracing::{debug, error, warn},
};

use {
    moltis_channels::{
        ChannelAckOutcome, ChannelAttachment, ChannelEvent, ChannelEventSink, ChannelMessageMeta,
        ChannelReplyTarget, Error as ChannelError, Result as ChannelResult, SavedChannelFile,
        config_view::{UntrustedAudience, UntrustedTools},
    },
    moltis_sessions::metadata::{SessionEntry, SqliteSessionMetadata},
    moltis_tools::approval::PendingApprovalView,
};

use crate::{
    broadcast::{BroadcastOpts, broadcast},
    channel_reactions::ChannelReactionController,
    state::GatewayState,
};

pub use moltis_channels::operators::ChannelSenderRole;

/// Default (deterministic) session key for a channel chat.
///
/// For Telegram forum topics the thread ID is appended so each topic gets its
/// own session: `telegram:bot:chat:thread`.
fn default_channel_session_key(target: &ChannelReplyTarget) -> String {
    target.default_session_key()
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

/// Denial shown when a non-operator attempts a privileged action.
///
/// `action` names what was refused, e.g. `"/sh"` or `"Shell access"`.
///
/// The sender's own platform ID is echoed back because `operators` entries must
/// be exact platform IDs — an owner who has not set the list yet is locked out
/// of their own bot and would otherwise have to go and find their ID by hand.
/// It is the sender's own already-public identifier on that platform, so this
/// discloses nothing they could not read off their own profile.
pub(in crate::channel_events) fn operator_denied_message(
    action: &str,
    sender_id: Option<&str>,
) -> String {
    let mut message = format!(
        "{action} is restricted to this bot's operators in direct chats. \
         Shared chats and chats that cannot be verified as direct are denied.\n\n\
         Open a direct chat first. If you own this moltis instance and are still denied, add yourself under \
         Settings → Channels → (your account) → Edit → Operators in the web UI, \
         or set `operators` for the account in moltis.toml. Entries must be exact \
         platform sender IDs."
    );
    if let Some(sender_id) = sender_id.map(str::trim).filter(|id| !id.is_empty()) {
        message.push_str(&format!("\n\nYour sender ID here is: {sender_id}"));
    }
    message
}

/// Resolve a channel sender's privilege level for an account.
///
/// Privileged actions (`/sh`, shell command mode, `/approve`, `/update`, and
/// host-reaching tools) require the sender to be an **operator**. Passing the
/// channel access gate is not enough: in a guild or group chat every member
/// clears that gate, so privilege is decided by the account's explicit
/// `operators` list.
///
/// Fail-closed at every step — an unknown account, a missing registry, or an
/// unidentified sender all resolve to [`ChannelSenderRole::Guest`].
async fn resolve_sender_role(
    state: &Arc<GatewayState>,
    account_id: &str,
    sender_id: Option<&str>,
) -> ChannelSenderRole {
    let Some(ref registry) = state.services.channel_registry else {
        return ChannelSenderRole::Guest;
    };
    let Some(config) = registry.account_config(account_id).await else {
        return ChannelSenderRole::Guest;
    };
    moltis_channels::operators::resolve_sender_role(sender_id, config.operators())
}

/// Read the account's untrusted-turn ceiling. A missing registry or an unknown
/// account falls back to the defaults, which are the unconfigured behaviour.
async fn resolve_untrusted_ceiling(
    state: &Arc<GatewayState>,
    account_id: &str,
) -> (UntrustedAudience, UntrustedTools) {
    let Some(ref registry) = state.services.channel_registry else {
        return Default::default();
    };
    let Some(config) = registry.account_config(account_id).await else {
        return Default::default();
    };
    (config.untrusted_audience(), config.untrusted_tools())
}

/// Apply the fail-closed context used for every untrusted channel turn.
///
/// The audience ceiling excludes trusted tools, while the deny-all name policy
/// also removes explicitly public tools. Configured policies may narrow this
/// context further but cannot widen it.
fn apply_untrusted_channel_context(params: &mut serde_json::Value) {
    apply_untrusted_channel_context_with(
        params,
        UntrustedAudience::default(),
        UntrustedTools::default(),
    );
}

/// Apply the untrusted channel context at the account's configured ceiling. The
/// defaults reproduce [`apply_untrusted_channel_context`] exactly.
///
/// `_private_context` is deliberately not configurable: owner memory, profile
/// and project context describe the owner rather than the conversation, so a
/// room with other people in it never receives them.
fn apply_untrusted_channel_context_with(
    params: &mut serde_json::Value,
    audience: UntrustedAudience,
    tools: UntrustedTools,
) {
    if audience == UntrustedAudience::Public {
        params["_tool_audience"] = serde_json::json!("public");
    }
    if tools == UntrustedTools::DenyAll {
        params["_tool_policy"] = serde_json::json!({ "deny": ["*"] });
    }
    params["_private_context"] = serde_json::json!(false);
}

/// Unknown chat kinds are treated as shared. Only channel types that can prove
/// a one-to-one conversation may expose private prompt context.
fn is_shared_channel_target(reply_to: &ChannelReplyTarget) -> bool {
    reply_to.channel_type.is_shared_chat(&reply_to.chat_id)
}

fn is_trusted_channel_turn(role: ChannelSenderRole, reply_to: &ChannelReplyTarget) -> bool {
    role.is_operator() && !is_shared_channel_target(reply_to)
}

fn is_channel_command_authorized(
    privilege: moltis_channels::commands::CommandPrivilege,
    role: ChannelSenderRole,
    reply_to: &ChannelReplyTarget,
) -> bool {
    match privilege {
        moltis_channels::commands::CommandPrivilege::Public => true,
        moltis_channels::commands::CommandPrivilege::OperatorDirect => {
            is_trusted_channel_turn(role, reply_to)
        },
    }
}

/// Whether `sender_id` may run privileged actions from this conversation.
async fn is_sender_authorized_for_target(
    state: &Arc<GatewayState>,
    reply_to: &ChannelReplyTarget,
    sender_id: Option<&str>,
) -> bool {
    let role = resolve_sender_role(state, &reply_to.account_id, sender_id).await;
    is_trusted_channel_turn(role, reply_to)
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

/// Create and register a per-turn acknowledgment reaction controller, keyed by
/// the inbound message's own ack key. The controller immediately adds 👀 to that
/// exact message and is later driven through phase emojis and finalized by
/// whichever agent run claims it.
///
/// No-op unless the channel populated `ack_message_id` (bot directly addressed
/// and reactions enabled) and an outbound implementation is available. Today
/// that is Slack; other channels leave `ack_message_id` unset and get no
/// reactions.
///
/// Returns the ack key so the caller can carry it into `chat.send`.
async fn register_channel_reaction_controller(
    state: &Arc<GatewayState>,
    reply_to: &ChannelReplyTarget,
) -> Option<String> {
    let message_id = reply_to.ack_message_id.clone()?;
    let outbound = state.services.channel_outbound_arc()?;
    let key =
        crate::channel_reactions::ack_key(&reply_to.account_id, &reply_to.chat_id, &message_id);
    let controller = ChannelReactionController::start(
        outbound,
        reply_to.account_id.clone(),
        reply_to.chat_id.clone(),
        message_id,
    );
    // Park it against this message's own identity. A run claims it once the
    // queue decision is known, so a message that queues behind an active run
    // keeps its own 👀 instead of hijacking the running turn's reactions.
    state
        .channel_reaction_controllers
        .register_pending(key.clone(), controller)
        .await;
    Some(key)
}

/// Finalize the acknowledgment reaction only when `chat.send` returned before
/// the run executed — an error, or an `Ok` payload carrying a terminal state
/// (`rejected`/`error`/`blocked`/`aborted`). Normal runs (which return
/// `{ok, runId}` with no terminal state) finalize from the run's completion, so
/// this leaves their controller in place.
async fn finalize_reaction_on_early_return(
    state: &Arc<GatewayState>,
    ack_key: Option<&String>,
    send_result: &Result<serde_json::Value, moltis_service_traits::ServiceError>,
) {
    let Some(ack_key) = ack_key else {
        return;
    };
    let outcome = match send_result {
        Err(_) => Some(ChannelAckOutcome::Failure),
        Ok(payload) => {
            // A queued message keeps its acknowledgment: it has not run yet and
            // will claim it on replay.
            if payload.get("queued").and_then(serde_json::Value::as_bool) == Some(true) {
                None
            } else if payload.get("rejected").and_then(serde_json::Value::as_bool) == Some(true) {
                // Blocked by a MessageReceived hook — no run will ever start.
                Some(ChannelAckOutcome::Failure)
            } else {
                match payload.get("state").and_then(|v| v.as_str()) {
                    Some("rejected" | "error" | "blocked") => Some(ChannelAckOutcome::Failure),
                    Some("aborted") => Some(ChannelAckOutcome::Cancelled),
                    _ => None,
                }
            }
        },
    };
    let Some(outcome) = outcome else {
        return;
    };
    state
        .channel_reaction_controllers
        .finalize_keys(std::slice::from_ref(ack_key), outcome)
        .await;
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
        sender_id: Option<&str>,
    ) -> ChannelResult<String> {
        commands::dispatch_interaction(&self.state, callback_data, reply_to, sender_id).await
    }

    async fn update_location(
        &self,
        reply_to: &ChannelReplyTarget,
        sender_id: Option<&str>,
        latitude: f64,
        longitude: f64,
    ) -> bool {
        commands::update_location(&self.state, reply_to, sender_id, latitude, longitude).await
    }

    async fn resolve_pending_location(
        &self,
        reply_to: &ChannelReplyTarget,
        sender_id: Option<&str>,
        latitude: f64,
        longitude: f64,
    ) -> bool {
        commands::resolve_pending_location(&self.state, reply_to, sender_id, latitude, longitude)
            .await
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
