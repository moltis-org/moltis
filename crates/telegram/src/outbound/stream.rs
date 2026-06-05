//! Streaming (edit-in-place) outbound message handling for Telegram.

use {
    async_trait::async_trait,
    std::time::Duration,
    teloxide::{
        prelude::*,
        types::{ChatAction, ChatId, MessageId, ParseMode, ReplyParameters, ThreadId},
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

use super::{StreamSendConfig, TYPING_REFRESH_INTERVAL, TelegramOutbound};

const MIN_PROGRESS_FLUSH_INTERVAL: Duration = Duration::from_millis(250);

pub(super) fn has_reached_stream_min_initial_chars(
    seen_chars: usize,
    min_initial_chars: usize,
) -> bool {
    seen_chars >= min_initial_chars
}

pub(super) fn format_stream_progress_html(progress: &str, older_progress_hidden: bool) -> String {
    fn build(progress: &str, older_progress_hidden: bool) -> (String, bool) {
        let marker = if older_progress_hidden {
            "<i>Older progress hidden.</i>\n\n"
        } else {
            ""
        };
        let prefix = marker.to_string();
        let available = TELEGRAM_MAX_MESSAGE_LEN.saturating_sub(prefix.len());
        let chunks = markdown::chunk_markdown_html(progress, available);
        if chunks.len() <= 1 {
            return (
                format!("{prefix}{}", chunks.into_iter().next().unwrap_or_default()),
                false,
            );
        }

        let (tail, truncated_by_limit) = escaped_recent_tail(progress, available);
        (format!("{prefix}{tail}"), truncated_by_limit)
    }

    let (html, truncated_by_limit) = build(progress, older_progress_hidden);
    if truncated_by_limit && !older_progress_hidden {
        build(progress, true).0
    } else {
        html
    }
}

pub(super) fn stream_progress_cleanup_html() -> &'static str {
    "<i>Final answer follows.</i>"
}

pub(super) struct StreamProgressState {
    accumulated: String,
    seen_chars: usize,
    last_progress_at: Option<tokio::time::Instant>,
    last_rendered_html: Option<String>,
    defer_until: Option<tokio::time::Instant>,
    min_initial_chars: usize,
    max_chars: usize,
    older_progress_hidden: bool,
    dirty: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProgressEditResult {
    Applied,
    RateLimited(Duration),
    Failed,
}

impl StreamProgressState {
    pub(super) fn new(min_initial_chars: usize, max_chars: usize) -> Self {
        Self {
            accumulated: String::new(),
            seen_chars: 0,
            last_progress_at: None,
            last_rendered_html: None,
            defer_until: None,
            min_initial_chars,
            max_chars: max_chars.max(1),
            older_progress_hidden: false,
            dirty: false,
        }
    }

    pub(super) fn push_delta(&mut self, delta: &str) {
        self.seen_chars += delta.chars().count();
        self.accumulated.push_str(delta);
        if trim_to_recent_chars(&mut self.accumulated, self.max_chars) {
            self.older_progress_hidden = true;
        }
        self.dirty = true;
    }

    fn replace_text(&mut self, text: &str) {
        self.accumulated.clear();
        self.seen_chars = 0;
        self.older_progress_hidden = false;
        self.last_rendered_html = None;
        self.defer_until = None;
        self.push_delta(text);
    }

    pub(super) fn should_send_initial_progress(&self) -> bool {
        self.last_progress_at.is_none()
            && has_reached_stream_min_initial_chars(self.seen_chars, self.min_initial_chars)
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

    pub(super) fn current_progress_html(&self) -> String {
        format_stream_progress_html(&self.accumulated, self.older_progress_hidden)
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

struct StreamFinalState {
    accumulated: String,
    seen_chars: usize,
    last_progress_at: Option<tokio::time::Instant>,
    last_rendered_html: Option<String>,
    defer_until: Option<tokio::time::Instant>,
    min_initial_chars: usize,
    dirty: bool,
    message_id: Option<MessageId>,
}

impl StreamFinalState {
    fn new(min_initial_chars: usize) -> Self {
        Self {
            accumulated: String::new(),
            seen_chars: 0,
            last_progress_at: None,
            last_rendered_html: None,
            defer_until: None,
            min_initial_chars,
            dirty: false,
            message_id: None,
        }
    }

    fn push_delta(&mut self, delta: &str) {
        self.seen_chars += delta.chars().count();
        self.accumulated.push_str(delta);
        self.dirty = true;
    }

    fn should_send_initial_final(&self) -> bool {
        self.message_id.is_none()
            && has_reached_stream_min_initial_chars(self.seen_chars, self.min_initial_chars)
    }

    fn should_flush_final(&self, now: tokio::time::Instant, throttle: Duration) -> bool {
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

    fn current_final_html(&self) -> String {
        render_stream_final_html(&self.accumulated)
    }

    fn mark_final_sent(&mut self, now: tokio::time::Instant, rendered_html: &str) {
        self.last_progress_at = Some(now);
        self.last_rendered_html = Some(rendered_html.to_string());
        self.defer_until = None;
        self.dirty = false;
    }

    fn mark_final_observed(&mut self, now: tokio::time::Instant, rendered_html: &str) {
        self.mark_final_sent(now, rendered_html);
    }

    fn defer_final_until(&mut self, until: tokio::time::Instant) {
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

    fn reset(&mut self) {
        let min_initial_chars = self.min_initial_chars;
        *self = Self::new(min_initial_chars);
    }
}

fn render_stream_final_html(text: &str) -> String {
    markdown::chunk_markdown_html(text, TELEGRAM_MAX_MESSAGE_LEN)
        .into_iter()
        .next()
        .unwrap_or_default()
}

impl TelegramOutbound {
    async fn edit_progress_chunk_once(
        &self,
        bot: &Bot,
        account_id: &str,
        to: &str,
        chat_id: ChatId,
        message_id: MessageId,
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
        message_id: MessageId,
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

    async fn delete_progress_message(
        &self,
        bot: &Bot,
        account_id: &str,
        to: &str,
        chat_id: ChatId,
        message_id: MessageId,
    ) -> bool {
        match bot.delete_message(chat_id, message_id).await {
            Ok(_) => true,
            Err(error) => {
                if let Some(wait) = super::retry::retry_after_duration(&error) {
                    warn!(
                        account_id,
                        chat_id = to,
                        retry_after_secs = wait.as_secs(),
                        "telegram progress delete rate limited; progress message may remain visible"
                    );
                } else {
                    warn!(
                        account_id,
                        chat_id = to,
                        error = %error,
                        "telegram progress delete failed; progress message may remain visible"
                    );
                }
                false
            },
        }
    }

    async fn flush_final_stream_message(
        &self,
        bot: &Bot,
        account_id: &str,
        to: &str,
        chat_id: ChatId,
        thread_id: Option<ThreadId>,
        reply_params: Option<&ReplyParameters>,
        stream_cfg: StreamSendConfig,
        progress_message_id: &mut Option<MessageId>,
        final_state: &mut StreamFinalState,
        rendered_html: &str,
        now: tokio::time::Instant,
        throttle: Duration,
        final_delivery: bool,
    ) -> Result<()> {
        if rendered_html.is_empty() {
            final_state.mark_final_observed(now, rendered_html);
            return Ok(());
        }

        if let Some(msg_id) = final_state.message_id {
            let result = self
                .edit_progress_chunk_once(bot, account_id, to, chat_id, msg_id, rendered_html)
                .await;
            if final_delivery && result != ProgressEditResult::Applied {
                warn!(
                    account_id,
                    chat_id = to,
                    "telegram final stream edit failed at completion; sending fallback final message"
                );
                let message_id = self
                    .send_chunk_with_fallback(
                        bot,
                        account_id,
                        to,
                        chat_id,
                        thread_id,
                        rendered_html,
                        reply_params,
                        !stream_cfg.notify_on_complete,
                    )
                    .await?;
                final_state.message_id = Some(message_id);
                final_state.mark_final_sent(now, rendered_html);
                return Ok(());
            }
            record_final_edit_result(final_state, now, rendered_html, throttle, result);
            return Ok(());
        }

        if let Some(progress_msg_id) = *progress_message_id {
            if stream_cfg.notify_on_complete {
                if self
                    .delete_progress_message(bot, account_id, to, chat_id, progress_msg_id)
                    .await
                {
                    *progress_message_id = None;
                }
                let message_id = self
                    .send_chunk_with_fallback(
                        bot,
                        account_id,
                        to,
                        chat_id,
                        thread_id,
                        rendered_html,
                        reply_params,
                        false,
                    )
                    .await?;
                final_state.message_id = Some(message_id);
                final_state.mark_final_sent(now, rendered_html);
                return Ok(());
            }

            let result = self
                .edit_progress_chunk_once(
                    bot,
                    account_id,
                    to,
                    chat_id,
                    progress_msg_id,
                    rendered_html,
                )
                .await;
            match result {
                ProgressEditResult::Applied => {
                    final_state.message_id = Some(progress_msg_id);
                    *progress_message_id = None;
                    final_state.mark_final_sent(now, rendered_html);
                    return Ok(());
                },
                ProgressEditResult::RateLimited(wait) => {
                    final_state.defer_final_until(now + wait.max(throttle));
                },
                ProgressEditResult::Failed => final_state.defer_final_until(now + throttle),
            }
        }

        let message_id = self
            .send_chunk_with_fallback(
                bot,
                account_id,
                to,
                chat_id,
                thread_id,
                rendered_html,
                reply_params,
                !stream_cfg.notify_on_complete,
            )
            .await?;
        final_state.message_id = Some(message_id);
        final_state.mark_final_sent(now, rendered_html);
        Ok(())
    }

    async fn send_remaining_final_chunks(
        &self,
        bot: &Bot,
        account_id: &str,
        to: &str,
        chat_id: ChatId,
        thread_id: Option<ThreadId>,
        reply_params: Option<&ReplyParameters>,
        final_state: &StreamFinalState,
    ) -> Result<()> {
        let chunks =
            markdown::chunk_markdown_html(&final_state.accumulated, TELEGRAM_MAX_MESSAGE_LEN);
        for chunk in chunks.into_iter().skip(1) {
            self.send_chunk_with_fallback(
                bot,
                account_id,
                to,
                chat_id,
                thread_id,
                &chunk,
                reply_params,
                true,
            )
            .await?;
        }
        Ok(())
    }

    async fn finish_final_stream_message(
        &self,
        bot: &Bot,
        account_id: &str,
        to: &str,
        chat_id: ChatId,
        thread_id: Option<ThreadId>,
        reply_params: Option<&ReplyParameters>,
        stream_cfg: StreamSendConfig,
        progress_message_id: &mut Option<MessageId>,
        final_state: &mut StreamFinalState,
        throttle: Duration,
    ) -> Result<()> {
        if final_state.accumulated.is_empty() {
            return Ok(());
        }

        let display = final_state.current_final_html();
        let now = tokio::time::Instant::now();
        if final_state.message_id.is_none() || final_state.rendered_html_changed(&display) {
            self.flush_final_stream_message(
                bot,
                account_id,
                to,
                chat_id,
                thread_id,
                reply_params,
                stream_cfg,
                progress_message_id,
                final_state,
                &display,
                now,
                throttle,
                true,
            )
            .await?;
        }
        self.send_remaining_final_chunks(
            bot,
            account_id,
            to,
            chat_id,
            thread_id,
            reply_params,
            final_state,
        )
        .await
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

fn record_final_edit_result(
    final_state: &mut StreamFinalState,
    now: tokio::time::Instant,
    rendered_html: &str,
    throttle: Duration,
    result: ProgressEditResult,
) {
    match result {
        ProgressEditResult::Applied => final_state.mark_final_sent(now, rendered_html),
        ProgressEditResult::RateLimited(wait) => {
            final_state.defer_final_until(now + wait.max(throttle))
        },
        ProgressEditResult::Failed => final_state.defer_final_until(now + throttle),
    }
}

fn escaped_recent_tail(text: &str, max_len: usize) -> (String, bool) {
    let total_chars = text.chars().count();
    let mut escaped_len = 0usize;
    let mut reversed = Vec::new();

    for ch in text.chars().rev() {
        let ch_len = escaped_char_len(ch);
        if escaped_len + ch_len > max_len {
            break;
        }
        escaped_len += ch_len;
        reversed.push(ch);
    }

    let truncated = reversed.len() < total_chars;
    reversed.reverse();
    let raw: String = reversed.into_iter().collect();
    (markdown::escape_html(&raw), truncated)
}

fn escaped_char_len(ch: char) -> usize {
    match ch {
        '&' => "&amp;".len(),
        '<' => "&lt;".len(),
        '>' => "&gt;".len(),
        _ => ch.len_utf8(),
    }
}

fn trim_to_recent_chars(text: &mut String, max_chars: usize) -> bool {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return false;
    }

    let remove_chars = char_count - max_chars;
    let Some((byte_start, _)) = text.char_indices().nth(remove_chars) else {
        text.clear();
        return true;
    };
    let tail = text.split_off(byte_start);
    *text = tail;
    true
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

        let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;
        let mut progress_message_id: Option<MessageId> = None;

        let mut progress =
            StreamProgressState::new(stream_cfg.min_initial_chars, stream_cfg.progress_max_chars);
        let mut final_state = StreamFinalState::new(stream_cfg.min_initial_chars);
        let throttle =
            Duration::from_millis(stream_cfg.edit_throttle_ms).max(MIN_PROGRESS_FLUSH_INTERVAL);
        let mut typing_interval = tokio::time::interval(TYPING_REFRESH_INTERVAL);
        typing_interval.tick().await;
        let mut flush_interval = tokio::time::interval(throttle);
        flush_interval.tick().await;

        loop {
            tokio::select! {
                event = stream.recv() => {
                    let Some(event) = event else {
                        self.finish_final_stream_message(
                            &bot,
                            account_id,
                            to,
                            chat_id,
                            thread_id,
                            rp.as_ref(),
                            stream_cfg,
                            &mut progress_message_id,
                            &mut final_state,
                            throttle,
                        )
                        .await?;
                        break;
                    };
                    match event {
                        StreamEvent::ProgressDelta(delta) => {
                            if final_state.message_id.is_some() || !final_state.accumulated.is_empty() {
                                let draft_message_id = final_state.message_id.take();
                                final_state.reset();
                                progress.replace_text(&delta);
                                if let Some(msg_id) = draft_message_id {
                                    progress_message_id = Some(msg_id);
                                    let now = tokio::time::Instant::now();
                                    let display = progress.current_progress_html();
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
                                    continue;
                                }
                            } else {
                                progress.push_delta(&delta);
                            }
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
                        StreamEvent::Delta(delta) => {
                            final_state.push_delta(&delta);
                            let now = tokio::time::Instant::now();
                            if final_state.message_id.is_none() {
                                if final_state.should_send_initial_final() {
                                    let display = final_state.current_final_html();
                                    self.flush_final_stream_message(
                                        &bot,
                                        account_id,
                                        to,
                                        chat_id,
                                        thread_id,
                                        rp.as_ref(),
                                        stream_cfg,
                                        &mut progress_message_id,
                                        &mut final_state,
                                        &display,
                                        now,
                                        throttle,
                                        false,
                                    )
                                    .await?;
                                }
                                continue;
                            }

                            if final_state.should_flush_final(now, throttle) {
                                let display = final_state.current_final_html();
                                if final_state.rendered_html_changed(&display) {
                                    self.flush_final_stream_message(
                                        &bot,
                                        account_id,
                                        to,
                                        chat_id,
                                        thread_id,
                                        rp.as_ref(),
                                        stream_cfg,
                                        &mut progress_message_id,
                                        &mut final_state,
                                        &display,
                                        now,
                                        throttle,
                                        false,
                                    )
                                    .await?;
                                } else {
                                    final_state.mark_final_observed(now, &display);
                                }
                            }
                        },
                        StreamEvent::Done => {
                            self.finish_final_stream_message(
                                &bot,
                                account_id,
                                to,
                                chat_id,
                                thread_id,
                                rp.as_ref(),
                                stream_cfg,
                                &mut progress_message_id,
                                &mut final_state,
                                throttle,
                            )
                            .await?;
                            break;
                        },
                        StreamEvent::Error(e) => {
                            debug!("stream error: {e}");
                            break;
                        },
                    }
                }
                _ = typing_interval.tick() => {
                    let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;
                }
                _ = flush_interval.tick() => {
                    let now = tokio::time::Instant::now();
                    if final_state.should_flush_final(now, throttle) {
                        let display = final_state.current_final_html();
                        if final_state.rendered_html_changed(&display) {
                            self.flush_final_stream_message(
                                &bot,
                                account_id,
                                to,
                                chat_id,
                                thread_id,
                                rp.as_ref(),
                                stream_cfg,
                                &mut progress_message_id,
                                &mut final_state,
                                &display,
                                now,
                                throttle,
                                false,
                            )
                            .await?;
                        } else {
                            final_state.mark_final_observed(now, &display);
                        }
                    } else if progress.should_flush_progress(now, throttle) {
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

    async fn receives_progress_deltas(&self, account_id: &str) -> bool {
        self.is_stream_enabled(account_id).await
    }
}
