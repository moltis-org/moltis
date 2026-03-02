use {
    async_trait::async_trait,
    base64::Engine,
    std::{future::Future, time::Duration},
    teloxide::{
        ApiError, RequestError,
        payloads::{SendLocationSetters, SendMessageSetters, SendVenueSetters},
        prelude::*,
        types::{ChatAction, ChatId, InputFile, MessageId, ParseMode, ReplyParameters},
    },
    tracing::{debug, info, warn},
};

use {
    moltis_channels::{
        Error as ChannelError, Result,
        plugin::{ChannelOutbound, ChannelStreamOutbound, StreamEvent, StreamReceiver},
    },
    moltis_common::types::ReplyPayload,
};

use crate::{
    config::StreamMode,
    markdown::{self, TELEGRAM_MAX_MESSAGE_LEN},
    state::AccountStateMap,
};

/// Outbound message sender for Telegram.
pub struct TelegramOutbound {
    pub(crate) accounts: AccountStateMap,
}

const TELEGRAM_RETRY_AFTER_MAX_RETRIES: usize = 4;

/// How often to re-send the typing indicator while waiting for stream events.
/// Telegram typing indicators expire after ~5 s; refresh well before that.
const TYPING_REFRESH_INTERVAL: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Copy)]
struct StreamSendConfig {
    edit_throttle_ms: u64,
    notify_on_complete: bool,
    min_initial_chars: usize,
}

impl Default for StreamSendConfig {
    fn default() -> Self {
        Self {
            edit_throttle_ms: 300,
            notify_on_complete: false,
            min_initial_chars: 30,
        }
    }
}

impl TelegramOutbound {
    fn get_bot(&self, account_id: &str) -> Result<Bot> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .map(|s| s.bot.clone())
            .ok_or_else(|| ChannelError::unknown_account(account_id))
    }

    /// Build reply parameters only when `reply_to_message` is enabled for this account.
    fn reply_params(&self, account_id: &str, reply_to: Option<&str>) -> Option<ReplyParameters> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        let enabled = accounts
            .get(account_id)
            .is_some_and(|s| s.config.reply_to_message);
        if enabled {
            parse_reply_params(reply_to)
        } else {
            None
        }
    }

    fn stream_send_config(&self, account_id: &str) -> StreamSendConfig {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .map(|s| StreamSendConfig {
                edit_throttle_ms: s.config.edit_throttle_ms,
                notify_on_complete: s.config.stream_notify_on_complete,
                min_initial_chars: s.config.stream_min_initial_chars,
            })
            .unwrap_or_default()
    }

    async fn send_chunk_with_fallback(
        &self,
        bot: &Bot,
        account_id: &str,
        to: &str,
        chat_id: ChatId,
        chunk: &str,
        reply_params: Option<&ReplyParameters>,
        silent: bool,
    ) -> Result<MessageId> {
        match self
            .run_telegram_request_with_retry(account_id, to, "send message (html)", || {
                let mut html_req = bot.send_message(chat_id, chunk).parse_mode(ParseMode::Html);
                if silent {
                    html_req = html_req.disable_notification(true);
                }
                if let Some(rp) = reply_params {
                    html_req = html_req.reply_parameters(rp.clone());
                }
                async move { html_req.await }
            })
            .await
        {
            Ok(message) => Ok(message.id),
            Err(e) => {
                warn!(
                    account_id,
                    chat_id = to,
                    error = %e,
                    "telegram HTML send failed, retrying as plain text"
                );
                let message = self
                    .run_telegram_request_with_retry(account_id, to, "send message (plain)", || {
                        let mut plain_req = bot.send_message(chat_id, chunk);
                        if silent {
                            plain_req = plain_req.disable_notification(true);
                        }
                        if let Some(rp) = reply_params {
                            plain_req = plain_req.reply_parameters(rp.clone());
                        }
                        async move { plain_req.await }
                    })
                    .await
                    .channel_context("send message (plain)")?;
                Ok(message.id)
            },
        }
    }

    async fn edit_chunk_with_fallback(
        &self,
        bot: &Bot,
        account_id: &str,
        to: &str,
        chat_id: ChatId,
        message_id: MessageId,
        chunk: &str,
    ) -> Result<()> {
        match self
            .run_telegram_request_with_retry(account_id, to, "edit message (html)", || {
                let html_req = bot
                    .edit_message_text(chat_id, message_id, chunk)
                    .parse_mode(ParseMode::Html);
                async move { html_req.await }
            })
            .await
        {
            Ok(_) => Ok(()),
            Err(e) => {
                if is_message_not_modified_error(&e) {
                    return Ok(());
                }
                warn!(
                    account_id,
                    chat_id = to,
                    error = %e,
                    "telegram HTML edit failed, retrying as plain text"
                );
                match self
                    .run_telegram_request_with_retry(account_id, to, "edit message (plain)", || {
                        let plain_req = bot.edit_message_text(chat_id, message_id, chunk);
                        async move { plain_req.await }
                    })
                    .await
                {
                    Ok(_) => Ok(()),
                    Err(plain_err) => {
                        if is_message_not_modified_error(&plain_err) {
                            Ok(())
                        } else {
                            Err(ChannelError::external("edit message (plain)", plain_err))
                        }
                    },
                }
            },
        }
    }

    async fn run_telegram_request_with_retry<T, F, Fut>(
        &self,
        account_id: &str,
        to: &str,
        operation: &'static str,
        mut request: F,
    ) -> std::result::Result<T, RequestError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = std::result::Result<T, RequestError>>,
    {
        let mut retries = 0usize;

        loop {
            match request().await {
                Ok(value) => return Ok(value),
                Err(err) => {
                    let Some(wait) = retry_after_duration(&err) else {
                        return Err(err);
                    };

                    if retries >= TELEGRAM_RETRY_AFTER_MAX_RETRIES {
                        warn!(
                            account_id,
                            chat_id = to,
                            operation,
                            retries,
                            max_retries = TELEGRAM_RETRY_AFTER_MAX_RETRIES,
                            retry_after_secs = wait.as_secs(),
                            "telegram rate limit persisted after retries"
                        );
                        return Err(err);
                    }

                    retries += 1;
                    warn!(
                        account_id,
                        chat_id = to,
                        operation,
                        retries,
                        max_retries = TELEGRAM_RETRY_AFTER_MAX_RETRIES,
                        retry_after_secs = wait.as_secs(),
                        "telegram rate limited, waiting before retry"
                    );
                    tokio::time::sleep(wait).await;
                },
            }
        }
    }
}

/// Parse a platform message ID string into Telegram `ReplyParameters`.
/// Returns `None` if the string is not a valid i32 (Telegram message IDs are i32).
fn parse_reply_params(reply_to: Option<&str>) -> Option<ReplyParameters> {
    reply_to
        .and_then(|id| id.parse::<i32>().ok())
        .map(|id| ReplyParameters::new(MessageId(id)).allow_sending_without_reply())
}

fn retry_after_duration(error: &RequestError) -> Option<Duration> {
    match error {
        RequestError::RetryAfter(wait) => Some(wait.duration()),
        _ => None,
    }
}

fn is_message_not_modified_error(error: &RequestError) -> bool {
    matches!(error, RequestError::Api(ApiError::MessageNotModified))
}

trait RequestResultExt<T> {
    fn channel_context(self, context: &'static str) -> Result<T>;
}

impl<T> RequestResultExt<T> for std::result::Result<T, RequestError> {
    fn channel_context(self, context: &'static str) -> Result<T> {
        self.map_err(|e| ChannelError::external(context, e))
    }
}

fn has_reached_stream_min_initial_chars(accumulated: &str, min_initial_chars: usize) -> bool {
    accumulated.chars().count() >= min_initial_chars
}

fn should_send_stream_completion_notification(
    notify_on_complete: bool,
    has_streamed_text: bool,
    sent_non_silent_completion_chunks: bool,
) -> bool {
    notify_on_complete && has_streamed_text && !sent_non_silent_completion_chunks
}

#[async_trait]
impl ChannelOutbound for TelegramOutbound {
    async fn send_text(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> Result<()> {
        let bot = self.get_bot(account_id)?;
        let chat_id = ChatId(to.parse::<i64>()?);
        let rp = self.reply_params(account_id, reply_to);

        // Send typing indicator
        let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;

        let chunks = markdown::chunk_markdown_html(text, TELEGRAM_MAX_MESSAGE_LEN);
        info!(
            account_id,
            chat_id = to,
            reply_to = ?reply_to,
            text_len = text.len(),
            chunk_count = chunks.len(),
            "telegram outbound text send start"
        );

        for chunk in chunks.iter() {
            let reply_params = rp.as_ref();
            self.send_chunk_with_fallback(
                &bot,
                account_id,
                to,
                chat_id,
                chunk,
                reply_params,
                false,
            )
            .await?;
        }

        info!(
            account_id,
            chat_id = to,
            reply_to = ?reply_to,
            text_len = text.len(),
            chunk_count = chunks.len(),
            "telegram outbound text sent"
        );
        Ok(())
    }

    async fn send_text_with_suffix(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        suffix_html: &str,
        reply_to: Option<&str>,
    ) -> Result<()> {
        let bot = self.get_bot(account_id)?;
        let chat_id = ChatId(to.parse::<i64>()?);
        let rp = self.reply_params(account_id, reply_to);

        // Send typing indicator
        let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;

        // Append the pre-formatted suffix (e.g. activity logbook) to the last chunk.
        let chunks = markdown::chunk_markdown_html(text, TELEGRAM_MAX_MESSAGE_LEN);
        let last_idx = chunks.len().saturating_sub(1);
        info!(
            account_id,
            chat_id = to,
            reply_to = ?reply_to,
            text_len = text.len(),
            suffix_len = suffix_html.len(),
            chunk_count = chunks.len(),
            "telegram outbound text+suffix send start"
        );

        for (i, chunk) in chunks.iter().enumerate() {
            let content = if i == last_idx {
                // Append suffix to the last chunk. If it would exceed the limit,
                // the suffix becomes a separate final message.
                let combined = format!("{chunk}\n\n{suffix_html}");
                if combined.len() <= TELEGRAM_MAX_MESSAGE_LEN {
                    combined
                } else {
                    // Send this chunk first, then the suffix as a separate message.
                    self.send_chunk_with_fallback(
                        &bot,
                        account_id,
                        to,
                        chat_id,
                        chunk,
                        rp.as_ref(),
                        false,
                    )
                    .await?;
                    // Send suffix as the final message (no reply threading).
                    self.send_chunk_with_fallback(
                        &bot,
                        account_id,
                        to,
                        chat_id,
                        suffix_html,
                        rp.as_ref(),
                        true,
                    )
                    .await?;
                    info!(
                        account_id,
                        chat_id = to,
                        reply_to = ?reply_to,
                        text_len = text.len(),
                        suffix_len = suffix_html.len(),
                        chunk_count = chunks.len(),
                        "telegram outbound text+suffix sent (separate suffix message)"
                    );
                    return Ok(());
                }
            } else {
                chunk.clone()
            };
            self.send_chunk_with_fallback(
                &bot,
                account_id,
                to,
                chat_id,
                &content,
                rp.as_ref(),
                false,
            )
            .await?;
        }

        info!(
            account_id,
            chat_id = to,
            reply_to = ?reply_to,
            text_len = text.len(),
            suffix_len = suffix_html.len(),
            chunk_count = chunks.len(),
            "telegram outbound text+suffix sent"
        );
        Ok(())
    }

    async fn send_html(
        &self,
        account_id: &str,
        to: &str,
        html: &str,
        reply_to: Option<&str>,
    ) -> Result<()> {
        let bot = self.get_bot(account_id)?;
        let chat_id = ChatId(to.parse::<i64>()?);
        let rp = self.reply_params(account_id, reply_to);

        // Send raw HTML chunks without markdown conversion.
        let chunks = markdown::chunk_message(html, TELEGRAM_MAX_MESSAGE_LEN);
        for chunk in &chunks {
            self.send_chunk_with_fallback(&bot, account_id, to, chat_id, chunk, rp.as_ref(), false)
                .await?;
        }
        Ok(())
    }

    async fn send_typing(&self, account_id: &str, to: &str) -> Result<()> {
        let bot = self.get_bot(account_id)?;
        let chat_id = ChatId(to.parse::<i64>()?);
        let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;
        Ok(())
    }

    async fn send_text_silent(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> Result<()> {
        let bot = self.get_bot(account_id)?;
        let chat_id = ChatId(to.parse::<i64>()?);
        let rp = self.reply_params(account_id, reply_to);

        let chunks = markdown::chunk_markdown_html(text, TELEGRAM_MAX_MESSAGE_LEN);
        info!(
            account_id,
            chat_id = to,
            reply_to = ?reply_to,
            text_len = text.len(),
            chunk_count = chunks.len(),
            "telegram outbound silent text send start"
        );

        for chunk in chunks.iter() {
            self.send_chunk_with_fallback(&bot, account_id, to, chat_id, chunk, rp.as_ref(), true)
                .await?;
        }

        info!(
            account_id,
            chat_id = to,
            reply_to = ?reply_to,
            text_len = text.len(),
            chunk_count = chunks.len(),
            "telegram outbound silent text sent"
        );
        Ok(())
    }

    async fn send_media(
        &self,
        account_id: &str,
        to: &str,
        payload: &ReplyPayload,
        reply_to: Option<&str>,
    ) -> Result<()> {
        let bot = self.get_bot(account_id)?;
        let chat_id = ChatId(to.parse::<i64>()?);
        let rp = self.reply_params(account_id, reply_to);
        let media_mime = payload
            .media
            .as_ref()
            .map(|m| m.mime_type.as_str())
            .unwrap_or("none");
        info!(
            account_id,
            chat_id = to,
            reply_to = ?reply_to,
            has_media = payload.media.is_some(),
            media_mime,
            caption_len = payload.text.len(),
            "telegram outbound media send start"
        );

        if let Some(ref media) = payload.media {
            // Handle base64 data URIs (e.g., "data:image/png;base64,...")
            if media.url.starts_with("data:") {
                // Parse data URI: data:<mime>;base64,<data>
                let Some(comma_pos) = media.url.find(',') else {
                    return Err(ChannelError::invalid_input(
                        "invalid data URI: no comma separator",
                    ));
                };
                let base64_data = &media.url[comma_pos + 1..];
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(base64_data)
                    .map_err(|e| {
                        ChannelError::invalid_input(format!("failed to decode base64: {e}"))
                    })?;

                debug!(
                    bytes = bytes.len(),
                    mime_type = %media.mime_type,
                    "sending base64 media to telegram"
                );

                // Determine file extension
                let ext = match media.mime_type.as_str() {
                    "image/png" => "png",
                    "image/jpeg" | "image/jpg" => "jpg",
                    "image/gif" => "gif",
                    "image/webp" => "webp",
                    _ => "bin",
                };
                let filename = format!("screenshot.{ext}");

                // For images, try as photo first, fall back to document on dimension errors
                if media.mime_type.starts_with("image/") {
                    let input = InputFile::memory(bytes.clone()).file_name(filename.clone());
                    let mut req = bot.send_photo(chat_id, input);
                    if !payload.text.is_empty() {
                        req = req.caption(&payload.text);
                    }
                    if let Some(ref rp) = rp {
                        req = req.reply_parameters(rp.clone());
                    }

                    match req.await {
                        Ok(_) => {
                            info!(
                                account_id,
                                chat_id = to,
                                reply_to = ?reply_to,
                                media_mime = %media.mime_type,
                                caption_len = payload.text.len(),
                                "telegram outbound media sent as photo"
                            );
                            return Ok(());
                        },
                        Err(e) => {
                            let err_str = e.to_string();
                            // Retry as document if photo dimensions are invalid
                            if err_str.contains("PHOTO_INVALID_DIMENSIONS")
                                || err_str.contains("PHOTO_SAVE_FILE_INVALID")
                            {
                                debug!(
                                    error = %err_str,
                                    "photo rejected, retrying as document"
                                );
                                let input = InputFile::memory(bytes).file_name(filename);
                                let mut req = bot.send_document(chat_id, input);
                                if !payload.text.is_empty() {
                                    req = req.caption(&payload.text);
                                }
                                req.await.channel_context("send document fallback")?;
                                info!(
                                    account_id,
                                    chat_id = to,
                                    reply_to = ?reply_to,
                                    media_mime = %media.mime_type,
                                    caption_len = payload.text.len(),
                                    "telegram outbound media sent as document fallback"
                                );
                                return Ok(());
                            }
                            return Err(ChannelError::external("send media photo", e));
                        },
                    }
                }

                // Non-image types: send as document
                if media.mime_type == "audio/ogg" {
                    let input = InputFile::memory(bytes).file_name("voice.ogg");
                    let mut req = bot.send_voice(chat_id, input);
                    if !payload.text.is_empty() {
                        req = req.caption(&payload.text);
                    }
                    req.await.channel_context("send voice media")?;
                    info!(
                        account_id,
                        chat_id = to,
                        reply_to = ?reply_to,
                        media_mime = %media.mime_type,
                        caption_len = payload.text.len(),
                        "telegram outbound media sent as voice"
                    );
                } else if media.mime_type.starts_with("audio/") {
                    let input = InputFile::memory(bytes).file_name("audio.mp3");
                    let mut req = bot.send_audio(chat_id, input);
                    if !payload.text.is_empty() {
                        req = req.caption(&payload.text);
                    }
                    req.await.channel_context("send audio media")?;
                    info!(
                        account_id,
                        chat_id = to,
                        reply_to = ?reply_to,
                        media_mime = %media.mime_type,
                        caption_len = payload.text.len(),
                        "telegram outbound media sent as audio"
                    );
                } else {
                    let input = InputFile::memory(bytes).file_name(filename);
                    let mut req = bot.send_document(chat_id, input);
                    if !payload.text.is_empty() {
                        req = req.caption(&payload.text);
                    }
                    req.await.channel_context("send document media")?;
                    info!(
                        account_id,
                        chat_id = to,
                        reply_to = ?reply_to,
                        media_mime = %media.mime_type,
                        caption_len = payload.text.len(),
                        "telegram outbound media sent as document"
                    );
                }
            } else {
                // URL-based media
                let url = media.url.parse().map_err(|e| {
                    ChannelError::invalid_input(format!("invalid media URL '{}': {e}", media.url))
                })?;
                let input = InputFile::url(url);

                match media.mime_type.as_str() {
                    t if t.starts_with("image/") => {
                        let mut req = bot.send_photo(chat_id, input);
                        if !payload.text.is_empty() {
                            req = req.caption(&payload.text);
                        }
                        req.await.channel_context("send URL photo media")?;
                        info!(
                            account_id,
                            chat_id = to,
                            reply_to = ?reply_to,
                            media_mime = %media.mime_type,
                            caption_len = payload.text.len(),
                            "telegram outbound URL media sent as photo"
                        );
                    },
                    "audio/ogg" => {
                        let mut req = bot.send_voice(chat_id, input);
                        if !payload.text.is_empty() {
                            req = req.caption(&payload.text);
                        }
                        req.await.channel_context("send URL voice media")?;
                        info!(
                            account_id,
                            chat_id = to,
                            reply_to = ?reply_to,
                            media_mime = %media.mime_type,
                            caption_len = payload.text.len(),
                            "telegram outbound URL media sent as voice"
                        );
                    },
                    t if t.starts_with("audio/") => {
                        let mut req = bot.send_audio(chat_id, input);
                        if !payload.text.is_empty() {
                            req = req.caption(&payload.text);
                        }
                        req.await.channel_context("send URL audio media")?;
                        info!(
                            account_id,
                            chat_id = to,
                            reply_to = ?reply_to,
                            media_mime = %media.mime_type,
                            caption_len = payload.text.len(),
                            "telegram outbound URL media sent as audio"
                        );
                    },
                    _ => {
                        let mut req = bot.send_document(chat_id, input);
                        if !payload.text.is_empty() {
                            req = req.caption(&payload.text);
                        }
                        req.await.channel_context("send URL document media")?;
                        info!(
                            account_id,
                            chat_id = to,
                            reply_to = ?reply_to,
                            media_mime = %media.mime_type,
                            caption_len = payload.text.len(),
                            "telegram outbound URL media sent as document"
                        );
                    },
                }
            }
        } else if !payload.text.is_empty() {
            self.send_text(account_id, to, &payload.text, reply_to)
                .await?;
        }

        Ok(())
    }

    async fn send_location(
        &self,
        account_id: &str,
        to: &str,
        latitude: f64,
        longitude: f64,
        title: Option<&str>,
        reply_to: Option<&str>,
    ) -> Result<()> {
        let bot = self.get_bot(account_id)?;
        let chat_id = ChatId(to.parse::<i64>()?);
        let rp = self.reply_params(account_id, reply_to);
        info!(
            account_id,
            chat_id = to,
            reply_to = ?reply_to,
            latitude,
            longitude,
            has_title = title.is_some(),
            "telegram outbound location send start"
        );

        if let Some(name) = title {
            // Venue shows the place name in the chat bubble.
            let address = format!("{latitude:.6}, {longitude:.6}");
            let mut req = bot.send_venue(chat_id, latitude, longitude, name, address);
            if let Some(ref rp) = rp {
                req = req.reply_parameters(rp.clone());
            }
            req.await.channel_context("send venue")?;
        } else {
            let mut req = bot.send_location(chat_id, latitude, longitude);
            if let Some(ref rp) = rp {
                req = req.reply_parameters(rp.clone());
            }
            req.await.channel_context("send location")?;
        }

        info!(
            account_id,
            chat_id = to,
            reply_to = ?reply_to,
            latitude,
            longitude,
            has_title = title.is_some(),
            "telegram outbound location sent"
        );
        Ok(())
    }
}

impl TelegramOutbound {
    /// Send a `ReplyPayload` — dispatches to text or media.
    pub async fn send_reply(&self, bot: &Bot, to: &str, payload: &ReplyPayload) -> Result<()> {
        let chat_id = ChatId(to.parse::<i64>()?);

        // Send typing indicator
        let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;

        if payload.media.is_some() {
            // Use the media path — but we need account_id, which we don't have here.
            // For direct bot usage, delegate to send_text for now.
            let chunks = markdown::chunk_markdown_html(&payload.text, TELEGRAM_MAX_MESSAGE_LEN);
            for chunk in chunks {
                bot.send_message(chat_id, &chunk)
                    .parse_mode(ParseMode::Html)
                    .await
                    .channel_context("send reply chunk (media)")?;
            }
        } else if !payload.text.is_empty() {
            let chunks = markdown::chunk_markdown_html(&payload.text, TELEGRAM_MAX_MESSAGE_LEN);
            for chunk in chunks {
                bot.send_message(chat_id, &chunk)
                    .parse_mode(ParseMode::Html)
                    .await
                    .channel_context("send reply chunk")?;
            }
        }

        Ok(())
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
        let chat_id = ChatId(to.parse::<i64>()?);
        let rp = self.reply_params(account_id, reply_to);
        let stream_cfg = self.stream_send_config(account_id);

        // Send typing indicator
        let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;
        let mut stream_message_id: Option<MessageId> = None;

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

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {
        super::*,
        std::{collections::HashMap, sync::Arc, time::Duration},
    };

    #[tokio::test]
    async fn send_location_unknown_account_returns_error() {
        let accounts: AccountStateMap = Arc::new(std::sync::RwLock::new(HashMap::new()));
        let outbound = TelegramOutbound {
            accounts: Arc::clone(&accounts),
        };

        let result = outbound
            .send_location("nonexistent", "12345", 48.8566, 2.3522, Some("Paris"), None)
            .await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("unknown channel account"),
            "should report unknown channel account"
        );
    }

    #[test]
    fn retry_after_duration_extracts_wait() {
        let err = RequestError::RetryAfter(teloxide::types::Seconds::from_seconds(42));
        assert_eq!(retry_after_duration(&err), Some(Duration::from_secs(42)));
    }

    #[test]
    fn retry_after_duration_ignores_other_errors() {
        let err = RequestError::Io(std::io::Error::other("boom"));
        assert_eq!(retry_after_duration(&err), None);
    }

    #[test]
    fn is_message_not_modified_error_detects_variant() {
        let err = RequestError::Api(ApiError::MessageNotModified);
        assert!(is_message_not_modified_error(&err));
    }

    #[test]
    fn is_message_not_modified_error_ignores_other_errors() {
        let err = RequestError::Io(std::io::Error::other("boom"));
        assert!(!is_message_not_modified_error(&err));
    }

    #[test]
    fn stream_min_initial_chars_uses_character_count() {
        assert!(has_reached_stream_min_initial_chars("hello", 5));
        assert!(has_reached_stream_min_initial_chars("🙂🙂🙂", 3));
        assert!(!has_reached_stream_min_initial_chars("🙂🙂🙂", 4));
    }

    #[test]
    fn stream_completion_notification_requires_opt_in() {
        assert!(!should_send_stream_completion_notification(
            false, true, false
        ));
    }

    #[test]
    fn stream_completion_notification_skips_when_no_text() {
        assert!(!should_send_stream_completion_notification(
            true, false, false
        ));
    }

    #[test]
    fn stream_completion_notification_skips_when_already_notified_by_chunks() {
        assert!(!should_send_stream_completion_notification(
            true, true, true
        ));
    }

    #[test]
    fn stream_completion_notification_enabled_when_needed() {
        assert!(should_send_stream_completion_notification(
            true, true, false
        ));
    }
}
