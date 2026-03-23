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
                let plain_chunk = telegram_html_to_plain_text(chunk);
                warn!(
                    account_id,
                    chat_id = to,
                    error = %e,
                    "telegram HTML send failed, retrying as plain text"
                );
                let message = self
                    .run_telegram_request_with_retry(account_id, to, "send message (plain)", || {
                        let mut plain_req = bot.send_message(chat_id, &plain_chunk);
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
                let plain_chunk = telegram_html_to_plain_text(chunk);
                warn!(
                    account_id,
                    chat_id = to,
                    error = %e,
                    "telegram HTML edit failed, retrying as plain text"
                );
                match self
                    .run_telegram_request_with_retry(account_id, to, "edit message (plain)", || {
                        let plain_req = bot.edit_message_text(chat_id, message_id, &plain_chunk);
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

fn telegram_html_to_plain_text(html: &str) -> String {
    let mut plain = String::with_capacity(html.len());
    let mut remaining = html;

    while let Some(ch) = remaining.chars().next() {
        if ch == '<' {
            if let Some((tag_name, consumed_len)) = consume_plain_text_html_tag(remaining) {
                if is_plain_text_line_break_tag(&tag_name) && !plain.ends_with('\n') {
                    plain.push('\n');
                }
                remaining = &remaining[consumed_len..];
                continue;
            }
        } else if ch == '&'
            && let Some((decoded, consumed_len)) = consume_html_entity(remaining)
        {
            plain.push_str(&decoded);
            remaining = &remaining[consumed_len..];
            continue;
        }

        plain.push(ch);
        remaining = &remaining[ch.len_utf8()..];
    }

    plain.trim_matches('\n').to_string()
}

fn consume_plain_text_html_tag(input: &str) -> Option<(String, usize)> {
    let bytes = input.as_bytes();
    if bytes.first().copied()? != b'<' {
        return None;
    }

    let mut index = 1usize;
    if bytes.get(index).copied() == Some(b'/') {
        index += 1;
    }

    let name_start = index;
    let first = bytes.get(index).copied()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }

    while let Some(next) = bytes.get(index).copied() {
        if next.is_ascii_alphanumeric() || next == b'-' {
            index += 1;
            continue;
        }
        break;
    }

    let tag_name = input[name_start..index].to_ascii_lowercase();
    if !is_plain_text_html_tag_name(&tag_name) {
        return None;
    }

    let mut quote = None;
    while let Some(next) = bytes.get(index).copied() {
        match quote {
            Some(delimiter) if next == delimiter => quote = None,
            Some(_) => {},
            None if next == b'\'' || next == b'"' => quote = Some(next),
            None if next == b'>' => return Some((tag_name, index + 1)),
            None => {},
        }
        index += 1;
    }

    None
}

fn consume_html_entity(input: &str) -> Option<(String, usize)> {
    if !input.starts_with('&') {
        return None;
    }

    let mut entity = String::new();
    for (index, ch) in input.char_indices() {
        entity.push(ch);
        let consumed_len = index + ch.len_utf8();
        if ch == ';' {
            return decode_html_entity(&entity).map(|decoded| (decoded, consumed_len));
        }
        if entity.len() > 12 {
            return None;
        }
    }

    None
}

fn is_plain_text_html_tag_name(tag_name: &str) -> bool {
    matches!(
        tag_name,
        "a" | "b"
            | "blockquote"
            | "br"
            | "code"
            | "del"
            | "div"
            | "em"
            | "i"
            | "ins"
            | "li"
            | "p"
            | "pre"
            | "s"
            | "span"
            | "strike"
            | "strong"
            | "tg-emoji"
            | "tg-spoiler"
            | "u"
    )
}

fn is_plain_text_line_break_tag(tag_name: &str) -> bool {
    matches!(tag_name, "blockquote" | "br" | "div" | "li" | "p" | "pre")
}

fn decode_html_entity(entity: &str) -> Option<String> {
    match entity {
        "&amp;" => Some("&".to_string()),
        "&lt;" => Some("<".to_string()),
        "&gt;" => Some(">".to_string()),
        "&quot;" => Some("\"".to_string()),
        "&apos;" | "&#39;" => Some("'".to_string()),
        "&nbsp;" | "&#160;" => Some(" ".to_string()),
        _ => decode_numeric_html_entity(entity),
    }
}

fn decode_numeric_html_entity(entity: &str) -> Option<String> {
    let value = entity
        .strip_prefix("&#x")
        .or_else(|| entity.strip_prefix("&#X"))
        .and_then(|hex| hex.strip_suffix(';'))
        .and_then(|hex| u32::from_str_radix(hex, 16).ok())
        .or_else(|| {
            entity
                .strip_prefix("&#")
                .and_then(|decimal| decimal.strip_suffix(';'))
                .and_then(|decimal| decimal.parse::<u32>().ok())
        })?;

    char::from_u32(value).map(|ch| ch.to_string())
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

                // Use the original filename when provided, otherwise derive
                // from MIME type.
                let filename = media.filename.clone().unwrap_or_else(|| {
                    let ext = moltis_media::mime::extension_for_mime(&media.mime_type);
                    format!("file.{ext}")
                });

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
        axum::{Json, Router, extract::State, http::StatusCode, routing::post},
        moltis_channels::gating::DmPolicy,
        secrecy::Secret,
        serde::{Deserialize, Serialize},
        std::{
            collections::HashMap,
            sync::{Arc, Mutex},
            time::Duration,
        },
        tokio::sync::oneshot,
        tokio_util::sync::CancellationToken,
    };

    use crate::{config::TelegramAccountConfig, otp::OtpState, state::AccountState};

    #[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
    struct SendMessageRequest {
        chat_id: i64,
        text: String,
        #[serde(default)]
        parse_mode: Option<String>,
    }

    #[derive(Debug, Serialize)]
    struct TelegramApiResponse {
        ok: bool,
        result: TelegramMessageResult,
    }

    #[derive(Debug, Serialize)]
    struct TelegramMessageResult {
        message_id: i64,
        date: i64,
        chat: TelegramChat,
        text: String,
    }

    #[derive(Debug, Serialize)]
    struct TelegramChat {
        id: i64,
        #[serde(rename = "type")]
        chat_type: String,
    }

    #[derive(Clone)]
    struct MockTelegramApi {
        requests: Arc<Mutex<Vec<SendMessageRequest>>>,
    }

    async fn send_message_handler(
        State(state): State<MockTelegramApi>,
        Json(body): Json<SendMessageRequest>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        state
            .requests
            .lock()
            .expect("lock requests")
            .push(body.clone());

        if body.parse_mode.as_deref() == Some("HTML") {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "ok": false,
                    "error_code": 400,
                    "description": "Bad Request: can't parse entities: unsupported start tag"
                })),
            );
        }

        (
            StatusCode::OK,
            Json(serde_json::json!(TelegramApiResponse {
                ok: true,
                result: TelegramMessageResult {
                    message_id: 1,
                    date: 0,
                    chat: TelegramChat {
                        id: body.chat_id,
                        chat_type: "private".to_string(),
                    },
                    text: body.text,
                },
            })),
        )
    }

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
    fn telegram_html_to_plain_text_strips_tags_and_decodes_entities() {
        let plain = telegram_html_to_plain_text(
            "<b>Hello</b> &amp; <i>world</i><br><code>&lt;ok&gt;</code>",
        );

        assert_eq!(plain, "Hello & world\n<ok>");
    }

    #[test]
    fn telegram_html_to_plain_text_decodes_numeric_entities() {
        let plain = telegram_html_to_plain_text("it&#39;s &#x1F642;");

        assert_eq!(plain, "it's 🙂");
    }

    #[test]
    fn telegram_html_to_plain_text_decodes_uppercase_hex_entities() {
        let plain = telegram_html_to_plain_text("smile &#X1F642;");

        assert_eq!(plain, "smile 🙂");
    }

    #[test]
    fn telegram_html_to_plain_text_preserves_non_tag_angle_bracket_text() {
        let plain = telegram_html_to_plain_text("<code>if a < b && c > d</code>");

        assert_eq!(plain, "if a < b && c > d");
    }

    #[test]
    fn telegram_html_to_plain_text_preserves_preformatted_indentation() {
        let plain = telegram_html_to_plain_text("<pre>    indented</pre>");

        assert_eq!(plain, "    indented");
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

    #[tokio::test]
    async fn send_html_fallback_sends_plain_text_without_raw_tags() {
        let recorded_requests = Arc::new(Mutex::new(Vec::<SendMessageRequest>::new()));
        let mock_api = MockTelegramApi {
            requests: Arc::clone(&recorded_requests),
        };
        let app = Router::new()
            .route("/{*path}", post(send_message_handler))
            .with_state(mock_api);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("local addr");
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("serve mock telegram api");
        });

        let api_url = reqwest::Url::parse(&format!("http://{addr}/")).expect("parse api url");
        let bot = Bot::new("test-token").set_api_url(api_url);

        let accounts: AccountStateMap = Arc::new(std::sync::RwLock::new(HashMap::new()));
        let outbound = Arc::new(TelegramOutbound {
            accounts: Arc::clone(&accounts),
        });
        let account_id = "test-account";

        {
            let mut map = accounts.write().expect("accounts write lock");
            map.insert(account_id.to_string(), AccountState {
                bot: bot.clone(),
                bot_username: Some("test_bot".to_string()),
                account_id: account_id.to_string(),
                config: TelegramAccountConfig {
                    token: Secret::new("test-token".to_string()),
                    dm_policy: DmPolicy::Open,
                    ..Default::default()
                },
                outbound: Arc::clone(&outbound),
                cancel: CancellationToken::new(),
                message_log: None,
                event_sink: None,
                otp: Mutex::new(OtpState::new(300)),
            });
        }

        outbound
            .send_html(
                account_id,
                "42",
                "<b>Hello</b> &amp; <i>world</i><br><code>&lt;ok&gt;</code>",
                None,
            )
            .await
            .expect("send html");

        {
            let requests = recorded_requests.lock().expect("requests lock");
            assert_eq!(requests.len(), 2, "expected HTML send plus plain fallback");
            assert_eq!(requests[0].parse_mode.as_deref(), Some("HTML"));
            assert_eq!(
                requests[0].text,
                "<b>Hello</b> &amp; <i>world</i><br><code>&lt;ok&gt;</code>"
            );
            assert_eq!(requests[1].parse_mode, None);
            assert_eq!(requests[1].text, "Hello & world\n<ok>");
        }

        let _ = shutdown_tx.send(());
        server.await.expect("server join");
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
