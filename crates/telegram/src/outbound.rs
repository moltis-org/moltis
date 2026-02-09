use {
    anyhow::Result,
    async_trait::async_trait,
    base64::Engine,
    teloxide::{
        payloads::SendMessageSetters,
        prelude::*,
        types::{ChatAction, ChatId, InputFile, MessageId, ParseMode, ReplyParameters},
    },
    tracing::debug,
};

use {
    moltis_channels::plugin::{
        ChannelOutbound, ChannelStreamOutbound, StreamEvent, StreamReceiver,
    },
    moltis_common::types::ReplyPayload,
};

use crate::{
    markdown::{self, TELEGRAM_MAX_MESSAGE_LEN},
    state::AccountStateMap,
};

/// Outbound message sender for Telegram.
pub struct TelegramOutbound {
    pub(crate) accounts: AccountStateMap,
}

impl TelegramOutbound {
    fn get_bot(&self, account_id: &str) -> Result<teloxide::Bot> {
        let accounts = self.accounts.read().unwrap();
        accounts
            .get(account_id)
            .map(|s| s.bot.clone())
            .ok_or_else(|| anyhow::anyhow!("unknown account: {account_id}"))
    }
}

/// Parse a platform message ID string into Telegram `ReplyParameters`.
/// Returns `None` if the string is not a valid i32 (Telegram message IDs are i32).
fn parse_reply_params(reply_to: Option<&str>) -> Option<ReplyParameters> {
    reply_to
        .and_then(|id| id.parse::<i32>().ok())
        .map(|id| ReplyParameters::new(MessageId(id)).allow_sending_without_reply())
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
        let rp = parse_reply_params(reply_to);

        // Send typing indicator
        let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;

        let html = markdown::markdown_to_telegram_html(text);
        let chunks = markdown::chunk_message(&html, TELEGRAM_MAX_MESSAGE_LEN);

        for (i, chunk) in chunks.iter().enumerate() {
            let mut req = bot.send_message(chat_id, chunk).parse_mode(ParseMode::Html);
            // Thread only the first chunk as a reply to the original message.
            if i == 0
                && let Some(ref rp) = rp
            {
                req = req.reply_parameters(rp.clone());
            }
            req.await?;
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
        let rp = parse_reply_params(reply_to);

        let html = markdown::markdown_to_telegram_html(text);
        let chunks = markdown::chunk_message(&html, TELEGRAM_MAX_MESSAGE_LEN);

        for (i, chunk) in chunks.iter().enumerate() {
            let mut req = bot
                .send_message(chat_id, chunk)
                .parse_mode(ParseMode::Html)
                .disable_notification(true);
            if i == 0
                && let Some(ref rp) = rp
            {
                req = req.reply_parameters(rp.clone());
            }
            req.await?;
        }

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
        let rp = parse_reply_params(reply_to);

        if let Some(ref media) = payload.media {
            // Handle base64 data URIs (e.g., "data:image/png;base64,...")
            if media.url.starts_with("data:") {
                // Parse data URI: data:<mime>;base64,<data>
                let Some(comma_pos) = media.url.find(',') else {
                    anyhow::bail!("invalid data URI: no comma separator");
                };
                let base64_data = &media.url[comma_pos + 1..];
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(base64_data)
                    .map_err(|e| anyhow::anyhow!("failed to decode base64: {e}"))?;

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
                        Ok(_) => return Ok(()),
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
                                req.await?;
                                return Ok(());
                            }
                            return Err(e.into());
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
                    req.await?;
                } else if media.mime_type.starts_with("audio/") {
                    let input = InputFile::memory(bytes).file_name("audio.mp3");
                    let mut req = bot.send_audio(chat_id, input);
                    if !payload.text.is_empty() {
                        req = req.caption(&payload.text);
                    }
                    req.await?;
                } else {
                    let input = InputFile::memory(bytes).file_name(filename);
                    let mut req = bot.send_document(chat_id, input);
                    if !payload.text.is_empty() {
                        req = req.caption(&payload.text);
                    }
                    req.await?;
                }
            } else {
                // URL-based media
                let input = InputFile::url(media.url.parse()?);

                match media.mime_type.as_str() {
                    t if t.starts_with("image/") => {
                        let mut req = bot.send_photo(chat_id, input);
                        if !payload.text.is_empty() {
                            req = req.caption(&payload.text);
                        }
                        req.await?;
                    },
                    "audio/ogg" => {
                        let mut req = bot.send_voice(chat_id, input);
                        if !payload.text.is_empty() {
                            req = req.caption(&payload.text);
                        }
                        req.await?;
                    },
                    t if t.starts_with("audio/") => {
                        let mut req = bot.send_audio(chat_id, input);
                        if !payload.text.is_empty() {
                            req = req.caption(&payload.text);
                        }
                        req.await?;
                    },
                    _ => {
                        let mut req = bot.send_document(chat_id, input);
                        if !payload.text.is_empty() {
                            req = req.caption(&payload.text);
                        }
                        req.await?;
                    },
                }
            }
        } else if !payload.text.is_empty() {
            self.send_text(account_id, to, &payload.text, reply_to)
                .await?;
        }

        Ok(())
    }
}

impl TelegramOutbound {
    /// Send a `ReplyPayload` — dispatches to text or media.
    pub async fn send_reply(
        &self,
        bot: &teloxide::Bot,
        to: &str,
        payload: &ReplyPayload,
    ) -> Result<()> {
        let chat_id = ChatId(to.parse::<i64>()?);

        // Send typing indicator
        let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;

        if payload.media.is_some() {
            // Use the media path — but we need account_id, which we don't have here.
            // For direct bot usage, delegate to send_text for now.
            let html = markdown::markdown_to_telegram_html(&payload.text);
            let chunks = markdown::chunk_message(&html, TELEGRAM_MAX_MESSAGE_LEN);
            for chunk in chunks {
                bot.send_message(chat_id, &chunk)
                    .parse_mode(ParseMode::Html)
                    .await?;
            }
        } else if !payload.text.is_empty() {
            let html = markdown::markdown_to_telegram_html(&payload.text);
            let chunks = markdown::chunk_message(&html, TELEGRAM_MAX_MESSAGE_LEN);
            for chunk in chunks {
                bot.send_message(chat_id, &chunk)
                    .parse_mode(ParseMode::Html)
                    .await?;
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
        mut stream: StreamReceiver,
    ) -> Result<()> {
        let bot = self.get_bot(account_id)?;
        let chat_id = ChatId(to.parse::<i64>()?);

        let throttle_ms = {
            let accounts = self.accounts.read().unwrap();
            accounts
                .get(account_id)
                .map(|s| s.config.edit_throttle_ms)
                .unwrap_or(300)
        };

        // Send typing indicator
        let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;

        // Send initial placeholder
        let placeholder = bot
            .send_message(chat_id, "…")
            .parse_mode(ParseMode::Html)
            .await?;
        let msg_id = placeholder.id;

        let mut accumulated = String::new();
        let mut last_edit = tokio::time::Instant::now();
        let throttle = std::time::Duration::from_millis(throttle_ms);

        while let Some(event) = stream.recv().await {
            match event {
                StreamEvent::Delta(delta) => {
                    accumulated.push_str(&delta);
                    if last_edit.elapsed() >= throttle {
                        let html = markdown::markdown_to_telegram_html(&accumulated);
                        // Telegram rejects edits with identical content; truncate to limit.
                        let display = if html.len() > TELEGRAM_MAX_MESSAGE_LEN {
                            &html[..TELEGRAM_MAX_MESSAGE_LEN]
                        } else {
                            &html
                        };
                        let _ = bot
                            .edit_message_text(chat_id, msg_id, display)
                            .parse_mode(ParseMode::Html)
                            .await;
                        last_edit = tokio::time::Instant::now();
                    }
                },
                StreamEvent::Done => {
                    break;
                },
                StreamEvent::Error(e) => {
                    debug!("stream error: {e}");
                    accumulated.push_str(&format!("\n\n⚠ Error: {e}"));
                    break;
                },
            }
        }

        // Final edit with complete content
        if !accumulated.is_empty() {
            let html = markdown::markdown_to_telegram_html(&accumulated);
            let chunks = markdown::chunk_message(&html, TELEGRAM_MAX_MESSAGE_LEN);

            // Edit the placeholder with the first chunk
            let _ = bot
                .edit_message_text(chat_id, msg_id, &chunks[0])
                .parse_mode(ParseMode::Html)
                .await;

            // Send remaining chunks as new messages
            for chunk in &chunks[1..] {
                bot.send_message(chat_id, chunk)
                    .parse_mode(ParseMode::Html)
                    .await?;
            }
        }

        Ok(())
    }
}
