use std::{sync::Arc, time::Duration};

use {async_trait::async_trait, slack_morphism::prelude::*, tracing::debug};

use {
    moltis_channels::{
        Error as ChannelError, Result as ChannelResult,
        plugin::{ChannelOutbound, ChannelStreamOutbound, StreamEvent, StreamReceiver},
    },
    moltis_common::types::ReplyPayload,
};

use crate::{
    markdown::{SLACK_MAX_MESSAGE_LEN, chunk_message, markdown_to_slack},
    state::AccountStateMap,
};

/// Outbound message sender for Slack.
pub struct SlackOutbound {
    pub(crate) accounts: AccountStateMap,
}

impl SlackOutbound {
    fn get_client(
        &self,
        account_id: &str,
    ) -> ChannelResult<(
        Arc<SlackClient<SlackClientHyperConnector<SlackHyperHttpsConnector>>>,
        SlackApiToken,
    )> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        let state = accounts
            .get(account_id)
            .ok_or_else(|| ChannelError::unknown_account(account_id))?;

        let token = SlackApiToken::new(
            secrecy::ExposeSecret::expose_secret(&state.config.bot_token).into(),
        );

        Ok((state.client.clone(), token))
    }

    fn get_thread_ts(&self, account_id: &str, channel_id: &str) -> Option<String> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .and_then(|s| s.pending_threads.get(channel_id).cloned())
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
impl ChannelOutbound for SlackOutbound {
    async fn send_text(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        _reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let (client, token) = self.get_client(account_id)?;
        let channel_id: SlackChannelId = to.into();
        let thread_ts = self.get_thread_ts(account_id, to);

        let slack_text = markdown_to_slack(text);
        let chunks = chunk_message(&slack_text, SLACK_MAX_MESSAGE_LEN);

        let session = client.open_session(&token);

        for chunk in chunks {
            let content = SlackMessageContent::new().with_text(chunk);
            let mut req = SlackApiChatPostMessageRequest::new(channel_id.clone(), content);

            if let Some(ref ts) = thread_ts {
                req = req.with_thread_ts(ts.clone().into());
            }

            session
                .chat_post_message(&req)
                .await
                .channel_context("slack chat.postMessage")?;
        }

        Ok(())
    }

    async fn send_typing(&self, _account_id: &str, _to: &str) -> ChannelResult<()> {
        // Slack doesn't have a typing indicator API for bots
        Ok(())
    }

    async fn send_media(
        &self,
        account_id: &str,
        to: &str,
        payload: &ReplyPayload,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        // For now, just send the text content. Media upload requires file upload API.
        if !payload.text.is_empty() {
            self.send_text(account_id, to, &payload.text, reply_to)
                .await?;
        }
        Ok(())
    }
}

#[async_trait]
impl ChannelStreamOutbound for SlackOutbound {
    async fn send_stream(
        &self,
        account_id: &str,
        to: &str,
        reply_to: Option<&str>,
        mut stream: StreamReceiver,
    ) -> ChannelResult<()> {
        let (client, token) = self.get_client(account_id)?;
        let channel_id: SlackChannelId = to.into();
        let thread_ts = reply_to
            .map(ToString::to_string)
            .or_else(|| self.get_thread_ts(account_id, to));
        let throttle_ms = self.get_throttle_ms(account_id);

        let session = client.open_session(&token);

        // Send initial placeholder
        let placeholder_content = SlackMessageContent::new().with_text("...".to_string());
        let mut req = SlackApiChatPostMessageRequest::new(channel_id.clone(), placeholder_content);

        if let Some(ref ts) = thread_ts {
            req = req.with_thread_ts(ts.clone().into());
        }

        let response = session
            .chat_post_message(&req)
            .await
            .channel_context("slack chat.postMessage (stream placeholder)")?;
        let message_ts = response.ts;

        let mut accumulated = String::new();
        let mut last_edit = tokio::time::Instant::now();
        let throttle = Duration::from_millis(throttle_ms);

        while let Some(event) = stream.recv().await {
            match event {
                StreamEvent::Delta(delta) => {
                    accumulated.push_str(&delta);
                    if last_edit.elapsed() >= throttle {
                        let slack_text = markdown_to_slack(&accumulated);
                        // Truncate for display during streaming
                        let display = if slack_text.len() > SLACK_MAX_MESSAGE_LEN {
                            &slack_text[..SLACK_MAX_MESSAGE_LEN]
                        } else {
                            &slack_text
                        };

                        let update_content =
                            SlackMessageContent::new().with_text(display.to_string());
                        let update_req = SlackApiChatUpdateRequest::new(
                            channel_id.clone(),
                            update_content,
                            message_ts.clone(),
                        );

                        let _ = session.chat_update(&update_req).await;
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
            let slack_text = markdown_to_slack(&accumulated);
            let chunks = chunk_message(&slack_text, SLACK_MAX_MESSAGE_LEN);

            // Update placeholder with first chunk
            let first_chunk_content = SlackMessageContent::new().with_text(chunks[0].clone());
            let update_req =
                SlackApiChatUpdateRequest::new(channel_id.clone(), first_chunk_content, message_ts);
            let _ = session.chat_update(&update_req).await;

            // Send remaining chunks as new messages
            for chunk in &chunks[1..] {
                let chunk_content = SlackMessageContent::new().with_text(chunk.clone());
                let mut req =
                    SlackApiChatPostMessageRequest::new(channel_id.clone(), chunk_content);

                if let Some(ref ts) = thread_ts {
                    req = req.with_thread_ts(ts.clone().into());
                }

                session
                    .chat_post_message(&req)
                    .await
                    .channel_context("slack chat.postMessage (stream chunk)")?;
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
