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

impl TelegramOutbound {
    /// Send raw HTML chunks, returning every message id they produced.
    async fn send_html_ids(
        &self,
        account_id: &str,
        to: &str,
        html: &str,
        reply_to: Option<&str>,
    ) -> Result<Vec<String>> {
        let bot = self.get_bot(account_id)?;
        let (chat_id, thread_id) = parse_chat_target(to)?;
        let rp = self.reply_params(account_id, reply_to);

        // Send raw HTML chunks without markdown conversion.
        let chunks = markdown::chunk_message(html, TELEGRAM_MAX_MESSAGE_LEN);
        let mut ids = Vec::with_capacity(chunks.len());
        for chunk in &chunks {
            let id = self
                .send_chunk_with_fallback(
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
            ids.push(id.0.to_string());
        }
        Ok(ids)
    }

    /// Send `text` with `suffix_html` appended, returning every message id it
    /// produced.
    ///
    /// Shared by both trait entry points so the chunking and suffix-overflow
    /// rules exist once; the non-reporting one simply discards the ids.
    async fn send_text_with_suffix_ids(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        suffix_html: &str,
        reply_to: Option<&str>,
    ) -> Result<Vec<String>> {
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

        let mut ids = Vec::with_capacity(chunks.len());
        for (i, chunk) in chunks.iter().enumerate() {
            let content = if i == last_idx {
                // Append suffix to the last chunk. If it would exceed the limit,
                // the suffix becomes a separate final message.
                let combined = format!("{chunk}\n\n{suffix_html}");
                if combined.len() <= TELEGRAM_MAX_MESSAGE_LEN {
                    combined
                } else {
                    // Send this chunk first, then the suffix as a separate message.
                    let id = self
                        .send_chunk_with_fallback(
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
                    ids.push(id.0.to_string());
                    // Send suffix as the final message (no reply threading).
                    let suffix_id = self
                        .send_chunk_with_fallback(
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
                    // A reader rating "the bot's answer" may land on the
                    // logbook, and it maps back to the same turn, so linking it
                    // recovers a score that would otherwise be dropped.
                    ids.push(suffix_id.0.to_string());
                    info!(
                        account_id,
                        chat_id = to,
                        reply_to = ?reply_to,
                        text_len = text.len(),
                        suffix_len = suffix_html.len(),
                        chunk_count = chunks.len(),
                        "telegram outbound text+suffix sent (separate suffix message)"
                    );
                    return Ok(ids);
                }
            } else {
                chunk.clone()
            };
            let id = self
                .send_chunk_with_fallback(
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
            ids.push(id.0.to_string());
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
        Ok(ids)
    }
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

    /// Telegram splits long replies into several messages, so every chunk id
    /// is reported: a reader may react to any of them.
    async fn send_text_reporting_ids(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> Result<Vec<String>> {
        let bot = self.get_bot(account_id)?;
        let (chat_id, thread_id) = parse_chat_target(to)?;
        let rp = self.reply_params(account_id, reply_to);

        let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;

        let chunks = markdown::chunk_markdown_html(text, TELEGRAM_MAX_MESSAGE_LEN);
        let mut ids = Vec::with_capacity(chunks.len());
        for chunk in chunks.iter() {
            let reply_params = rp.as_ref();
            let id = self
                .send_chunk_with_fallback(
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
            ids.push(id.0.to_string());
        }

        info!(
            account_id,
            chat_id = to,
            chunk_count = ids.len(),
            "telegram outbound text sent with ids"
        );
        Ok(ids)
    }

    async fn send_text_with_suffix(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        suffix_html: &str,
        reply_to: Option<&str>,
    ) -> Result<()> {
        self.send_text_with_suffix_ids(account_id, to, text, suffix_html, reply_to)
            .await?;
        Ok(())
    }

    async fn send_text_with_suffix_reporting_ids(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        suffix_html: &str,
        reply_to: Option<&str>,
    ) -> Result<Vec<String>> {
        self.send_text_with_suffix_ids(account_id, to, text, suffix_html, reply_to)
            .await
    }

    async fn send_html(
        &self,
        account_id: &str,
        to: &str,
        html: &str,
        reply_to: Option<&str>,
    ) -> Result<()> {
        self.send_html_ids(account_id, to, html, reply_to).await?;
        Ok(())
    }

    async fn send_html_reporting_ids(
        &self,
        account_id: &str,
        to: &str,
        html: &str,
        reply_to: Option<&str>,
    ) -> Result<Vec<String>> {
        self.send_html_ids(account_id, to, html, reply_to).await
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

    async fn send_media_reporting_ids(
        &self,
        account_id: &str,
        to: &str,
        payload: &ReplyPayload,
        reply_to: Option<&str>,
    ) -> Result<Vec<String>> {
        super::media::send_media_reporting_ids_impl(self, account_id, to, payload, reply_to).await
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
