//! Outbound message sending for Nostr channels.
//!
//! Implements `ChannelOutbound` and `ChannelStreamOutbound`. A send target is
//! either a configured NIP-29 group id — published as a plaintext kind:9 group
//! chat message (e.g. a Buzz channel reply) — or a pubkey, sent as a NIP-59
//! gift-wrapped DM (kind:1059). Routing is decided by whether the target
//! matches one of the account's configured `groups`.

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

/// Where an outbound message should go: a NIP-29 group or a DM recipient.
enum SendTarget {
    /// A configured NIP-29 group id (`h` tag value).
    Group(String),
    /// A DM recipient pubkey.
    Dm(PublicKey),
}

impl NostrOutbound {
    /// Look up account state and resolve the target: a configured group id is
    /// treated as a NIP-29 group send; anything else is parsed as a DM pubkey.
    async fn resolve(
        &self,
        account_id: &str,
        to: &str,
    ) -> ChannelResult<(Client, Keys, SendTarget)> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        let state = accounts.get(account_id).ok_or_else(|| {
            moltis_channels::Error::unavailable(format!("nostr account not found: {account_id}"))
        })?;
        let client = state.client.clone();
        let keys = state.keys.clone();

        let is_group = {
            let cfg = state.config.read().unwrap_or_else(|e| e.into_inner());
            cfg.groups.iter().any(|g| g == to)
        };
        if is_group {
            return Ok((client, keys, SendTarget::Group(to.to_string())));
        }

        let recipient = PublicKey::parse(to).map_err(|e| {
            moltis_channels::Error::invalid_input(format!("invalid recipient pubkey: {e}"))
        })?;
        Ok((client, keys, SendTarget::Dm(recipient)))
    }
}

#[async_trait]
impl ChannelOutbound for NostrOutbound {
    async fn send_text(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> ChannelResult<()> {
        let (client, keys, target) = self.resolve(account_id, to).await?;

        #[cfg(feature = "metrics")]
        let start = tokio::time::Instant::now();

        match target {
            SendTarget::Group(group_id) => {
                // NIP-29 group chat reply (kind:9). `reply_to` is the inbound
                // event id, threaded via a NIP-10 `e` tag.
                let reply_event = reply_to.and_then(|id| EventId::parse(id).ok());
                crate::groups::send_group_message(&client, &group_id, text, reply_event, None)
                    .await
                    .map_err(|e| {
                        #[cfg(feature = "metrics")]
                        counter!(nostr_metrics::MESSAGE_SEND_ERRORS_TOTAL, "reason" => "group")
                            .increment(1);
                        moltis_channels::Error::external("nostr", e)
                    })?;
                tracing::debug!(account_id, group = %group_id, len = text.len(), "sent group message");
            },
            SendTarget::Dm(recipient) => {
                // NIP-59 gift-wrapped DM (kind:1059).
                crate::gift_wrap::send_gift_wrapped_dm(&client, &keys, &recipient, text)
                    .await
                    .map_err(|e| {
                        #[cfg(feature = "metrics")]
                        counter!(nostr_metrics::MESSAGE_SEND_ERRORS_TOTAL, "reason" => "gift_wrap")
                            .increment(1);
                        moltis_channels::Error::external("nostr", e)
                    })?;
                let npub = recipient.to_bech32().unwrap_or_else(|_| recipient.to_hex());
                tracing::debug!(account_id, to = %npub, len = text.len(), "sent gift-wrapped DM");
            },
        }

        #[cfg(feature = "metrics")]
        {
            counter!(nostr_metrics::MESSAGES_SENT_TOTAL).increment(1);
            histogram!(nostr_metrics::MESSAGE_SEND_DURATION_SECONDS)
                .record(start.elapsed().as_secs_f64());
        }

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
                StreamEvent::Delta(chunk) | StreamEvent::ProgressDelta(chunk) => {
                    buffer.push_str(&chunk)
                },
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

#[cfg(test)]
mod tests {
    use {
        crate::{config::NostrAccountConfig, state::AccountState},
        tokio_util::sync::CancellationToken,
    };

    use super::*;

    /// Build an outbound adapter with one account whose config names `groups`.
    /// No relay connection is made — `resolve` is pure routing logic.
    fn outbound_with_groups(groups: Vec<String>) -> NostrOutbound {
        let keys = Keys::generate();
        let client = Client::new(keys.clone());
        let config = NostrAccountConfig {
            groups,
            ..Default::default()
        };
        let state = AccountState {
            client,
            keys,
            config: Arc::new(RwLock::new(config)),
            cached_allowlist: Arc::new(RwLock::new(Vec::new())),
            cancel: CancellationToken::new(),
            otp: Arc::new(std::sync::Mutex::new(moltis_channels::otp::OtpState::new(
                300,
            ))),
        };
        let accounts: AccountStateMap = Arc::new(RwLock::new(HashMap::new()));
        accounts
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert("acct".to_string(), state);
        NostrOutbound { accounts }
    }

    #[tokio::test]
    async fn resolves_configured_group_as_group_send() {
        let outbound = outbound_with_groups(vec!["buzz-general".into()]);
        let resolved = outbound.resolve("acct", "buzz-general").await;
        assert!(matches!(resolved, Ok((_, _, SendTarget::Group(ref g))) if g == "buzz-general"));
    }

    #[tokio::test]
    async fn resolves_pubkey_as_dm_send() {
        let outbound = outbound_with_groups(vec!["buzz-general".into()]);
        let peer = Keys::generate().public_key().to_hex();
        let resolved = outbound.resolve("acct", &peer).await;
        assert!(matches!(resolved, Ok((_, _, SendTarget::Dm(_)))));
    }

    /// Turning group chat off means clearing `groups`, and outbound reads that
    /// same list — so a queued reply cannot keep publishing kind:9 events to a
    /// group the operator has since removed. A group id is not a valid pubkey,
    /// so it fails closed rather than falling through to a DM.
    #[tokio::test]
    async fn refuses_group_send_once_groups_cleared() {
        let outbound = outbound_with_groups(Vec::new());
        let resolved = outbound.resolve("acct", "buzz-general").await;
        assert!(
            resolved.is_err(),
            "group send must fail once the group is no longer configured"
        );
    }

    #[tokio::test]
    async fn unknown_account_is_unavailable() {
        let outbound = outbound_with_groups(vec!["buzz-general".into()]);
        let resolved = outbound.resolve("missing", "buzz-general").await;
        assert!(resolved.is_err(), "unknown account must not resolve");
    }
}
