use {async_trait::async_trait, tracing::debug};

use {wacore_binary::jid::Jid, waproto::whatsapp as wa, whatsapp_rust::ChatStateType};

use {
    moltis_channels::{Result as ChannelResult, plugin::ChannelOutbound},
    moltis_common::types::ReplyPayload,
};

use crate::state::{AccountStateMap, BOT_WATERMARK};

/// Outbound message sender for WhatsApp.
pub struct WhatsAppOutbound {
    pub(crate) accounts: AccountStateMap,
}

impl WhatsAppOutbound {
    fn get_client(
        &self,
        account_id: &str,
    ) -> ChannelResult<std::sync::Arc<whatsapp_rust::client::Client>> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        accounts
            .get(account_id)
            .map(|s| std::sync::Arc::clone(&s.client))
            .ok_or_else(|| moltis_channels::Error::unknown_account(account_id))
    }

    /// Record a sent message ID for self-chat loop detection.
    fn record_sent_id(&self, account_id: &str, msg_id: &str) {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = accounts.get(account_id) {
            state.record_sent_id(msg_id);
        }
    }
}

#[async_trait]
impl ChannelOutbound for WhatsAppOutbound {
    async fn send_text(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        _reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let client = self.get_client(account_id)?;
        let jid: Jid = to
            .parse()
            .map_err(|e| moltis_channels::Error::invalid_input(format!("invalid JID: {e:?}")))?;

        debug!(
            account_id,
            to,
            text_len = text.len(),
            "sending WhatsApp text"
        );

        let mut watermarked = text.to_string();
        watermarked.push_str(BOT_WATERMARK);
        let msg = wa::Message {
            conversation: Some(watermarked),
            ..Default::default()
        };
        let msg_id = client
            .send_message(jid, msg)
            .await
            .map_err(|e| moltis_channels::Error::external("whatsapp send_text", e))?;
        self.record_sent_id(account_id, &msg_id);
        Ok(())
    }

    async fn send_media(
        &self,
        account_id: &str,
        to: &str,
        payload: &ReplyPayload,
        _reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        // For now, send text only. Media upload support to be added.
        if !payload.text.is_empty() {
            self.send_text(account_id, to, &payload.text, None).await?;
        }
        Ok(())
    }

    async fn send_typing(&self, account_id: &str, to: &str) -> ChannelResult<()> {
        let client = self.get_client(account_id)?;
        let jid: Jid = to
            .parse()
            .map_err(|e| moltis_channels::Error::invalid_input(format!("invalid JID: {e:?}")))?;
        client
            .chatstate()
            .send(&jid, ChatStateType::Composing)
            .await
            .map_err(|e| moltis_channels::Error::external("whatsapp chatstate", e))?;
        Ok(())
    }
}
