//! Fanning a finished reply out to the channels that asked for it.
//!
//! Split from the main channel module to stay inside the file-size limit; the
//! reply fan-out is a self-contained concern, and keeping it together is what
//! makes the "every delivery path records a trace link" rule checkable by
//! reading one file.

use std::{collections::HashSet, sync::Arc};

use tracing::warn;

use crate::{
    agent_loop::ChannelReplyTargetKey,
    channels::{build_tts_payload, format_logbook_html},
    runtime::ChatRuntime,
    types::ReplyMedium,
};

/// Send the reply text (optionally carrying an activity logbook) and link the
/// messages it produced to the trace that wrote them.
///
/// One helper for both the Telegram and generic branches so a delivery path
/// cannot quietly lose feedback attribution by picking the non-reporting send:
/// the reporting variants are the only ones reachable from here. Channels that
/// cannot report ids return an empty list and lose attribution, not the reply.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn deliver_text_reply(
    outbound: &Arc<dyn moltis_channels::plugin::ChannelOutbound>,
    feedback: Option<&moltis_channels::FeedbackService>,
    target: &moltis_channels::ChannelReplyTarget,
    to: &str,
    text: &str,
    logbook_html: &str,
    reply_to: Option<&str>,
    session_key: &str,
) {
    let result = if logbook_html.is_empty() {
        outbound
            .send_text_reporting_ids(&target.account_id, to, text, reply_to)
            .await
    } else {
        outbound
            .send_text_with_suffix_reporting_ids(
                &target.account_id,
                to,
                text,
                logbook_html,
                reply_to,
            )
            .await
    };
    match result {
        Ok(message_ids) => {
            crate::channel_feedback::record_reply_trace(
                feedback,
                target,
                &message_ids,
                session_key,
            )
            .await;
        },
        Err(e) => {
            warn!(
                account_id = target.account_id,
                chat_id = target.chat_id,
                thread_id = target.thread_id.as_deref().unwrap_or("-"),
                "failed to send channel reply: {e}"
            );
        },
    }
}

pub(crate) async fn deliver_channel_replies_to_targets(
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
    // Resolved once rather than per target: it is the same service either way.
    let feedback = state.feedback();
    let mut tasks = Vec::with_capacity(targets.len());
    for target in targets {
        let outbound = Arc::clone(&outbound);
        let state = Arc::clone(&state);
        let feedback = feedback.clone();
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
                        } else {
                            // Check if transcript fits as Telegram caption (when feature enabled).
                            // When telegram feature is disabled, this evaluates to false and we
                            // send voice + follow-up text.
                            #[cfg(feature = "telegram")]
                            let fits_in_caption = transcript.len()
                                <= moltis_telegram::markdown::TELEGRAM_CAPTION_LIMIT;
                            #[cfg(not(feature = "telegram"))]
                            let fits_in_caption = false;

                            if fits_in_caption {
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
                                // The transcript follow-up is the readable form
                                // of this turn's answer, so it is as much a
                                // reaction target as a plain text reply and
                                // goes out through the same attributing path.
                                deliver_text_reply(
                                    &outbound,
                                    feedback.as_deref(),
                                    &target,
                                    &to,
                                    &transcript,
                                    &logbook_html,
                                    None,
                                    &session_key,
                                )
                                .await;
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
                        deliver_text_reply(
                            &outbound,
                            feedback.as_deref(),
                            &target,
                            &to,
                            &text,
                            &logbook_html,
                            reply_to,
                            &session_key,
                        )
                        .await;
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
                        deliver_text_reply(
                            &outbound,
                            feedback.as_deref(),
                            &target,
                            &to,
                            &text,
                            &logbook_html,
                            reply_to,
                            &session_key,
                        )
                        .await;
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

#[cfg(test)]
mod tests {
    use {
        super::*,
        async_trait::async_trait,
        moltis_channels::{ChannelReplyTarget, ChannelType},
        moltis_common::types::ReplyPayload,
        std::sync::Mutex,
    };

    /// Records which send method was used and hands back ids, so a test can
    /// tell an attributable delivery from one that silently drops the ids.
    #[derive(Default)]
    struct ReportingOutbound {
        calls: Mutex<Vec<&'static str>>,
    }

    #[async_trait]
    impl moltis_channels::ChannelOutbound for ReportingOutbound {
        async fn send_text(
            &self,
            _account_id: &str,
            _to: &str,
            _text: &str,
            _reply_to: Option<&str>,
        ) -> moltis_channels::Result<()> {
            self.calls
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push("send_text");
            Ok(())
        }

        async fn send_media(
            &self,
            _account_id: &str,
            _to: &str,
            _payload: &ReplyPayload,
            _reply_to: Option<&str>,
        ) -> moltis_channels::Result<()> {
            Ok(())
        }

        async fn send_text_reporting_ids(
            &self,
            _account_id: &str,
            _to: &str,
            _text: &str,
            _reply_to: Option<&str>,
        ) -> moltis_channels::Result<Vec<String>> {
            self.calls
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push("send_text_reporting_ids");
            Ok(vec!["plain-1".into()])
        }

        async fn send_text_with_suffix(
            &self,
            _account_id: &str,
            _to: &str,
            _text: &str,
            _suffix_html: &str,
            _reply_to: Option<&str>,
        ) -> moltis_channels::Result<()> {
            self.calls
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push("send_text_with_suffix");
            Ok(())
        }

        async fn send_text_with_suffix_reporting_ids(
            &self,
            _account_id: &str,
            _to: &str,
            _text: &str,
            _suffix_html: &str,
            _reply_to: Option<&str>,
        ) -> moltis_channels::Result<Vec<String>> {
            self.calls
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push("send_text_with_suffix_reporting_ids");
            Ok(vec!["logbook-1".into()])
        }
    }

    fn reply_target() -> ChannelReplyTarget {
        ChannelReplyTarget {
            ack_message_id: None,
            channel_type: ChannelType::Discord,
            account_id: "acct".into(),
            chat_id: "chan".into(),
            message_id: None,
            thread_id: None,
        }
    }

    /// A reply carrying an activity logbook is still a reply someone can react
    /// to, so it must go out through the id-reporting suffix send rather than
    /// the plain one, which reports nothing and loses attribution.
    #[tokio::test]
    async fn a_logbook_reply_uses_the_id_reporting_suffix_send() {
        let recorder = Arc::new(ReportingOutbound::default());
        let outbound: Arc<dyn moltis_channels::plugin::ChannelOutbound> = recorder.clone();

        deliver_text_reply(
            &outbound,
            None,
            &reply_target(),
            "chan",
            "the answer",
            "<blockquote>log</blockquote>",
            None,
            "session",
        )
        .await;

        let calls = recorder.calls.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(*calls, vec!["send_text_with_suffix_reporting_ids"]);
    }

    #[tokio::test]
    async fn a_plain_reply_uses_the_id_reporting_text_send() {
        let recorder = Arc::new(ReportingOutbound::default());
        let outbound: Arc<dyn moltis_channels::plugin::ChannelOutbound> = recorder.clone();

        deliver_text_reply(
            &outbound,
            None,
            &reply_target(),
            "chan",
            "the answer",
            "",
            None,
            "session",
        )
        .await;

        let calls = recorder.calls.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(*calls, vec!["send_text_reporting_ids"]);
    }
}
