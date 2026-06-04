//! Streaming (edit-in-place) outbound message handling for Telegram.

use {
    async_trait::async_trait,
    std::time::Duration,
    teloxide::{
        prelude::*,
        types::{ChatAction, ParseMode},
    },
    tracing::{debug, warn},
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

const MIN_PROGRESS_FLUSH_INTERVAL: Duration = Duration::from_millis(250);

pub(super) fn has_reached_stream_min_initial_chars(
    accumulated: &str,
    min_initial_chars: usize,
) -> bool {
    accumulated.chars().count() >= min_initial_chars
}

pub(super) fn format_stream_progress_html(accumulated: &str) -> String {
    let body = markdown::markdown_to_telegram_html(accumulated);
    let html = format!("<b>Progress update</b>\n\n{body}");
    markdown::truncate_at_char_boundary(&html, TELEGRAM_MAX_MESSAGE_LEN).to_string()
}

pub(super) fn stream_progress_cleanup_html() -> &'static str {
    "<i>Progress update complete; final answer follows.</i>"
}

pub(super) struct StreamProgressState {
    accumulated: String,
    last_progress_at: Option<tokio::time::Instant>,
    last_rendered_html: Option<String>,
    defer_until: Option<tokio::time::Instant>,
    min_initial_chars: usize,
    dirty: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProgressEditResult {
    Applied,
    RateLimited(Duration),
    Failed,
}

impl StreamProgressState {
    pub(super) fn new(min_initial_chars: usize) -> Self {
        Self {
            accumulated: String::new(),
            last_progress_at: None,
            last_rendered_html: None,
            defer_until: None,
            min_initial_chars,
            dirty: false,
        }
    }

    pub(super) fn push_delta(&mut self, delta: &str) {
        self.accumulated.push_str(delta);
        self.dirty = true;
    }

    pub(super) fn should_send_initial_progress(&self) -> bool {
        self.last_progress_at.is_none()
            && has_reached_stream_min_initial_chars(&self.accumulated, self.min_initial_chars)
    }

    pub(super) fn should_flush_progress(
        &self,
        now: tokio::time::Instant,
        throttle: Duration,
    ) -> bool {
        let Some(last_progress_at) = self.last_progress_at else {
            return false;
        };
        if let Some(defer_until) = self.defer_until
            && now < defer_until
        {
            return false;
        }
        self.dirty && now.saturating_duration_since(last_progress_at) >= throttle
    }

    fn current_progress_html(&self) -> String {
        format_stream_progress_html(&self.accumulated)
    }

    pub(super) fn mark_progress_sent(&mut self, now: tokio::time::Instant, rendered_html: &str) {
        self.last_progress_at = Some(now);
        self.last_rendered_html = Some(rendered_html.to_string());
        self.defer_until = None;
        self.dirty = false;
    }

    pub(super) fn mark_progress_observed(
        &mut self,
        now: tokio::time::Instant,
        rendered_html: &str,
    ) {
        self.mark_progress_sent(now, rendered_html);
    }

    pub(super) fn defer_progress_until(&mut self, until: tokio::time::Instant) {
        self.defer_until = match self.defer_until {
            Some(existing) if existing > until => Some(existing),
            _ => Some(until),
        };
    }

    fn rendered_html_changed(&self, rendered_html: &str) -> bool {
        match self.last_rendered_html.as_deref() {
            Some(last) => last != rendered_html,
            None => true,
        }
    }
}

impl TelegramOutbound {
    async fn edit_progress_chunk_once(
        &self,
        bot: &Bot,
        account_id: &str,
        to: &str,
        chat_id: ChatId,
        message_id: teloxide::types::MessageId,
        chunk: &str,
    ) -> ProgressEditResult {
        match bot
            .edit_message_text(chat_id, message_id, chunk)
            .parse_mode(ParseMode::Html)
            .await
        {
            Ok(_) => ProgressEditResult::Applied,
            Err(error) if super::retry::is_message_not_modified_error(&error) => {
                ProgressEditResult::Applied
            },
            Err(error) => {
                if let Some(wait) = super::retry::retry_after_duration(&error) {
                    warn!(
                        account_id,
                        chat_id = to,
                        retry_after_secs = wait.as_secs(),
                        "telegram progress edit rate limited; skipping intermediate update"
                    );
                    return ProgressEditResult::RateLimited(wait);
                } else {
                    warn!(
                        account_id,
                        chat_id = to,
                        error = %error,
                        "telegram progress edit failed; skipping intermediate update"
                    );
                }
                ProgressEditResult::Failed
            },
        }
    }

    pub(super) async fn cleanup_progress_message(
        &self,
        bot: &Bot,
        account_id: &str,
        to: &str,
        chat_id: ChatId,
        message_id: teloxide::types::MessageId,
    ) -> bool {
        match bot.delete_message(chat_id, message_id).await {
            Ok(_) => return true,
            Err(error) => {
                if let Some(wait) = super::retry::retry_after_duration(&error) {
                    warn!(
                        account_id,
                        chat_id = to,
                        retry_after_secs = wait.as_secs(),
                        "telegram progress delete rate limited; trying cleanup marker edit"
                    );
                } else {
                    warn!(
                        account_id,
                        chat_id = to,
                        error = %error,
                        "telegram progress delete failed; trying cleanup marker edit"
                    );
                }
            },
        }

        match self
            .edit_progress_chunk_once(
                bot,
                account_id,
                to,
                chat_id,
                message_id,
                stream_progress_cleanup_html(),
            )
            .await
        {
            ProgressEditResult::Applied => true,
            ProgressEditResult::RateLimited(wait) => {
                warn!(
                    account_id,
                    chat_id = to,
                    retry_after_secs = wait.as_secs(),
                    "telegram progress cleanup edit also rate limited; message may remain visible"
                );
                false
            },
            ProgressEditResult::Failed => {
                warn!(
                    account_id,
                    chat_id = to,
                    "telegram progress cleanup edit also failed; message may remain visible"
                );
                false
            },
        }
    }
}

fn record_progress_edit_result(
    progress: &mut StreamProgressState,
    now: tokio::time::Instant,
    rendered_html: &str,
    throttle: Duration,
    result: ProgressEditResult,
) {
    match result {
        ProgressEditResult::Applied => progress.mark_progress_sent(now, rendered_html),
        ProgressEditResult::RateLimited(wait) => {
            progress.defer_progress_until(now + wait.max(throttle))
        },
        ProgressEditResult::Failed => progress.defer_progress_until(now + throttle),
    }
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
        let mut progress_message_id: Option<teloxide::types::MessageId> = None;

        let mut progress = StreamProgressState::new(stream_cfg.min_initial_chars);
        let throttle =
            Duration::from_millis(stream_cfg.edit_throttle_ms).max(MIN_PROGRESS_FLUSH_INTERVAL);
        let mut typing_interval = tokio::time::interval(TYPING_REFRESH_INTERVAL);
        typing_interval.tick().await; // consume the immediate first tick
        let mut flush_interval = tokio::time::interval(throttle);
        flush_interval.tick().await; // consume the immediate first tick

        loop {
            tokio::select! {
                event = stream.recv() => {
                    let Some(event) = event else { break };
                    match event {
                        StreamEvent::Delta(delta) => {
                            progress.push_delta(&delta);
                            if progress_message_id.is_none() {
                                if progress.should_send_initial_progress() {
                                    let display = progress.current_progress_html();
                                    let message_id = self
                                        .send_chunk_with_fallback(
                                            &bot,
                                            account_id,
                                            to,
                                            chat_id,
                                            thread_id,
                                            &display,
                                            rp.as_ref(),
                                            true,
                                        )
                                        .await?;
                                    let now = tokio::time::Instant::now();
                                    progress_message_id = Some(message_id);
                                    progress.mark_progress_sent(now, &display);
                                }
                                continue;
                            }

                            let now = tokio::time::Instant::now();
                            if progress.should_flush_progress(now, throttle) {
                                let display = progress.current_progress_html();
                                if progress.rendered_html_changed(&display)
                                    && let Some(msg_id) = progress_message_id
                                {
                                    let result = self
                                        .edit_progress_chunk_once(
                                            &bot, account_id, to, chat_id, msg_id, &display,
                                        )
                                        .await;
                                    record_progress_edit_result(
                                        &mut progress,
                                        now,
                                        &display,
                                        throttle,
                                        result,
                                    );
                                } else {
                                    progress.mark_progress_observed(now, &display);
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
                _ = flush_interval.tick() => {
                    let now = tokio::time::Instant::now();
                    if progress.should_flush_progress(now, throttle) {
                        let display = progress.current_progress_html();
                        if progress.rendered_html_changed(&display)
                            && let Some(msg_id) = progress_message_id
                        {
                            let result = self
                                .edit_progress_chunk_once(
                                    &bot, account_id, to, chat_id, msg_id, &display,
                                )
                                .await;
                            record_progress_edit_result(
                                &mut progress,
                                now,
                                &display,
                                throttle,
                                result,
                            );
                        } else {
                            progress.mark_progress_observed(now, &display);
                        }
                    }
                }
            }
        }

        if let Some(msg_id) = progress_message_id {
            let _ = self
                .cleanup_progress_message(&bot, account_id, to, chat_id, msg_id)
                .await;
        }

        Ok(())
    }

    async fn is_stream_enabled(&self, account_id: &str) -> bool {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .is_some_and(|s| s.config.stream_mode != StreamMode::Off)
    }

    async fn streams_final_replies(&self, _account_id: &str) -> bool {
        false
    }
}
