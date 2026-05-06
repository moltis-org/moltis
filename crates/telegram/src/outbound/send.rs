//! Text and location sending for the `ChannelOutbound` trait implementation.

use {
    async_trait::async_trait,
    teloxide::{
        payloads::{SendChatActionSetters, SendLocationSetters, SendVenueSetters},
        prelude::*,
        types::{ChatAction, ChatId, ParseMode},
    },
    tracing::info,
};

use moltis_channels::{Result, plugin::ChannelOutbound};

use moltis_common::types::ReplyPayload;

use crate::{
    markdown::{self, TELEGRAM_MAX_MESSAGE_LEN},
    topic::parse_chat_target,
};

use super::{TelegramOutbound, retry::RequestResultExt};

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
        let (chat_id, thread_id) = parse_chat_target(to)?;
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
                thread_id,
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
        let (chat_id, thread_id) = parse_chat_target(to)?;
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
                        thread_id,
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
                        thread_id,
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
                thread_id,
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
        let (chat_id, thread_id) = parse_chat_target(to)?;
        let rp = self.reply_params(account_id, reply_to);

        // Send raw HTML chunks without markdown conversion.
        let chunks = markdown::chunk_message(html, TELEGRAM_MAX_MESSAGE_LEN);
        for chunk in &chunks {
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
        }
        Ok(())
    }

    async fn send_typing(&self, account_id: &str, to: &str) -> Result<()> {
        let bot = self.get_bot(account_id)?;
        let (chat_id, thread_id) = parse_chat_target(to)?;
        let mut req = bot.send_chat_action(chat_id, ChatAction::Typing);
        if let Some(tid) = thread_id {
            req = req.message_thread_id(tid);
        }
        let _ = req.await;
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
        let (chat_id, thread_id) = parse_chat_target(to)?;
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
            self.send_chunk_with_fallback(
                &bot,
                account_id,
                to,
                chat_id,
                thread_id,
                chunk,
                rp.as_ref(),
                true,
            )
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
        super::media::send_media_impl(self, account_id, to, payload, reply_to).await
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
        let (chat_id, thread_id) = parse_chat_target(to)?;
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
            if let Some(tid) = thread_id {
                req = req.message_thread_id(tid);
            }
            if let Some(ref rp) = rp {
                req = req.reply_parameters(rp.clone());
            }
            req.await.channel_context("send venue")?;
        } else {
            let mut req = bot.send_location(chat_id, latitude, longitude);
            if let Some(tid) = thread_id {
                req = req.message_thread_id(tid);
            }
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
    /// Send a `ReplyPayload` -- dispatches to text or media.
    pub async fn send_reply(&self, bot: &Bot, to: &str, payload: &ReplyPayload) -> Result<()> {
        let chat_id = ChatId(to.parse::<i64>()?);

        // Send typing indicator
        let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;

        if payload.media.is_some() {
            // Use the media path -- but we need account_id, which we don't have here.
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
