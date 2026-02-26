use {async_trait::async_trait, tracing::debug};

use {
    moltis_channels::{
        Error as ChannelError, Result as ChannelResult,
        plugin::{ChannelOutbound, ChannelStreamOutbound, StreamEvent, StreamReceiver},
    },
    moltis_common::types::ReplyPayload,
    serenity::all::ChannelId,
};

use crate::{handler::send_discord_message, state::AccountStateMap};

/// Outbound sender for Discord channel accounts.
pub struct DiscordOutbound {
    pub(crate) accounts: AccountStateMap,
}

impl DiscordOutbound {
    fn resolve_http(
        &self,
        account_id: &str,
    ) -> ChannelResult<std::sync::Arc<serenity::http::Http>> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        let state = accounts
            .get(account_id)
            .ok_or_else(|| ChannelError::unknown_account(account_id))?;
        state.http.clone().ok_or_else(|| {
            ChannelError::unavailable(format!(
                "Discord bot for account '{account_id}' is not connected yet"
            ))
        })
    }

    fn parse_channel_id(to: &str) -> ChannelResult<ChannelId> {
        to.parse::<u64>()
            .map(ChannelId::new)
            .map_err(|_| ChannelError::invalid_input(format!("invalid Discord channel ID: {to}")))
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
        let http = self.resolve_http(account_id)?;
        let channel_id = Self::parse_channel_id(to)?;
        send_discord_message(&http, channel_id, text)
            .await
            .map_err(|e| ChannelError::external("Discord send", std::io::Error::other(e)))
    }

    async fn send_media(
        &self,
        account_id: &str,
        to: &str,
        payload: &ReplyPayload,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let mut text = payload.text.clone();
        if let Some(media) = payload.media.as_ref() {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            if media.url.starts_with("data:") {
                text.push_str(
                    "[media omitted: Discord channel currently supports URL attachments only]",
                );
            } else {
                text.push_str(&media.url);
            }
        }
        self.send_text(account_id, to, &text, reply_to).await
    }

    async fn send_typing(&self, account_id: &str, to: &str) -> ChannelResult<()> {
        let http = self.resolve_http(account_id)?;
        let channel_id = Self::parse_channel_id(to)?;
        channel_id.broadcast_typing(&http).await.map_err(|e| {
            ChannelError::external("Discord typing", std::io::Error::other(e.to_string()))
        })
    }

    async fn send_text_with_suffix(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        suffix_html: &str,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        // Discord doesn't render HTML, so just append the suffix as plain text.
        let mut merged = text.to_string();
        if !suffix_html.is_empty() {
            merged.push_str("\n\n");
            merged.push_str(suffix_html);
        }
        self.send_text(account_id, to, &merged, reply_to).await
    }

    async fn send_location(
        &self,
        account_id: &str,
        to: &str,
        latitude: f64,
        longitude: f64,
        title: Option<&str>,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let mut text = String::new();
        if let Some(title) = title {
            text.push_str(title);
            text.push('\n');
        }
        text.push_str(&format!(
            "https://www.google.com/maps?q={latitude:.6},{longitude:.6}"
        ));
        self.send_text(account_id, to, &text, reply_to).await
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
        let mut text = String::new();
        while let Some(event) = stream.recv().await {
            match event {
                StreamEvent::Delta(delta) => text.push_str(&delta),
                StreamEvent::Done => break,
                StreamEvent::Error(err) => {
                    debug!(account_id, chat_id = to, "Discord stream error: {err}");
                    if text.is_empty() {
                        text = err;
                    }
                    break;
                },
            }
        }
        if text.is_empty() {
            return Ok(());
        }
        self.send_text(account_id, to, &text, reply_to).await
    }

    async fn is_stream_enabled(&self, _account_id: &str) -> bool {
        false
    }
}
