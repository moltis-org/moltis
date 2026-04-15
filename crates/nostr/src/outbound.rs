//! Outbound message sending for Nostr channels.
//!
//! Implements `ChannelOutbound` and `ChannelStreamOutbound` — encrypts text
//! with NIP-04 and publishes to connected relays.

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use {
    async_trait::async_trait,
    moltis_channels::{
        ChannelOutbound, ChannelStreamOutbound, Result as ChannelResult, StreamReceiver,
        plugin::StreamEvent,
    },
    moltis_common::types::ReplyPayload,
    nostr_sdk::prelude::*,
};

use crate::state::AccountState;

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, histogram, nostr as nostr_metrics};

/// Shared account state map type.
///
/// Uses `std::sync::RwLock` (not `tokio::sync::RwLock`) so that sync
/// `ChannelPlugin` trait methods (`has_account`, `account_ids`, etc.) can
/// read from it without panicking inside a tokio runtime.
pub type AccountStateMap = Arc<RwLock<HashMap<String, AccountState>>>;

/// Nostr outbound adapter.
pub struct NostrOutbound {
    pub accounts: AccountStateMap,
}

impl NostrOutbound {
    /// Look up account state and resolve the recipient pubkey.
    async fn resolve(
        &self,
        account_id: &str,
        to: &str,
    ) -> ChannelResult<(Client, Keys, PublicKey)> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        let state = accounts.get(account_id).ok_or_else(|| {
            moltis_channels::Error::unavailable(format!("nostr account not found: {account_id}"))
        })?;
        let recipient = PublicKey::parse(to).map_err(|e| {
            moltis_channels::Error::invalid_input(format!("invalid recipient pubkey: {e}"))
        })?;
        Ok((state.client.clone(), state.keys.clone(), recipient))
    }
}

#[async_trait]
impl ChannelOutbound for NostrOutbound {
    async fn send_text(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        _reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let (client, keys, recipient) = self.resolve(account_id, to).await?;

        #[cfg(feature = "metrics")]
        let start = tokio::time::Instant::now();

        // Encrypt with NIP-04
        let encrypted = nip04::encrypt(keys.secret_key(), &recipient, text).map_err(|e| {
            #[cfg(feature = "metrics")]
            counter!(nostr_metrics::MESSAGE_SEND_ERRORS_TOTAL, "reason" => "encrypt").increment(1);
            moltis_channels::Error::external(
                "nostr",
                crate::error::Error::Encryption(format!("NIP-04 encrypt failed: {e}")),
            )
        })?;

        // Build and publish kind:4 event
        let tag = Tag::public_key(recipient);
        let builder = EventBuilder::new(Kind::EncryptedDirectMessage, &encrypted).tag(tag);

        client.send_event_builder(builder).await.map_err(|e| {
            #[cfg(feature = "metrics")]
            counter!(nostr_metrics::MESSAGE_SEND_ERRORS_TOTAL, "reason" => "relay").increment(1);
            moltis_channels::Error::external("nostr", crate::error::Error::Sdk(e))
        })?;

        #[cfg(feature = "metrics")]
        {
            counter!(nostr_metrics::MESSAGES_SENT_TOTAL).increment(1);
            histogram!(nostr_metrics::MESSAGE_SEND_DURATION_SECONDS)
                .record(start.elapsed().as_secs_f64());
        }

        let npub = recipient.to_bech32().unwrap_or_else(|_| recipient.to_hex());
        tracing::debug!(account_id, to = %npub, len = text.len(), "sent NIP-04 DM");

        Ok(())
    }

    async fn send_media(
        &self,
        _account_id: &str,
        _to: &str,
        _payload: &ReplyPayload,
        _reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        // Media not yet supported on Nostr (future: NIP-94)
        tracing::debug!("send_media not supported for Nostr");
        Ok(())
    }
}

#[async_trait]
impl ChannelStreamOutbound for NostrOutbound {
    async fn send_stream(
        &self,
        account_id: &str,
        to: &str,
        reply_to: Option<&str>,
        mut stream: StreamReceiver,
    ) -> ChannelResult<()> {
        // Nostr doesn't support edit-in-place streaming.
        // Collect all chunks and send as a single message.
        let mut buffer = String::new();

        while let Some(event) = stream.recv().await {
            match event {
                StreamEvent::Delta(chunk) => buffer.push_str(&chunk),
                StreamEvent::Done => break,
                StreamEvent::Error(e) => {
                    tracing::warn!(account_id, "stream error: {e}");
                    if buffer.is_empty() {
                        buffer.push_str("[Error generating response]");
                    }
                    break;
                },
            }
        }

        if !buffer.is_empty() {
            self.send_text(account_id, to, &buffer, reply_to).await?;
        }

        Ok(())
    }

    async fn is_stream_enabled(&self, _account_id: &str) -> bool {
        true
    }
}

impl std::fmt::Debug for NostrOutbound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NostrOutbound").finish_non_exhaustive()
    }
}
