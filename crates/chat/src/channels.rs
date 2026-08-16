//! Channel delivery, tool status, screenshots, documents, and location.
//!
//! TTS synthesis lives in [`tts`]; everything here is about getting a finished
//! reply out to the channel targets a session accumulated during its run.
//! Web push notifications live in [`crate::channel_push`].

use std::{collections::HashSet, sync::Arc, time::Duration};

use {
    serde_json::Value,
    tokio::task::JoinSet,
    tracing::{debug, info, warn},
};

use moltis_sessions::store::SessionStore;

use crate::{
    agent_loop::ChannelReplyTargetKey, channel_acks::note_delivery_failed,
    channels::tts::build_tts_payload, compaction_run, runtime::ChatRuntime, types::*,
};

pub(crate) use crate::channels::tts::generate_tts_audio;

/// Cap on the final, irreversible channel I/O (last reply or terminal error).
///
/// The run has already committed its terminal by the time these are sent, so a
/// channel API that never answers must not hold the run's task — and its
/// session permit — open indefinitely. Overrunning it is reported as a delivery
/// failure so the acknowledgment reaction shows ❌ rather than ✅.
const FINAL_CHANNEL_IO_TIMEOUT: Duration = Duration::from_secs(30);

/// Drain any pending channel reply targets for a session and send the
/// response text back to each originating channel via outbound.
///
/// Each delivery runs in its own spawned task so slow network calls don't block
/// each other or the chat pipeline. The whole fan-out is bounded by
/// [`FINAL_CHANNEL_IO_TIMEOUT`]; overrunning it (or any individual send
/// failing) marks the turn's acknowledgment as a delivery failure, since the
/// user never received the reply.
pub(crate) async fn deliver_channel_replies(
    state: &Arc<dyn ChatRuntime>,
    activity_id: &str,
    session_key: &str,
    text: &str,
    desired_reply_medium: ReplyMedium,
    streamed_target_keys: &HashSet<ChannelReplyTargetKey>,
) {
    if tokio::time::timeout(
        FINAL_CHANNEL_IO_TIMEOUT,
        deliver_channel_replies_inner(
            state,
            activity_id,
            session_key,
            text,
            desired_reply_medium,
            streamed_target_keys,
        ),
    )
    .await
    .is_err()
    {
        warn!(
            activity_id,
            session_key, "timed out delivering final channel reply"
        );
        note_delivery_failed(state, activity_id).await;
    }
}

async fn deliver_channel_replies_inner(
    state: &Arc<dyn ChatRuntime>,
    activity_id: &str,
    session_key: &str,
    text: &str,
    desired_reply_medium: ReplyMedium,
    streamed_target_keys: &HashSet<ChannelReplyTargetKey>,
) {
    let drained_targets = state.drain_channel_replies(session_key).await;
    let mut targets = Vec::with_capacity(drained_targets.len());
    let mut streamed_targets = Vec::new();
    // When the reply medium is voice we must still deliver TTS audio even if
    // the text was already streamed — skip the stream dedupe entirely.
    if desired_reply_medium != ReplyMedium::Voice && !streamed_target_keys.is_empty() {
        for target in drained_targets {
            let key = ChannelReplyTargetKey::from(&target);
            if streamed_target_keys.contains(&key) {
                streamed_targets.push(target);
            } else {
                targets.push(target);
            }
        }
    } else {
        targets = drained_targets;
    }
    let is_channel_session = session_key.starts_with("telegram:")
        || session_key.starts_with("msteams:")
        || session_key.starts_with("discord:");
    if targets.is_empty() && streamed_targets.is_empty() {
        let _ = state.drain_channel_status_log(session_key).await;
        if is_channel_session {
            info!(
                session_key,
                text_len = text.len(),
                streamed_count = streamed_target_keys.len(),
                "channel reply delivery skipped: no pending targets after stream dedupe"
            );
        }
        return;
    }
    if text.is_empty() {
        let _ = state.drain_channel_status_log(session_key).await;
        if is_channel_session {
            info!(
                session_key,
                target_count = targets.len() + streamed_targets.len(),
                "channel reply delivery skipped: empty response text"
            );
        }
        return;
    }
    if is_channel_session {
        info!(
            session_key,
            target_count = targets.len(),
            text_len = text.len(),
            reply_medium = ?desired_reply_medium,
            "channel reply delivery starting"
        );
    }
    let outbound = match state.channel_outbound() {
        Some(o) => o,
        None => {
            if is_channel_session {
                info!(
                    session_key,
                    target_count = targets.len(),
                    "channel reply delivery skipped: outbound unavailable"
                );
            }
            note_delivery_failed(state, activity_id).await;
            return;
        },
    };
    // Drain buffered status log entries to build a logbook suffix.
    let status_log = state.drain_channel_status_log(session_key).await;
    let logbook_html = format_logbook_html(&status_log);
    if !streamed_targets.is_empty() && !logbook_html.is_empty() {
        send_channel_logbook_follow_up_to_targets(
            Arc::clone(&outbound),
            streamed_targets,
            &logbook_html,
            state.feedback(),
            session_key,
            activity_id,
        )
        .await;
    }
    if targets.is_empty() {
        if is_channel_session {
            info!(
                session_key,
                text_len = text.len(),
                streamed_count = streamed_target_keys.len(),
                "channel reply delivery completed via stream-only targets"
            );
        }
        return;
    }
    crate::channel_reply_delivery::deliver_channel_replies_to_targets(
        outbound,
        targets,
        session_key,
        activity_id,
        text,
        Arc::clone(state),
        desired_reply_medium,
        status_log,
        streamed_target_keys,
    )
    .await;
}

/// Format buffered status log entries into a Telegram expandable blockquote HTML.
/// Returns an empty string if there are no entries.
pub(crate) fn format_logbook_html(entries: &[String]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let mut html = String::from("<blockquote expandable>\n\u{1f4cb} <b>Activity log</b>\n");
    for entry in entries {
        // Escape HTML entities in the entry text.
        let escaped = entry
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        html.push_str(&format!("\u{2022} {escaped}\n"));
    }
    html.push_str("</blockquote>");
    html
}

async fn send_channel_logbook_follow_up_to_targets(
    outbound: Arc<dyn moltis_channels::plugin::ChannelOutbound>,
    targets: Vec<moltis_channels::ChannelReplyTarget>,
    logbook_html: &str,
    feedback: Option<Arc<moltis_channels::FeedbackService>>,
    session_key: &str,
    trace_correlation_key: &str,
) {
    if targets.is_empty() || logbook_html.is_empty() {
        return;
    }

    let html = logbook_html.to_string();
    let session_key = session_key.to_string();
    let trace_correlation_key = trace_correlation_key.to_string();
    let mut tasks = JoinSet::new();
    for target in targets {
        let outbound = Arc::clone(&outbound);
        let html = html.clone();
        let feedback = feedback.clone();
        let session_key = session_key.clone();
        let trace_correlation_key = trace_correlation_key.clone();
        let to = target.outbound_to().into_owned();
        tasks.spawn(async move {
            match outbound
                .send_html_reporting_ids(&target.account_id, &to, &html, None)
                .await
            {
                Ok(message_ids) => {
                    crate::channel_feedback::record_reply_trace(
                        feedback.as_deref(),
                        &target,
                        &message_ids,
                        &session_key,
                        &trace_correlation_key,
                    )
                    .await;
                },
                Err(e) => {
                    warn!(
                        account_id = target.account_id,
                        chat_id = target.chat_id,
                        thread_id = target.thread_id.as_deref().unwrap_or("-"),
                        "failed to send logbook follow-up: {e}"
                    );
                },
            }
        });
    }

    while let Some(result) = tasks.join_next().await {
        if let Err(e) = result {
            warn!(error = %e, "channel logbook follow-up task join failed");
        }
    }
}

fn format_channel_retry_message(error_obj: &Value, retry_after: Duration) -> String {
    let retry_secs = ((retry_after.as_millis() as u64).saturating_add(999) / 1_000).max(1);
    if error_obj.get("type").and_then(|v| v.as_str()) == Some("rate_limit_exceeded") {
        format!("⏳ Provider rate limited. Retrying in {retry_secs}s.")
    } else {
        format!("⏳ Temporary provider issue. Retrying in {retry_secs}s.")
    }
}

fn format_channel_error_message(error_obj: &Value) -> String {
    let title = error_obj
        .get("title")
        .and_then(|v| v.as_str())
        .or_else(|| match error_obj.get("type").and_then(|v| v.as_str()) {
            Some("message_rejected") => Some("Message rejected"),
            _ => None,
        })
        .unwrap_or("Request failed");
    let detail = error_obj
        .get("detail")
        .and_then(|v| v.as_str())
        .or_else(|| error_obj.get("message").and_then(|v| v.as_str()))
        .unwrap_or("Please try again.");
    format!("⚠️ {title}: {detail}")
}

/// Format a user-facing notice announcing that a session was compacted.
///
/// Shown verbatim to channel users (Telegram, Discord, WhatsApp, etc.) and
/// kept short so small mobile clients don't wrap the whole thing.
///
/// When `include_settings_hint` is false, the "Change chat.compaction.mode…"
/// footer is omitted so users who have set
/// `chat.compaction.show_settings_hint = false` don't see the repetitive
/// hint on every compaction. Mode + token lines are always included.
/// The LLM retry path never sees this text regardless.
#[allow(dead_code)]
fn format_channel_compaction_notice(
    outcome: &compaction_run::CompactionOutcome,
    include_settings_hint: bool,
) -> String {
    let mode_label = match outcome.effective_mode {
        moltis_config::CompactionMode::Deterministic => "Deterministic",
        moltis_config::CompactionMode::RecencyPreserving => "Recency preserving",
        moltis_config::CompactionMode::Structured => "Structured",
        moltis_config::CompactionMode::LlmReplace => "LLM replace",
    };
    let total = outcome.total_tokens();
    let token_line = if total == 0 {
        // Any strategy that made no LLM calls ends up here: Deterministic,
        // RecencyPreserving, or a Structured run that fell back to
        // recency_preserving before the LLM call landed. Report the
        // actual effective mode so users don't see "deterministic
        // strategy" when they picked recency_preserving.
        format!(
            "No LLM tokens used ({} strategy)",
            mode_label.to_lowercase()
        )
    } else {
        format!(
            "Used {total} tokens ({input} in + {output} out)",
            total = total,
            input = outcome.input_tokens,
            output = outcome.output_tokens,
        )
    };
    let body = format!(
        "🧹 Conversation compacted\n\
         Mode: {mode_label}\n\
         {token_line}",
    );
    if include_settings_hint {
        format!("{body}\n{hint}", hint = compaction_run::SETTINGS_HINT)
    } else {
        body
    }
}

/// Send a silent "session compacted" notice to pending channel targets
/// without draining them.
///
/// Mirrors [`send_retry_status_to_channels`]: the targets are *peeked*,
/// not drained, so the in-flight agent run can still deliver its final
/// reply to them afterward. Uses `send_text_silent` so the channel
/// integration doesn't count it toward user-visible interactive replies
/// (no TTS, no delivery receipts beyond the channel's own).
#[allow(dead_code)]
pub(crate) async fn notify_channels_of_compaction(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    outcome: &compaction_run::CompactionOutcome,
    include_settings_hint: bool,
) {
    let targets = state.peek_channel_replies(session_key).await;
    if targets.is_empty() {
        return;
    }

    let Some(outbound) = state.channel_outbound() else {
        return;
    };

    let message = format_channel_compaction_notice(outcome, include_settings_hint);
    let mut tasks = JoinSet::new();
    for target in targets {
        let outbound = Arc::clone(&outbound);
        let message = message.clone();
        let to = target.outbound_to().into_owned();
        tasks.spawn(async move {
            let reply_to = target.message_id.as_deref();
            if let Err(e) = outbound
                .send_text_silent(&target.account_id, &to, &message, reply_to)
                .await
            {
                warn!(
                    account_id = target.account_id,
                    chat_id = target.chat_id,
                    thread_id = target.thread_id.as_deref().unwrap_or("-"),
                    "failed to send compaction notice to channel: {e}"
                );
            }
        });
    }

    while let Some(result) = tasks.join_next().await {
        if let Err(e) = result {
            warn!(error = %e, "channel compaction notice task join failed");
        }
    }
}

/// Send a short retry status update to pending channel targets without draining
/// them. The final reply (or terminal error) will still use the same targets.
pub(crate) async fn send_retry_status_to_channels(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    error_obj: &Value,
    retry_after: Duration,
) {
    let targets = state.peek_channel_replies(session_key).await;
    if targets.is_empty() {
        return;
    }

    let outbound = match state.channel_outbound() {
        Some(o) => o,
        None => return,
    };

    let message = format_channel_retry_message(error_obj, retry_after);
    let mut tasks = JoinSet::new();
    for target in targets {
        let outbound = Arc::clone(&outbound);
        let message = message.clone();
        let to = target.outbound_to().into_owned();
        tasks.spawn(async move {
            let reply_to = target.message_id.as_deref();
            if let Err(e) = outbound
                .send_text_silent(&target.account_id, &to, &message, reply_to)
                .await
            {
                warn!(
                    account_id = target.account_id,
                    chat_id = target.chat_id,
                    thread_id = target.thread_id.as_deref().unwrap_or("-"),
                    "failed to send retry status to channel: {e}"
                );
            }
        });
    }

    while let Some(result) = tasks.join_next().await {
        if let Err(e) = result {
            warn!(error = %e, "channel retry status task join failed");
        }
    }
}

/// Drain pending channel targets for a session and send a terminal error message.
pub(crate) async fn deliver_channel_error(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    error_obj: &Value,
) {
    if tokio::time::timeout(
        FINAL_CHANNEL_IO_TIMEOUT,
        deliver_channel_error_inner(state, session_key, error_obj),
    )
    .await
    .is_err()
    {
        warn!(session_key, "timed out delivering terminal channel error");
    }
}

async fn deliver_channel_error_inner(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    error_obj: &Value,
) {
    let targets = state.drain_channel_replies(session_key).await;
    let status_log = state.drain_channel_status_log(session_key).await;
    if targets.is_empty() {
        return;
    }

    let outbound = match state.channel_outbound() {
        Some(o) => o,
        None => return,
    };

    let error_text = format_channel_error_message(error_obj);
    let logbook_html = format_logbook_html(&status_log);
    let mut tasks = JoinSet::new();
    for target in targets {
        let outbound = Arc::clone(&outbound);
        let error_text = error_text.clone();
        let logbook_html = logbook_html.clone();
        let to = target.outbound_to().into_owned();
        tasks.spawn(async move {
            let reply_to = target.message_id.as_deref();
            let send_result = if logbook_html.is_empty() {
                outbound
                    .send_text(&target.account_id, &to, &error_text, reply_to)
                    .await
            } else {
                outbound
                    .send_text_with_suffix(
                        &target.account_id,
                        &to,
                        &error_text,
                        &logbook_html,
                        reply_to,
                    )
                    .await
            };
            if let Err(e) = send_result {
                warn!(
                    account_id = target.account_id,
                    chat_id = target.chat_id,
                    thread_id = target.thread_id.as_deref().unwrap_or("-"),
                    "failed to send channel error reply: {e}"
                );
            }
        });
    }

    while let Some(result) = tasks.join_next().await {
        if let Err(e) = result {
            warn!(error = %e, "channel error task join failed");
        }
    }
}

#[allow(dead_code)]
async fn deliver_channel_replies_to_targets(
    outbound: Arc<dyn moltis_channels::plugin::ChannelOutbound>,
    targets: Vec<moltis_channels::ChannelReplyTarget>,
    activity_id: &str,
    session_key: &str,
    text: &str,
    state: Arc<dyn ChatRuntime>,
    desired_reply_medium: ReplyMedium,
    status_log: Vec<String>,
    streamed_target_keys: &HashSet<ChannelReplyTargetKey>,
) {
    let session_key = session_key.to_string();
    let activity_id = activity_id.to_string();
    let text = text.to_string();
    let logbook_html = format_logbook_html(&status_log);
    let mut tasks = JoinSet::new();
    for target in targets {
        let outbound = Arc::clone(&outbound);
        let state = Arc::clone(&state);
        let session_key = session_key.clone();
        let activity_id = activity_id.clone();
        let text = text.clone();
        let logbook_html = logbook_html.clone();
        let text_already_streamed =
            streamed_target_keys.contains(&ChannelReplyTargetKey::from(&target));
        let to = target.outbound_to().into_owned();
        tasks.spawn(async move {
            deliver_reply_to_target(
                outbound.as_ref(),
                &state,
                &activity_id,
                &session_key,
                &target,
                &to,
                &text,
                &logbook_html,
                desired_reply_medium,
                text_already_streamed,
            )
            .await;
        });
    }

    while let Some(result) = tasks.join_next().await {
        if let Err(e) = result {
            warn!(error = %e, "channel reply task join failed");
            // A panicked task may have sent nothing — do not claim success.
            note_delivery_failed(&state, &activity_id).await;
        }
    }
}

/// Deliver the final reply to one channel target.
///
/// A voice reply becomes a TTS media message; if TTS is unavailable, or the
/// medium is text, the reply falls back to plain text. Telegram is special-cased
/// because it can carry the transcript as a caption on the voice note, which
/// saves a second message when the transcript is short enough.
#[allow(clippy::too_many_arguments)]
#[allow(dead_code)]
async fn deliver_reply_to_target(
    outbound: &dyn moltis_channels::ChannelOutbound,
    state: &Arc<dyn ChatRuntime>,
    activity_id: &str,
    session_key: &str,
    target: &moltis_channels::ChannelReplyTarget,
    to: &str,
    text: &str,
    logbook_html: &str,
    desired_reply_medium: ReplyMedium,
    text_already_streamed: bool,
) {
    let tts_payload = match desired_reply_medium {
        ReplyMedium::Voice => build_tts_payload(state, session_key, target, text).await,
        ReplyMedium::Text => None,
    };
    let reply_to = target.message_id.as_deref();

    // TTS disabled or failed — deliver the text instead.
    let Some(mut payload) = tts_payload else {
        if !deliver_text_fallback(
            outbound,
            target,
            to,
            text,
            logbook_html,
            reply_to,
            text_already_streamed,
        )
        .await
        {
            note_delivery_failed(state, activity_id).await;
        }
        return;
    };

    if target.channel_type != moltis_channels::ChannelType::Telegram {
        send_voice_reply(outbound, state, activity_id, target, to, &payload, reply_to).await;
        return;
    }

    // Telegram can attach the transcript as a caption, so take it off the
    // payload and decide below whether it goes on the voice note or follows it.
    let transcript = std::mem::take(&mut payload.text);

    // Text was already delivered via edit-in-place streaming — skip the
    // caption/follow-up and only send the TTS voice audio.
    if text_already_streamed {
        send_voice_reply(outbound, state, activity_id, target, to, &payload, reply_to).await;
        send_logbook_follow_up(outbound, target, to, logbook_html).await;
        return;
    }

    // Check if the transcript fits as a Telegram caption (when the feature is
    // enabled). With the telegram feature off this is always false, so the
    // voice note is followed by the text as a separate message.
    #[cfg(feature = "telegram")]
    let fits_in_caption = transcript.len() <= moltis_telegram::markdown::TELEGRAM_CAPTION_LIMIT;
    #[cfg(not(feature = "telegram"))]
    let fits_in_caption = false;

    // Short transcript fits as a caption on the voice message.
    if fits_in_caption {
        payload.text = transcript;
        send_voice_reply(outbound, state, activity_id, target, to, &payload, reply_to).await;
        send_logbook_follow_up(outbound, target, to, logbook_html).await;
        return;
    }

    // Transcript too long for a caption — send voice without a caption, then
    // the full text as a follow-up.
    send_voice_reply(outbound, state, activity_id, target, to, &payload, reply_to).await;
    let text_result = if logbook_html.is_empty() {
        outbound
            .send_text(&target.account_id, to, &transcript, None)
            .await
    } else {
        outbound
            .send_text_with_suffix(&target.account_id, to, &transcript, logbook_html, None)
            .await
    };
    if let Err(e) = text_result {
        warn!(
            account_id = target.account_id,
            chat_id = target.chat_id,
            thread_id = target.thread_id.as_deref().unwrap_or("-"),
            "failed to send transcript follow-up: {e}"
        );
        note_delivery_failed(state, activity_id).await;
    }
}

/// Send a TTS voice message, reporting a delivery failure if it does not land.
#[allow(dead_code)]
async fn send_voice_reply(
    outbound: &dyn moltis_channels::ChannelOutbound,
    state: &Arc<dyn ChatRuntime>,
    activity_id: &str,
    target: &moltis_channels::ChannelReplyTarget,
    to: &str,
    payload: &moltis_common::types::ReplyPayload,
    reply_to: Option<&str>,
) {
    if let Err(e) = outbound
        .send_media(&target.account_id, to, payload, reply_to)
        .await
    {
        warn!(
            account_id = target.account_id,
            chat_id = target.chat_id,
            thread_id = target.thread_id.as_deref().unwrap_or("-"),
            "failed to send channel voice reply: {e}"
        );
        note_delivery_failed(state, activity_id).await;
    }
}

/// Send the buffered status logbook as a separate follow-up message.
///
/// Best-effort: the reply itself already reached the user, so a lost logbook is
/// not a delivery failure and must not turn the acknowledgment into ❌.
#[allow(dead_code)]
async fn send_logbook_follow_up(
    outbound: &dyn moltis_channels::ChannelOutbound,
    target: &moltis_channels::ChannelReplyTarget,
    to: &str,
    logbook_html: &str,
) {
    if logbook_html.is_empty() {
        return;
    }
    if let Err(e) = outbound
        .send_html(&target.account_id, to, logbook_html, None)
        .await
    {
        warn!(
            account_id = target.account_id,
            chat_id = target.chat_id,
            thread_id = target.thread_id.as_deref().unwrap_or("-"),
            "failed to send logbook follow-up: {e}"
        );
    }
}

/// Deliver the reply as plain text, or just the logbook when the text was
/// already streamed in place. Returns whether the user received the reply.
#[allow(dead_code)]
async fn deliver_text_fallback(
    outbound: &dyn moltis_channels::ChannelOutbound,
    target: &moltis_channels::ChannelReplyTarget,
    to: &str,
    text: &str,
    logbook_html: &str,
    reply_to: Option<&str>,
    text_already_streamed: bool,
) -> bool {
    if text_already_streamed {
        send_logbook_follow_up(outbound, target, to, logbook_html).await;
        return true;
    }

    let result = if logbook_html.is_empty() {
        outbound
            .send_text(&target.account_id, to, text, reply_to)
            .await
    } else {
        outbound
            .send_text_with_suffix(&target.account_id, to, text, logbook_html, reply_to)
            .await
    };
    if let Err(e) = result {
        warn!(
            account_id = target.account_id,
            chat_id = target.chat_id,
            thread_id = target.thread_id.as_deref().unwrap_or("-"),
            "failed to send channel reply: {e}"
        );
        return false;
    }
    true
}

/// Buffer a tool execution status into the channel status log for a session.
/// The buffered entries are appended as a collapsible logbook when the final
/// response is delivered, instead of being sent as separate messages.
pub(crate) async fn send_tool_status_to_channels(
    state: &Arc<dyn ChatRuntime>,
    activity_id: &str,
    session_key: &str,
    tool_name: &str,
    arguments: &Value,
) {
    let targets = state.peek_channel_replies(session_key).await;
    if targets.is_empty() {
        return;
    }

    // Drive the acknowledgment reaction into its "tool" phase (🛠️/🌐/💻/…).
    state
        .note_channel_activity(
            activity_id,
            moltis_channels::ChannelActivity::Tool(tool_name.to_string()),
        )
        .await;

    // Buffer the status message for the logbook
    let message = format_tool_status_message(tool_name, arguments);
    state.push_channel_status_log(session_key, message).await;
}

/// Buffer a tool error result into the channel status log for a session.
/// Called from `ToolCallEnd` for failed tool calls only — success is implicit
/// and does not need a separate log entry.
pub(crate) async fn send_tool_result_to_channels(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    tool_name: &str,
    success: bool,
    error: &Option<String>,
    result: &Option<Value>,
) {
    if success {
        return;
    }
    let targets = state.peek_channel_replies(session_key).await;
    if targets.is_empty() {
        return;
    }

    let message = format_tool_result_message(tool_name, error, result);
    state.push_channel_status_log(session_key, message).await;
}

/// Format a human-readable error summary for a failed tool call.
fn format_tool_result_message(
    tool_name: &str,
    error: &Option<String>,
    result: &Option<Value>,
) -> String {
    let detail = match tool_name {
        "exec" => {
            let exit_code = result
                .as_ref()
                .and_then(|r| r.get("exitCode"))
                .and_then(|v| v.as_i64());
            let stderr = result
                .as_ref()
                .and_then(|r| r.get("stderr"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let first_line = stderr.lines().next().unwrap_or_default();
            let truncated = truncate_at_char_boundary(first_line, 120);
            match exit_code {
                Some(code) => {
                    if truncated.is_empty() {
                        format!("exit {code}")
                    } else {
                        format!("exit {code} — {truncated}")
                    }
                },
                None => {
                    if truncated.is_empty() {
                        error
                            .as_deref()
                            .map(|e| truncate_at_char_boundary(e, 120).to_string())
                            .unwrap_or_else(|| "failed".to_string())
                    } else {
                        truncated.to_string()
                    }
                },
            }
        },
        _ => {
            // Browser, web_fetch, web_search, and other tools: use error string.
            error
                .as_deref()
                .map(|e| {
                    let first_line = e.lines().next().unwrap_or_default();
                    truncate_at_char_boundary(first_line, 120).to_string()
                })
                .unwrap_or_else(|| "failed".to_string())
        },
    };
    format!("  ❌ {detail}")
}

/// Format a human-readable tool execution message.
fn format_tool_status_message(tool_name: &str, arguments: &Value) -> String {
    match tool_name {
        "browser" => {
            let action = arguments
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let url = arguments.get("url").and_then(|v| v.as_str());
            let ref_ = arguments.get("ref_").and_then(|v| v.as_u64());

            match action {
                "navigate" => {
                    if let Some(u) = url {
                        format!("🌐 Navigating to {}", truncate_url(u))
                    } else {
                        "🌐 Navigating...".to_string()
                    }
                },
                "screenshot" => "📸 Taking screenshot...".to_string(),
                "snapshot" => "📋 Getting page snapshot...".to_string(),
                "click" => {
                    if let Some(r) = ref_ {
                        format!("👆 Clicking element #{}", r)
                    } else {
                        "👆 Clicking...".to_string()
                    }
                },
                "type" => "⌨️ Typing...".to_string(),
                "scroll" => "📜 Scrolling...".to_string(),
                "evaluate" => "⚡ Running JavaScript...".to_string(),
                "wait" => "⏳ Waiting for element...".to_string(),
                "close" => "🚪 Closing browser...".to_string(),
                _ => format!("🌐 Browser: {}", action),
            }
        },
        "exec" => {
            let command = arguments.get("command").and_then(|v| v.as_str());
            if let Some(cmd) = command {
                // Show first ~50 chars of command
                let display_cmd = if cmd.len() > 50 {
                    format!("{}...", truncate_at_char_boundary(cmd, 50))
                } else {
                    cmd.to_string()
                };
                format!("💻 Running: `{}`", display_cmd)
            } else {
                "💻 Executing command...".to_string()
            }
        },
        "web_fetch" => {
            let url = arguments.get("url").and_then(|v| v.as_str());
            if let Some(u) = url {
                format!("🔗 Fetching {}", truncate_url(u))
            } else {
                "🔗 Fetching URL...".to_string()
            }
        },
        "web_search" => {
            let query = arguments.get("query").and_then(|v| v.as_str());
            if let Some(q) = query {
                let display_q = if q.len() > 40 {
                    format!("{}...", truncate_at_char_boundary(q, 40))
                } else {
                    q.to_string()
                };
                format!("🔍 Searching: {}", display_q)
            } else {
                "🔍 Searching...".to_string()
            }
        },
        "calc" => {
            let expr = arguments
                .get("expression")
                .or_else(|| arguments.get("expr"))
                .and_then(|v| v.as_str());
            if let Some(expression) = expr {
                let display = if expression.len() > 50 {
                    format!("{}...", truncate_at_char_boundary(expression, 50))
                } else {
                    expression.to_string()
                };
                format!("🧮 Calculating: {}", display)
            } else {
                "🧮 Calculating...".to_string()
            }
        },
        "memory_search" => "🧠 Searching memory...".to_string(),
        "memory_delete" => "🧠 Removing memory snippet...".to_string(),
        "memory_forget" => "🧠 Forgetting memory...".to_string(),
        "memory_store" => "🧠 Storing to memory...".to_string(),
        _ => format!("🔧 {}", tool_name),
    }
}

/// Truncate a URL for display (show domain + short path).
fn truncate_url(url: &str) -> String {
    // Try to extract domain from URL
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);

    // Take first 50 chars max
    if without_scheme.len() > 50 {
        format!("{}...", truncate_at_char_boundary(without_scheme, 50))
    } else {
        without_scheme.to_string()
    }
}

/// Send a screenshot to all pending channel targets for a session.
/// Uses `peek_channel_replies` so targets remain for the final text response.
pub(crate) async fn send_screenshot_to_channels(
    state: &Arc<dyn ChatRuntime>,
    activity_id: &str,
    session_key: &str,
    screenshot_data: &str,
    caption: Option<&str>,
) {
    let targets = state.peek_channel_replies(session_key).await;
    if targets.is_empty() {
        return;
    }

    let outbound = match state.channel_outbound() {
        Some(o) => o,
        None => {
            note_delivery_failed(state, activity_id).await;
            return;
        },
    };

    if !dispatch_screenshot_to_targets(outbound, targets, screenshot_data, caption).await {
        note_delivery_failed(state, activity_id).await;
    }
}

fn build_screenshot_reply_payload(
    screenshot_data: &str,
    caption: Option<&str>,
) -> moltis_common::types::ReplyPayload {
    use moltis_common::types::{MediaAttachment, ReplyPayload};

    // Extract actual MIME from "data:image/jpeg;base64,..." instead of
    // hardcoding PNG — supports JPEG, GIF, WebP from send_image tool.
    let mime_type = screenshot_data
        .strip_prefix("data:")
        .and_then(|s| s.split(';').next())
        .unwrap_or("image/png")
        .to_string();

    ReplyPayload {
        text: caption.unwrap_or_default().to_string(),
        media: Some(MediaAttachment {
            url: screenshot_data.to_string(),
            mime_type,
            filename: None,
        }),
        reply_to_id: None,
        silent: false,
    }
}

async fn dispatch_screenshot_to_targets(
    outbound: Arc<dyn moltis_channels::ChannelOutbound>,
    targets: Vec<moltis_channels::ChannelReplyTarget>,
    screenshot_data: &str,
    caption: Option<&str>,
) -> bool {
    let payload = build_screenshot_reply_payload(screenshot_data, caption);

    let mut tasks = JoinSet::new();
    for target in targets {
        let outbound = Arc::clone(&outbound);
        let payload = payload.clone();
        let to = target.outbound_to().into_owned();
        tasks.spawn(async move {
            let reply_to = target.message_id.as_deref();
            if let Err(e) = outbound
                .send_media(&target.account_id, &to, &payload, reply_to)
                .await
            {
                warn!(
                    account_id = target.account_id,
                    chat_id = target.chat_id,
                    "failed to send screenshot to channel: {e}"
                );
                // Always tell the user something went wrong — a silently
                // missing screenshot looks like the tool did nothing.
                let error_msg = format!("⚠️ Failed to send screenshot: {e}");
                let _ = outbound
                    .send_text(&target.account_id, &to, &error_msg, reply_to)
                    .await;
                false
            } else {
                debug!(
                    account_id = target.account_id,
                    chat_id = target.chat_id,
                    "sent screenshot to channel"
                );
                true
            }
        });
    }

    let mut delivered = true;
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(sent) => delivered &= sent,
            Err(e) => {
                warn!(error = %e, "channel reply task join failed");
                delivered = false;
            },
        }
    }
    delivered
}

/// Send a document payload to all pending channel targets for a session.
/// Uses `peek_channel_replies` so targets remain for the final text response.
pub(crate) async fn dispatch_document_to_channels(
    state: &Arc<dyn ChatRuntime>,
    activity_id: &str,
    session_key: &str,
    payload: moltis_common::types::ReplyPayload,
) {
    let targets = state.peek_channel_replies(session_key).await;
    if targets.is_empty() {
        return;
    }

    let outbound = match state.channel_outbound() {
        Some(o) => o,
        None => {
            note_delivery_failed(state, activity_id).await;
            return;
        },
    };

    let mut tasks = JoinSet::new();
    for target in targets {
        let outbound = Arc::clone(&outbound);
        let payload = payload.clone();
        let to = target.outbound_to().into_owned();
        tasks.spawn(async move {
            let reply_to = target.message_id.as_deref();
            if let Err(e) = outbound
                .send_media(&target.account_id, &to, &payload, reply_to)
                .await
            {
                warn!(
                    account_id = target.account_id,
                    chat_id = target.chat_id,
                    thread_id = target.thread_id.as_deref().unwrap_or("-"),
                    "failed to send document to channel: {e}"
                );
                let error_msg = format!("\u{26a0}\u{fe0f} Failed to send document: {e}");
                let _ = outbound
                    .send_text(&target.account_id, &to, &error_msg, reply_to)
                    .await;
                false
            } else {
                debug!(
                    account_id = target.account_id,
                    chat_id = target.chat_id,
                    thread_id = target.thread_id.as_deref().unwrap_or("-"),
                    "sent document to channel"
                );
                true
            }
        });
    }

    let mut delivered = true;
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(sent) => delivered &= sent,
            Err(e) => {
                warn!(error = %e, "channel document task join failed");
                delivered = false;
            },
        }
    }
    if !delivered {
        note_delivery_failed(state, activity_id).await;
    }
}

/// Build a `ReplyPayload` from a data URI (legacy path).
pub(crate) fn document_payload_from_data_uri(
    data_uri: &str,
    filename: Option<&str>,
    caption: Option<&str>,
) -> moltis_common::types::ReplyPayload {
    use moltis_common::types::{MediaAttachment, ReplyPayload};

    let mime_type = data_uri
        .strip_prefix("data:")
        .and_then(|s| s.split(';').next())
        .unwrap_or("application/octet-stream")
        .to_string();

    ReplyPayload {
        text: caption.unwrap_or_default().to_string(),
        media: Some(MediaAttachment {
            url: data_uri.to_string(),
            mime_type,
            filename: filename.map(String::from),
        }),
        reply_to_id: None,
        silent: false,
    }
}

/// Build a `ReplyPayload` by reading from the session media directory.
/// Returns `None` if the store is unavailable or the read fails.
pub(crate) async fn document_payload_from_ref(
    session_store: Option<&Arc<SessionStore>>,
    session_key: &str,
    media_ref: &str,
    mime_type: &str,
    filename: Option<&str>,
    caption: Option<&str>,
) -> Option<moltis_common::types::ReplyPayload> {
    use moltis_common::types::{MediaAttachment, ReplyPayload};

    let store = match session_store {
        Some(s) => s,
        None => {
            warn!("document_payload_from_ref: no session store available");
            return None;
        },
    };

    let ref_filename = match media_ref.rsplit('/').next() {
        Some(f) => f,
        None => {
            warn!(media_ref, "invalid document_ref path");
            return None;
        },
    };

    let bytes = match store.read_media(session_key, ref_filename).await {
        Ok(b) => b,
        Err(e) => {
            warn!(media_ref, error = %e, "failed to read document from media dir");
            return None;
        },
    };

    let b64 = {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    };
    let data_uri = format!("data:{mime_type};base64,{b64}");

    Some(ReplyPayload {
        text: caption.unwrap_or_default().to_string(),
        media: Some(MediaAttachment {
            url: data_uri,
            mime_type: mime_type.to_string(),
            filename: filename.map(String::from),
        }),
        reply_to_id: None,
        silent: false,
    })
}

/// Send a native location pin to all pending channel targets for a session.
/// Uses `peek_channel_replies` so targets remain for the final text response.
pub(crate) async fn send_location_to_channels(
    state: &Arc<dyn ChatRuntime>,
    activity_id: &str,
    session_key: &str,
    latitude: f64,
    longitude: f64,
    title: Option<&str>,
) {
    let targets = state.peek_channel_replies(session_key).await;
    if targets.is_empty() {
        return;
    }

    let outbound = match state.channel_outbound() {
        Some(o) => o,
        None => {
            note_delivery_failed(state, activity_id).await;
            return;
        },
    };

    let title_owned = title.map(String::from);

    let mut tasks = JoinSet::new();
    for target in targets {
        let outbound = Arc::clone(&outbound);
        let title_ref = title_owned.clone();
        let to = target.outbound_to().into_owned();
        tasks.spawn(async move {
            let reply_to = target.message_id.as_deref();
            if let Err(e) = outbound
                .send_location(
                    &target.account_id,
                    &to,
                    latitude,
                    longitude,
                    title_ref.as_deref(),
                    reply_to,
                )
                .await
            {
                warn!(
                    account_id = target.account_id,
                    chat_id = target.chat_id,
                    thread_id = target.thread_id.as_deref().unwrap_or("-"),
                    "failed to send location to channel: {e}"
                );
                false
            } else {
                debug!(
                    account_id = target.account_id,
                    chat_id = target.chat_id,
                    thread_id = target.thread_id.as_deref().unwrap_or("-"),
                    "sent location pin to channel"
                );
                true
            }
        });
    }

    let mut delivered = true;
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(sent) => delivered &= sent,
            Err(e) => {
                warn!(error = %e, "channel location task join failed");
                delivered = false;
            },
        }
    }
    if !delivered {
        note_delivery_failed(state, activity_id).await;
    }
}

pub(crate) mod tts;

#[cfg(test)]
mod tests;
