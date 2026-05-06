//! Streaming (edit-in-place) outbound message handling for Telegram.

use {
    async_trait::async_trait,
    std::time::Duration,
    teloxide::{prelude::*, types::ChatAction},
    tracing::debug,
};

use moltis_channels::{
    Result,
    plugin::{ChannelStreamOutbound, StreamEvent, StreamReceiver},
};

use crate::{
    config::StreamMode,
    markdown::{self, TELEGRAM_MAX_MESSAGE_LEN},
    topic::parse_chat_target,
};

use super::{TYPING_REFRESH_INTERVAL, TelegramOutbound};

pub(super) fn has_reached_stream_min_initial_chars(
    accumulated: &str,
    min_initial_chars: usize,
) -> bool {
    accumulated.chars().count() >= min_initial_chars
}

pub(super) fn should_send_stream_completion_notification(
    notify_on_complete: bool,
    has_streamed_text: bool,
    sent_non_silent_completion_chunks: bool,
) -> bool {
    notify_on_complete && has_streamed_text && !sent_non_silent_completion_chunks
}

#[async_trait]
impl ChannelStreamOutbound for TelegramOutbound {
    async fn send_stream(
        &self,
        account_id: &str,
        to: &str,
        reply_to: Option<&str>,
        mut stream: StreamReceiver,
    ) -> Result<()> {
        let bot = self.get_bot(account_id)?;
        let (chat_id, thread_id) = parse_chat_target(to)?;
        let rp = self.reply_params(account_id, reply_to);
        let stream_cfg = self.stream_send_config(account_id);

        // Send typing indicator
        let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;
        let mut stream_message_id: Option<teloxide::types::MessageId> = None;

        let mut accumulated = String::new();
        let mut last_edit = tokio::time::Instant::now();
        let throttle = Duration::from_millis(stream_cfg.edit_throttle_ms);
        let mut typing_interval = tokio::time::interval(TYPING_REFRESH_INTERVAL);
        typing_interval.tick().await; // consume the immediate first tick

        loop {
            tokio::select! {
                event = stream.recv() => {
                    let Some(event) = event else { break };
                    match event {
                        StreamEvent::Delta(delta) => {
                            accumulated.push_str(&delta);
                            if stream_message_id.is_none() {
                                if has_reached_stream_min_initial_chars(
                                    &accumulated,
                                    stream_cfg.min_initial_chars,
                                ) {
                                    let html = markdown::markdown_to_telegram_html(&accumulated);
                                    let display = markdown::truncate_at_char_boundary(
                                        &html,
                                        TELEGRAM_MAX_MESSAGE_LEN,
                                    );
                                    let message_id = self
                                        .send_chunk_with_fallback(
                                            &bot,
                                            account_id,
                                            to,
                                            chat_id,
                                            thread_id,
                                            display,
                                            rp.as_ref(),
                                            false,
                                        )
                                        .await?;
                                    stream_message_id = Some(message_id);
                                    last_edit = tokio::time::Instant::now();
                                }
                                continue;
                            }

                            if last_edit.elapsed() >= throttle {
                                let html = markdown::markdown_to_telegram_html(&accumulated);
                                // Telegram rejects edits with identical content; truncate to limit.
                                let display =
                                    markdown::truncate_at_char_boundary(&html, TELEGRAM_MAX_MESSAGE_LEN);
                                if let Some(msg_id) = stream_message_id {
                                    let _ = self
                                        .edit_chunk_with_fallback(
                                            &bot, account_id, to, chat_id, msg_id, display,
                                        )
                                        .await;
                                    last_edit = tokio::time::Instant::now();
                                }
                            }
                        },
                        StreamEvent::Done => {
                            break;
                        },
                        StreamEvent::Error(e) => {
                            debug!("stream error: {e}");
                            break;
                        },
                    }
                }
                _ = typing_interval.tick() => {
                    // Re-send typing indicator to keep it visible during
                    // long-running tool execution or pauses in the stream.
                    let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;
                }
            }
        }

        // Final edit with complete content
        if !accumulated.is_empty() {
            let chunks = markdown::chunk_markdown_html(&accumulated, TELEGRAM_MAX_MESSAGE_LEN);
            let mut sent_non_silent_completion_chunks = false;
            if let Some((first, rest)) = chunks.split_first() {
                if let Some(msg_id) = stream_message_id {
                    self.edit_chunk_with_fallback(&bot, account_id, to, chat_id, msg_id, first)
                        .await?;
                } else {
                    self.send_chunk_with_fallback(
                        &bot,
                        account_id,
                        to,
                        chat_id,
                        thread_id,
                        first,
                        rp.as_ref(),
                        false,
                    )
                    .await?;
                    sent_non_silent_completion_chunks = true;
                }

                // Send remaining chunks as new messages.
                for chunk in rest {
                    self.send_chunk_with_fallback(
                        &bot,
                        account_id,
                        to,
                        chat_id,
                        thread_id,
                        chunk,
                        rp.as_ref(),
                        false,
                    )
                    .await?;
                    sent_non_silent_completion_chunks = true;
                }
            }

            if should_send_stream_completion_notification(
                stream_cfg.notify_on_complete,
                true,
                sent_non_silent_completion_chunks,
            ) {
                self.send_chunk_with_fallback(
                    &bot,
                    account_id,
                    to,
                    chat_id,
                    thread_id,
                    "Reply complete.",
                    rp.as_ref(),
                    false,
                )
                .await?;
            }
        }

        Ok(())
    }

    async fn is_stream_enabled(&self, account_id: &str) -> bool {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .is_some_and(|s| s.config.stream_mode != StreamMode::Off)
    }
}
