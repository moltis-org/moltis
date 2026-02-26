use std::sync::Arc;

use {async_trait::async_trait, tokio::sync::RwLock};

use {
    moltis_channels::{
        ChannelOutbound, ChannelStreamOutbound, Error as ChannelError, Result as ChannelResult,
        StreamReceiver,
    },
    moltis_discord::DiscordPlugin,
    moltis_msteams::MsTeamsPlugin,
    moltis_telegram::TelegramPlugin,
};

/// Routes outbound messages to the correct channel plugin based on account_id.
///
/// Implements both [`ChannelOutbound`] and [`ChannelStreamOutbound`] by resolving
/// the account_id to a plugin at call time.
pub struct MultiChannelOutbound {
    telegram_plugin: Arc<RwLock<TelegramPlugin>>,
    msteams_plugin: Arc<RwLock<MsTeamsPlugin>>,
    discord_plugin: Arc<RwLock<DiscordPlugin>>,
    telegram_outbound: Arc<dyn ChannelOutbound>,
    msteams_outbound: Arc<dyn ChannelOutbound>,
    discord_outbound: Arc<dyn ChannelOutbound>,
    telegram_stream: Arc<dyn ChannelStreamOutbound>,
    msteams_stream: Arc<dyn ChannelStreamOutbound>,
    discord_stream: Arc<dyn ChannelStreamOutbound>,
}

impl MultiChannelOutbound {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        telegram_plugin: Arc<RwLock<TelegramPlugin>>,
        msteams_plugin: Arc<RwLock<MsTeamsPlugin>>,
        discord_plugin: Arc<RwLock<DiscordPlugin>>,
        telegram_outbound: Arc<dyn ChannelOutbound>,
        msteams_outbound: Arc<dyn ChannelOutbound>,
        discord_outbound: Arc<dyn ChannelOutbound>,
        telegram_stream: Arc<dyn ChannelStreamOutbound>,
        msteams_stream: Arc<dyn ChannelStreamOutbound>,
        discord_stream: Arc<dyn ChannelStreamOutbound>,
    ) -> Self {
        Self {
            telegram_plugin,
            msteams_plugin,
            discord_plugin,
            telegram_outbound,
            msteams_outbound,
            discord_outbound,
            telegram_stream,
            msteams_stream,
            discord_stream,
        }
    }

    async fn resolve_outbound(&self, account_id: &str) -> ChannelResult<&dyn ChannelOutbound> {
        let (tg_has, ms_has, dc_has) = {
            let tg = self.telegram_plugin.read().await;
            let ms = self.msteams_plugin.read().await;
            let dc = self.discord_plugin.read().await;
            (
                tg.has_account(account_id),
                ms.has_account(account_id),
                dc.has_account(account_id),
            )
        };
        let count = usize::from(tg_has) + usize::from(ms_has) + usize::from(dc_has);
        if count > 1 {
            return Err(ChannelError::invalid_input(format!(
                "account_id '{account_id}' exists in multiple channels; explicit routing required"
            )));
        }
        if tg_has {
            Ok(self.telegram_outbound.as_ref())
        } else if ms_has {
            Ok(self.msteams_outbound.as_ref())
        } else if dc_has {
            Ok(self.discord_outbound.as_ref())
        } else {
            Err(ChannelError::unknown_account(account_id))
        }
    }

    async fn resolve_stream(&self, account_id: &str) -> ChannelResult<&dyn ChannelStreamOutbound> {
        let (tg_has, ms_has, dc_has) = {
            let tg = self.telegram_plugin.read().await;
            let ms = self.msteams_plugin.read().await;
            let dc = self.discord_plugin.read().await;
            (
                tg.has_account(account_id),
                ms.has_account(account_id),
                dc.has_account(account_id),
            )
        };
        let count = usize::from(tg_has) + usize::from(ms_has) + usize::from(dc_has);
        if count > 1 {
            return Err(ChannelError::invalid_input(format!(
                "account_id '{account_id}' exists in multiple channels; explicit routing required"
            )));
        }
        if tg_has {
            Ok(self.telegram_stream.as_ref())
        } else if ms_has {
            Ok(self.msteams_stream.as_ref())
        } else if dc_has {
            Ok(self.discord_stream.as_ref())
        } else {
            Err(ChannelError::unknown_account(account_id))
        }
    }
}

#[async_trait]
impl ChannelOutbound for MultiChannelOutbound {
    async fn send_text(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        self.resolve_outbound(account_id)
            .await?
            .send_text(account_id, to, text, reply_to)
            .await
    }

    async fn send_media(
        &self,
        account_id: &str,
        to: &str,
        payload: &moltis_common::types::ReplyPayload,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        self.resolve_outbound(account_id)
            .await?
            .send_media(account_id, to, payload, reply_to)
            .await
    }

    async fn send_typing(&self, account_id: &str, to: &str) -> ChannelResult<()> {
        self.resolve_outbound(account_id)
            .await?
            .send_typing(account_id, to)
            .await
    }

    async fn send_text_with_suffix(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        suffix_html: &str,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        self.resolve_outbound(account_id)
            .await?
            .send_text_with_suffix(account_id, to, text, suffix_html, reply_to)
            .await
    }

    async fn send_html(
        &self,
        account_id: &str,
        to: &str,
        html: &str,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        self.resolve_outbound(account_id)
            .await?
            .send_html(account_id, to, html, reply_to)
            .await
    }

    async fn send_text_silent(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        self.resolve_outbound(account_id)
            .await?
            .send_text_silent(account_id, to, text, reply_to)
            .await
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
        self.resolve_outbound(account_id)
            .await?
            .send_location(account_id, to, latitude, longitude, title, reply_to)
            .await
    }
}

#[async_trait]
impl ChannelStreamOutbound for MultiChannelOutbound {
    async fn send_stream(
        &self,
        account_id: &str,
        to: &str,
        reply_to: Option<&str>,
        stream: StreamReceiver,
    ) -> ChannelResult<()> {
        self.resolve_stream(account_id)
            .await?
            .send_stream(account_id, to, reply_to, stream)
            .await
    }

    async fn is_stream_enabled(&self, account_id: &str) -> bool {
        match self.resolve_stream(account_id).await {
            Ok(stream) => stream.is_stream_enabled(account_id).await,
            Err(_) => false,
        }
    }
}
