use std::{sync::Arc, time::Duration};

use {
    async_trait::async_trait,
    serenity::all::{ChannelId, CreateMessage, EditMessage, Http, MessageId},
    tracing::debug,
};

use {
    moltis_channels::{
        Error as ChannelError, Result as ChannelResult,
        plugin::{ChannelOutbound, ChannelStreamOutbound, StreamEvent, StreamReceiver},
    },
    moltis_common::types::ReplyPayload,
};

use crate::{
    markdown::{DISCORD_MAX_MESSAGE_LEN, chunk_message, truncate_with_ellipsis},
    state::AccountStateMap,
};

/// Outbound message sender for Discord.
pub struct DiscordOutbound {
    pub(crate) accounts: AccountStateMap,
}

impl DiscordOutbound {
    fn get_http(&self, account_id: &str) -> ChannelResult<Arc<Http>> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .map(|s| s.http.clone())
            .ok_or_else(|| ChannelError::unknown_account(account_id))
    }

    fn get_pending_reply(&self, account_id: &str, channel_id: &str) -> Option<u64> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .and_then(|s| s.pending_replies.get(channel_id).copied())
    }

    fn get_throttle_ms(&self, account_id: &str) -> u64 {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .map(|s| s.config.edit_throttle_ms)
            .unwrap_or(500)
    }
}

#[async_trait]
impl ChannelOutbound for DiscordOutbound {
    async fn send_text(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        _reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let http = self.get_http(account_id)?;
        let channel_id: u64 = to.parse()?;
        let channel = ChannelId::new(channel_id);

        let reply_to = self.get_pending_reply(account_id, to);
        let chunks = chunk_message(text, DISCORD_MAX_MESSAGE_LEN);

        for (i, chunk) in chunks.iter().enumerate() {
            let mut builder = CreateMessage::new().content(chunk);

            // Reply to original message on first chunk
            if i == 0
                && let Some(msg_id) = reply_to
            {
                builder = builder.reference_message((channel, MessageId::new(msg_id)));
            }

            channel
                .send_message(&http, builder)
                .await
                .channel_context("discord send message")?;
        }

        Ok(())
    }

    async fn send_typing(&self, account_id: &str, to: &str) -> ChannelResult<()> {
        let http = self.get_http(account_id)?;
        let channel_id: u64 = to.parse()?;
        ChannelId::new(channel_id)
            .broadcast_typing(&http)
            .await
            .channel_context("discord send typing")?;
        Ok(())
    }

    async fn send_media(
        &self,
        account_id: &str,
        to: &str,
        payload: &ReplyPayload,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        // For now, just send the text content. Media requires attachment API.
        if !payload.text.is_empty() {
            self.send_text(account_id, to, &payload.text, reply_to)
                .await?;
        }
        Ok(())
    }
}

#[async_trait]
impl ChannelStreamOutbound for DiscordOutbound {
    async fn send_stream(
        &self,
        account_id: &str,
        to: &str,
        reply_to: Option<&str>,
        mut stream: StreamReceiver,
    ) -> ChannelResult<()> {
        let http = self.get_http(account_id)?;
        let channel_id: u64 = to.parse()?;
        let channel = ChannelId::new(channel_id);
        let throttle_ms = self.get_throttle_ms(account_id);

        // Send typing indicator
        let _ = channel.broadcast_typing(&http).await;

        // Send initial placeholder
        let reply_to = reply_to
            .and_then(|value| value.parse().ok())
            .or_else(|| self.get_pending_reply(account_id, to));
        let mut builder = CreateMessage::new().content("...");
        if let Some(msg_id) = reply_to {
            builder = builder.reference_message((channel, MessageId::new(msg_id)));
        }

        let initial_msg = channel
            .send_message(&http, builder)
            .await
            .channel_context("discord send stream placeholder")?;
        let message_id = initial_msg.id;

        let mut accumulated = String::new();
        let mut last_edit = tokio::time::Instant::now();
        let throttle = Duration::from_millis(throttle_ms);

        while let Some(event) = stream.recv().await {
            match event {
                StreamEvent::Delta(delta) => {
                    accumulated.push_str(&delta);
                    if last_edit.elapsed() >= throttle {
                        // Truncate for display during streaming
                        let display = truncate_with_ellipsis(&accumulated, DISCORD_MAX_MESSAGE_LEN);

                        let _ = channel
                            .edit_message(&http, message_id, EditMessage::new().content(&display))
                            .await;
                        last_edit = tokio::time::Instant::now();
                    }
                },
                StreamEvent::Done => {
                    break;
                },
                StreamEvent::Error(e) => {
                    debug!("stream error: {e}");
                    accumulated.push_str(&format!("\n\n:warning: Error: {e}"));
                    break;
                },
            }
        }

        // Final update with complete content
        if !accumulated.is_empty() {
            if accumulated.len() <= DISCORD_MAX_MESSAGE_LEN {
                // Fits in one message, just edit
                let _ = channel
                    .edit_message(&http, message_id, EditMessage::new().content(&accumulated))
                    .await;
            } else {
                // Need to split - delete placeholder and send chunks
                let _ = channel.delete_message(&http, message_id).await;

                let chunks = chunk_message(&accumulated, DISCORD_MAX_MESSAGE_LEN);
                for chunk in chunks {
                    channel
                        .send_message(&http, CreateMessage::new().content(&chunk))
                        .await
                        .channel_context("discord send stream chunk")?;
                }
            }
        }

        Ok(())
    }
}

trait ChannelContext<T> {
    fn channel_context(self, context: &'static str) -> ChannelResult<T>;
}

impl<T, E> ChannelContext<T> for Result<T, E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn channel_context(self, context: &'static str) -> ChannelResult<T> {
        self.map_err(|e| ChannelError::external(context, e))
    }
}
