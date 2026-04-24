//! Channel delivery, TTS, push notifications, tool status, screenshots, documents, and location.

use std::{collections::HashSet, sync::Arc, time::Duration};

use {
    serde::{Deserialize, Serialize},
    serde_json::Value,
    tracing::{debug, info, warn},
};

use moltis_sessions::store::SessionStore;

use crate::{
    agent_loop::ChannelReplyTargetKey, compaction_run, error, runtime::ChatRuntime, types::*,
};

/// Build the SPA URL for a push notification click-through.
///
/// Must match the frontend `sessionPath()` in `router.ts`:
/// `/chats/${key.replace(/:/g, "/")}`.
#[cfg(any(feature = "push-notifications", test))]
pub(crate) fn push_notification_url(session_key: &str) -> String {
    format!("/chats/{}", session_key.replace(':', "/"))
}

#[cfg(feature = "push-notifications")]
pub(crate) async fn send_chat_push_notification(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    text: &str,
) {
    // Create a short summary of the response (first 100 chars)
    let summary = if text.len() > 100 {
        format!("{}…", truncate_at_char_boundary(text, 100))
    } else {
        text.to_string()
    };

    let title = "Message received";
    let url = push_notification_url(session_key);

    match state
        .send_push_notification(title, &summary, Some(&url), Some(session_key))
        .await
    {
        Ok(sent) => {
            tracing::info!(sent, "push notification sent");
        },
        Err(e) => {
            tracing::warn!("failed to send push notification: {e}");
        },
    }
}

/// Drain any pending channel reply targets for a session and send the
/// response text back to each originating channel via outbound.
/// Each delivery runs in its own spawned task so slow network calls
/// don't block each other or the chat pipeline.
pub(crate) async fn deliver_channel_replies(
    state: &Arc<dyn ChatRuntime>,
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
    deliver_channel_replies_to_targets(
        outbound,
        targets,
        session_key,
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
fn format_logbook_html(entries: &[String]) -> String {
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
) {
    if targets.is_empty() || logbook_html.is_empty() {
        return;
    }

    let html = logbook_html.to_string();
    let mut tasks = Vec::with_capacity(targets.len());
    for target in targets {
        let outbound = Arc::clone(&outbound);
        let html = html.clone();
        let to = target.outbound_to().into_owned();
        tasks.push(tokio::spawn(async move {
            if let Err(e) = outbound
                .send_html(&target.account_id, &to, &html, None)
                .await
            {
                warn!(
                    account_id = target.account_id,
                    chat_id = target.chat_id,
                    thread_id = target.thread_id.as_deref().unwrap_or("-"),
                    "failed to send logbook follow-up: {e}"
                );
            }
        }));
    }

    for task in tasks {
        if let Err(e) = task.await {
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
    let mut tasks = Vec::with_capacity(targets.len());
    for target in targets {
        let outbound = Arc::clone(&outbound);
        let message = message.clone();
        let to = target.outbound_to().into_owned();
        tasks.push(tokio::spawn(async move {
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
        }));
    }

    for task in tasks {
        if let Err(e) = task.await {
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
    let mut tasks = Vec::with_capacity(targets.len());
    for target in targets {
        let outbound = Arc::clone(&outbound);
        let message = message.clone();
        let to = target.outbound_to().into_owned();
        tasks.push(tokio::spawn(async move {
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
        }));
    }

    for task in tasks {
        if let Err(e) = task.await {
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
    let mut tasks = Vec::with_capacity(targets.len());
    for target in targets {
        let outbound = Arc::clone(&outbound);
        let error_text = error_text.clone();
        let logbook_html = logbook_html.clone();
        let to = target.outbound_to().into_owned();
        tasks.push(tokio::spawn(async move {
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
        }));
    }

    for task in tasks {
        if let Err(e) = task.await {
            warn!(error = %e, "channel error task join failed");
        }
    }
}

async fn deliver_channel_replies_to_targets(
    outbound: Arc<dyn moltis_channels::plugin::ChannelOutbound>,
    targets: Vec<moltis_channels::ChannelReplyTarget>,
    session_key: &str,
    text: &str,
    state: Arc<dyn ChatRuntime>,
    desired_reply_medium: ReplyMedium,
    status_log: Vec<String>,
    streamed_target_keys: &HashSet<ChannelReplyTargetKey>,
) {
    let session_key = session_key.to_string();
    let text = text.to_string();
    let logbook_html = format_logbook_html(&status_log);
    let mut tasks = Vec::with_capacity(targets.len());
    for target in targets {
        let outbound = Arc::clone(&outbound);
        let state = Arc::clone(&state);
        let session_key = session_key.clone();
        let text = text.clone();
        let logbook_html = logbook_html.clone();
        // Text was already delivered via edit-in-place streaming — skip text
        // caption/follow-up and only send the TTS voice audio.
        let text_already_streamed =
            streamed_target_keys.contains(&ChannelReplyTargetKey::from(&target));
        let to = target.outbound_to().into_owned();
        tasks.push(tokio::spawn(async move {
            let tts_payload = match desired_reply_medium {
                ReplyMedium::Voice => build_tts_payload(&state, &session_key, &target, &text).await,
                ReplyMedium::Text => None,
            };
            let reply_to = target.message_id.as_deref();
            match target.channel_type {
                moltis_channels::ChannelType::Telegram => match tts_payload {
                    Some(mut payload) => {
                        let transcript = std::mem::take(&mut payload.text);

                        if text_already_streamed {
                            // Text was already streamed — send voice audio only.
                            if let Err(e) = outbound
                                .send_media(&target.account_id, &to, &payload, reply_to)
                                .await
                            {
                                warn!(
                                    account_id = target.account_id,
                                    chat_id = target.chat_id,
                                    thread_id = target.thread_id.as_deref().unwrap_or("-"),
                                    "failed to send channel voice reply: {e}"
                                );
                            }
                            // Send logbook as a follow-up if present.
                            if !logbook_html.is_empty()
                                && let Err(e) = outbound
                                    .send_html(&target.account_id, &to, &logbook_html, None)
                                    .await
                            {
                                warn!(
                                    account_id = target.account_id,
                                    chat_id = target.chat_id,
                                    thread_id = target.thread_id.as_deref().unwrap_or("-"),
                                    "failed to send logbook follow-up: {e}"
                                );
                            }
                        } else if transcript.len()
                            <= moltis_telegram::markdown::TELEGRAM_CAPTION_LIMIT
                        {
                            // Short transcript fits as a caption on the voice message.
                            payload.text = transcript;
                            if let Err(e) = outbound
                                .send_media(&target.account_id, &to, &payload, reply_to)
                                .await
                            {
                                warn!(
                                    account_id = target.account_id,
                                    chat_id = target.chat_id,
                                    thread_id = target.thread_id.as_deref().unwrap_or("-"),
                                    "failed to send channel voice reply: {e}"
                                );
                            }
                            // Send logbook as a follow-up if present.
                            if !logbook_html.is_empty()
                                && let Err(e) = outbound
                                    .send_html(&target.account_id, &to, &logbook_html, None)
                                    .await
                            {
                                warn!(
                                    account_id = target.account_id,
                                    chat_id = target.chat_id,
                                    thread_id = target.thread_id.as_deref().unwrap_or("-"),
                                    "failed to send logbook follow-up: {e}"
                                );
                            }
                        } else {
                            // Transcript too long for a caption — send voice
                            // without caption, then the full text as a follow-up.
                            if let Err(e) = outbound
                                .send_media(&target.account_id, &to, &payload, reply_to)
                                .await
                            {
                                warn!(
                                    account_id = target.account_id,
                                    chat_id = target.chat_id,
                                    thread_id = target.thread_id.as_deref().unwrap_or("-"),
                                    "failed to send channel voice reply: {e}"
                                );
                            }
                            let text_result = if logbook_html.is_empty() {
                                outbound
                                    .send_text(&target.account_id, &to, &transcript, None)
                                    .await
                            } else {
                                outbound
                                    .send_text_with_suffix(
                                        &target.account_id,
                                        &to,
                                        &transcript,
                                        &logbook_html,
                                        None,
                                    )
                                    .await
                            };
                            if let Err(e) = text_result {
                                warn!(
                                    account_id = target.account_id,
                                    chat_id = target.chat_id,
                                    thread_id = target.thread_id.as_deref().unwrap_or("-"),
                                    "failed to send transcript follow-up: {e}"
                                );
                            }
                        }
                    },
                    None if text_already_streamed => {
                        // TTS disabled/failed but text was already streamed —
                        // only send logbook follow-up if present.
                        if !logbook_html.is_empty()
                            && let Err(e) = outbound
                                .send_html(&target.account_id, &to, &logbook_html, None)
                                .await
                        {
                            warn!(
                                account_id = target.account_id,
                                chat_id = target.chat_id,
                                thread_id = target.thread_id.as_deref().unwrap_or("-"),
                                "failed to send logbook follow-up: {e}"
                            );
                        }
                    },
                    None => {
                        let result = if logbook_html.is_empty() {
                            outbound
                                .send_text(&target.account_id, &to, &text, reply_to)
                                .await
                        } else {
                            outbound
                                .send_text_with_suffix(
                                    &target.account_id,
                                    &to,
                                    &text,
                                    &logbook_html,
                                    reply_to,
                                )
                                .await
                        };
                        if let Err(e) = result {
                            warn!(
                                account_id = target.account_id,
                                chat_id = target.chat_id,
                                thread_id = target.thread_id.as_deref().unwrap_or("-"),
                                "failed to send channel reply: {e}"
                            );
                        }
                    },
                },
                _ => match tts_payload {
                    Some(payload) => {
                        if let Err(e) = outbound
                            .send_media(&target.account_id, &to, &payload, reply_to)
                            .await
                        {
                            warn!(
                                account_id = target.account_id,
                                chat_id = target.chat_id,
                                thread_id = target.thread_id.as_deref().unwrap_or("-"),
                                "failed to send channel voice reply: {e}"
                            );
                        }
                    },
                    None if text_already_streamed => {
                        // TTS disabled/failed but text was already streamed —
                        // only send logbook follow-up if present.
                        if !logbook_html.is_empty()
                            && let Err(e) = outbound
                                .send_html(&target.account_id, &to, &logbook_html, None)
                                .await
                        {
                            warn!(
                                account_id = target.account_id,
                                chat_id = target.chat_id,
                                thread_id = target.thread_id.as_deref().unwrap_or("-"),
                                "failed to send logbook follow-up: {e}"
                            );
                        }
                    },
                    None => {
                        let result = if logbook_html.is_empty() {
                            outbound
                                .send_text(&target.account_id, &to, &text, reply_to)
                                .await
                        } else {
                            outbound
                                .send_text_with_suffix(
                                    &target.account_id,
                                    &to,
                                    &text,
                                    &logbook_html,
                                    reply_to,
                                )
                                .await
                        };
                        if let Err(e) = result {
                            warn!(
                                account_id = target.account_id,
                                chat_id = target.chat_id,
                                thread_id = target.thread_id.as_deref().unwrap_or("-"),
                                "failed to send channel reply: {e}"
                            );
                        }
                    },
                },
            }
        }));
    }

    for task in tasks {
        if let Err(e) = task.await {
            warn!(error = %e, "channel reply task join failed");
        }
    }
}

#[derive(Debug, Deserialize)]
struct TtsStatusResponse {
    enabled: bool,
}

#[derive(Debug, Serialize)]
struct TtsConvertRequest<'a> {
    text: &'a str,
    format: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "voiceId")]
    voice_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TtsConvertResponse {
    audio: String,
    #[serde(default)]
    mime_type: Option<String>,
}

/// Generate TTS audio bytes for a web UI response.
///
/// Uses the session-level TTS override if configured, otherwise the global TTS
/// config. Returns raw audio bytes (OGG format) on success, `None` if TTS is
/// disabled or generation fails.
pub(crate) async fn generate_tts_audio(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    text: &str,
) -> error::Result<Vec<u8>> {
    use base64::Engine;

    let tts_status = state
        .tts_service()
        .status()
        .await
        .map_err(error::Error::message)?;
    let status: TtsStatusResponse = serde_json::from_value(tts_status)
        .map_err(|_| error::Error::message("invalid tts.status response"))?;
    if !status.enabled {
        return Err(error::Error::message("TTS is disabled or not configured"));
    }

    // Layer 2: strip markdown/URLs the LLM may have included despite the prompt.
    let text = moltis_voice::tts::sanitize_text_for_tts(text);
    let text = text.trim();
    if text.is_empty() {
        return Err(error::Error::message("response has no speakable text"));
    }

    let (_, session_override) = state.tts_overrides(session_key, "").await;

    let request = TtsConvertRequest {
        text,
        format: "ogg",
        provider: session_override.as_ref().and_then(|o| o.provider.clone()),
        voice_id: session_override.as_ref().and_then(|o| o.voice_id.clone()),
        model: session_override.as_ref().and_then(|o| o.model.clone()),
    };

    let request_value = serde_json::to_value(request)
        .map_err(|_| error::Error::message("failed to build tts.convert request"))?;
    let tts_result = state
        .tts_service()
        .convert(request_value)
        .await
        .map_err(error::Error::message)?;

    let response: TtsConvertResponse = serde_json::from_value(tts_result)
        .map_err(|_| error::Error::message("invalid tts.convert response"))?;
    base64::engine::general_purpose::STANDARD
        .decode(&response.audio)
        .map_err(|_| error::Error::message("invalid base64 audio returned by TTS provider"))
}

async fn build_tts_payload(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    target: &moltis_channels::ChannelReplyTarget,
    text: &str,
) -> Option<moltis_common::types::ReplyPayload> {
    use moltis_common::types::{MediaAttachment, ReplyPayload};

    let tts_status = state.tts_service().status().await.ok()?;
    let status: TtsStatusResponse = serde_json::from_value(tts_status).ok()?;
    if !status.enabled {
        return None;
    }

    // Strip markdown/URLs the LLM may have included — use sanitized text
    // only for TTS conversion, but keep the original for the caption.
    let sanitized = moltis_voice::tts::sanitize_text_for_tts(text);

    let channel_key = format!("{}:{}", target.channel_type.as_str(), target.account_id);
    let (channel_override, session_override) = state.tts_overrides(session_key, &channel_key).await;
    let resolved = channel_override.or(session_override);

    let request = TtsConvertRequest {
        text: &sanitized,
        format: "ogg",
        provider: resolved.as_ref().and_then(|o| o.provider.clone()),
        voice_id: resolved.as_ref().and_then(|o| o.voice_id.clone()),
        model: resolved.as_ref().and_then(|o| o.model.clone()),
    };

    let tts_result = state
        .tts_service()
        .convert(serde_json::to_value(request).ok()?)
        .await
        .ok()?;

    let response: TtsConvertResponse = serde_json::from_value(tts_result).ok()?;

    let mime_type = response
        .mime_type
        .unwrap_or_else(|| "audio/ogg".to_string());

    Some(ReplyPayload {
        text: text.to_string(),
        media: Some(MediaAttachment {
            url: format!("data:{mime_type};base64,{}", response.audio),
            mime_type,
            filename: None,
        }),
        reply_to_id: None,
        silent: false,
    })
}

/// Buffer a tool execution status into the channel status log for a session.
/// The buffered entries are appended as a collapsible logbook when the final
/// response is delivered, instead of being sent as separate messages.
pub(crate) async fn send_tool_status_to_channels(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    tool_name: &str,
    arguments: &Value,
) {
    let targets = state.peek_channel_replies(session_key).await;
    if targets.is_empty() {
        return;
    }

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
    session_key: &str,
    screenshot_data: &str,
    caption: Option<&str>,
) {
    use moltis_common::types::{MediaAttachment, ReplyPayload};

    let targets = state.peek_channel_replies(session_key).await;
    if targets.is_empty() {
        return;
    }

    let outbound = match state.channel_outbound() {
        Some(o) => o,
        None => return,
    };

    // Extract actual MIME from "data:image/jpeg;base64,..." instead of
    // hardcoding PNG — supports JPEG, GIF, WebP from send_image tool.
    let mime_type = screenshot_data
        .strip_prefix("data:")
        .and_then(|s| s.split(';').next())
        .unwrap_or("image/png")
        .to_string();

    let payload = ReplyPayload {
        text: caption.unwrap_or_default().to_string(),
        media: Some(MediaAttachment {
            url: screenshot_data.to_string(),
            mime_type,
            filename: None,
        }),
        reply_to_id: None,
        silent: false,
    };

    let mut tasks = Vec::with_capacity(targets.len());
    for target in targets {
        let outbound = Arc::clone(&outbound);
        let payload = payload.clone();
        let to = target.outbound_to().into_owned();
        tasks.push(tokio::spawn(async move {
            {
                let reply_to = target.message_id.as_deref();
                if let Err(e) = outbound
                    .send_media(&target.account_id, &to, &payload, reply_to)
                    .await
                {
                    warn!(
                        account_id = target.account_id,
                        chat_id = target.chat_id,
                        thread_id = target.thread_id.as_deref().unwrap_or("-"),
                        "failed to send screenshot to channel: {e}"
                    );
                    // Notify the user of the error
                    let error_msg = format!("⚠️ Failed to send screenshot: {e}");
                    let _ = outbound
                        .send_text(&target.account_id, &to, &error_msg, reply_to)
                        .await;
                } else {
                    debug!(
                        account_id = target.account_id,
                        chat_id = target.chat_id,
                        thread_id = target.thread_id.as_deref().unwrap_or("-"),
                        "sent screenshot to channel"
                    );
                }
            }
        }));
    }

    for task in tasks {
        if let Err(e) = task.await {
            warn!(error = %e, "channel reply task join failed");
        }
    }
}

/// Send a document payload to all pending channel targets for a session.
/// Uses `peek_channel_replies` so targets remain for the final text response.
pub(crate) async fn dispatch_document_to_channels(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    payload: moltis_common::types::ReplyPayload,
) {
    let targets = state.peek_channel_replies(session_key).await;
    if targets.is_empty() {
        return;
    }

    let outbound = match state.channel_outbound() {
        Some(o) => o,
        None => return,
    };

    let mut tasks = Vec::with_capacity(targets.len());
    for target in targets {
        let outbound = Arc::clone(&outbound);
        let payload = payload.clone();
        let to = target.outbound_to().into_owned();
        tasks.push(tokio::spawn(async move {
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
            } else {
                debug!(
                    account_id = target.account_id,
                    chat_id = target.chat_id,
                    thread_id = target.thread_id.as_deref().unwrap_or("-"),
                    "sent document to channel"
                );
            }
        }));
    }

    for task in tasks {
        if let Err(e) = task.await {
            warn!(error = %e, "channel document task join failed");
        }
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
        None => return,
    };

    let title_owned = title.map(String::from);

    let mut tasks = Vec::with_capacity(targets.len());
    for target in targets {
        let outbound = Arc::clone(&outbound);
        let title_ref = title_owned.clone();
        let to = target.outbound_to().into_owned();
        tasks.push(tokio::spawn(async move {
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
            } else {
                debug!(
                    account_id = target.account_id,
                    chat_id = target.chat_id,
                    thread_id = target.thread_id.as_deref().unwrap_or("-"),
                    "sent location pin to channel"
                );
            }
        }));
    }

    for task in tasks {
        if let Err(e) = task.await {
            warn!(error = %e, "channel location task join failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_notification_url_uses_chats_prefix_and_replaces_colons() {
        // Must match frontend sessionPath(): `/chats/${key.replace(/:/g, "/")}`
        assert_eq!(push_notification_url("session:42"), "/chats/session/42");
    }

    #[test]
    fn push_notification_url_handles_nested_session_keys() {
        assert_eq!(
            push_notification_url("telegram:bot123:chat456"),
            "/chats/telegram/bot123/chat456"
        );
    }

    #[test]
    fn push_notification_url_handles_key_without_colons() {
        assert_eq!(push_notification_url("main"), "/chats/main");
    }
}
