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
    /// A configured group id (`h` tag value).
    Group(String),
    /// A DM recipient pubkey.
    Dm(PublicKey),
}

/// How to publish a group reply: which dialect, and whom to notify.
struct GroupSendPlan {
    kind: Kind,
    mention: Option<PublicKey>,
}

impl NostrOutbound {
    /// Decide the dialect and `p`-tag for a group send.
    ///
    /// Prefer the exact message being replied to (mirrors its kind and
    /// notifies its author), then the last kind seen in that group, then the
    /// account's configured default. Buzz uses `kind:40002` while plain NIP-29
    /// uses `kind:9`, and a client filtering one never sees the other — so
    /// answering in the wrong dialect makes the bot invisible.
    fn plan_group_send(
        &self,
        account_id: &str,
        group_id: &str,
        reply_to: Option<&str>,
    ) -> GroupSendPlan {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        let Some(state) = accounts.get(account_id) else {
            return GroupSendPlan {
                kind: crate::groups::group_chat_kind(),
                mention: None,
            };
        };

        let reply_event = reply_to.and_then(|id| EventId::parse(id).ok());
        let ctx = reply_event.and_then(|id| {
            let ctxs = state.reply_ctx.lock().unwrap_or_else(|e| e.into_inner());
            ctxs.get(&id)
        });
        if let Some(ctx) = ctx {
            return GroupSendPlan {
                kind: ctx.kind,
                mention: Some(ctx.author),
            };
        }

        let learned = {
            let ctxs = state.reply_ctx.lock().unwrap_or_else(|e| e.into_inner());
            ctxs.kind_for_group(group_id)
        };
        let kind = learned.unwrap_or_else(|| {
            let cfg = state.config.read().unwrap_or_else(|e| e.into_inner());
            Kind::from(cfg.group_message_kind)
        });
        GroupSendPlan {
            kind,
            mention: None,
        }
    }
}

/// Minimum gap between edit events while streaming a reply.
///
/// Every token would otherwise be a signed event on the relay; this keeps the
/// reply visibly live without flooding the channel or the audit log.
const STREAM_EDIT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(900);

impl NostrOutbound {
    /// Drain a stream and send the result as one message.
    ///
    /// Used for DMs and plain NIP-29 groups, neither of which can edit in place.
    async fn collect_then_send(
        &self,
        account_id: &str,
        to: &str,
        reply_to: Option<&str>,
        stream: StreamReceiver,
    ) -> ChannelResult<()> {
        self.collect_remaining_then_send(account_id, to, reply_to, stream, String::new())
            .await
    }

    /// Same as [`Self::collect_then_send`], but keeping text already received.
    async fn collect_remaining_then_send(
        &self,
        account_id: &str,
        to: &str,
        reply_to: Option<&str>,
        mut stream: StreamReceiver,
        mut buffer: String,
    ) -> ChannelResult<()> {
        while let Some(event) = stream.recv().await {
            match event {
                StreamEvent::Delta(chunk) | StreamEvent::ProgressDelta(chunk) => {
                    buffer.push_str(&chunk);
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
                // Group chat reply. `reply_to` is the inbound event id,
                // threaded via a NIP-10 `e` tag; the plan mirrors that
                // message's kind and `p`-tags its author so they are notified.
                let reply_event = reply_to.and_then(|id| EventId::parse(id).ok());
                let plan = self.plan_group_send(account_id, &group_id, reply_to);
                crate::groups::send_group_message(
                    &client,
                    plan.kind,
                    &group_id,
                    text,
                    reply_event,
                    plan.mention,
                )
                .await
                .map_err(|e| {
                    #[cfg(feature = "metrics")]
                    counter!(nostr_metrics::MESSAGE_SEND_ERRORS_TOTAL, "reason" => "group")
                        .increment(1);
                    moltis_channels::Error::external("nostr", e)
                })?;
                tracing::debug!(
                    account_id,
                    group = %group_id,
                    kind = plan.kind.as_u16(),
                    len = text.len(),
                    "sent group message"
                );
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

    /// React to a group message (NIP-25 `kind:7`), used for ack reactions.
    ///
    /// The gateway passes Slack-style shortcodes, which are translated to the
    /// emoji glyph NIP-25 expects. Reactions only make sense on group messages;
    /// DMs are gift-wrapped, so reacting would leak the conversation.
    async fn add_reaction(
        &self,
        account_id: &str,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> ChannelResult<()> {
        let Ok((client, _, SendTarget::Group(_))) = self.resolve(account_id, channel_id).await
        else {
            return Ok(());
        };
        let Ok(target) = EventId::parse(message_id) else {
            return Ok(());
        };

        let glyph = crate::groups::ack_emoji_glyph(emoji);
        let reaction = crate::groups::send_reaction(&client, target, glyph)
            .await
            .map_err(|e| moltis_channels::Error::external("nostr", e))?;

        // Remember it so `remove_reaction` can retract it via NIP-09.
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = accounts.get(account_id) {
            let mut ctxs = state.reply_ctx.lock().unwrap_or_else(|e| e.into_inner());
            ctxs.record_reaction(target, glyph, reaction);
        }
        Ok(())
    }

    /// Retract a reaction by publishing a NIP-09 deletion (`kind:5`) for it.
    ///
    /// Nostr has no "unreact", and deletion is only a request the relay may
    /// honour — so this is best-effort and silently no-ops when the reaction
    /// id is no longer known.
    async fn remove_reaction(
        &self,
        account_id: &str,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> ChannelResult<()> {
        let Ok((client, _, SendTarget::Group(_))) = self.resolve(account_id, channel_id).await
        else {
            return Ok(());
        };
        let Ok(target) = EventId::parse(message_id) else {
            return Ok(());
        };
        let glyph = crate::groups::ack_emoji_glyph(emoji);

        let reaction = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            accounts.get(account_id).and_then(|state| {
                let mut ctxs = state.reply_ctx.lock().unwrap_or_else(|e| e.into_inner());
                ctxs.take_reaction(target, glyph)
            })
        };
        let Some(reaction) = reaction else {
            return Ok(());
        };

        if let Err(e) = crate::groups::delete_event(&client, reaction).await {
            tracing::debug!(account_id, "failed to retract reaction: {e}");
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
        // Buzz groups support edit-in-place (kind:40003), so the reply can be
        // published early and revised as tokens arrive. Plain NIP-29 has no
        // edit kind, and DMs are gift-wrapped, so both collect and send once.
        let live = match self.resolve(account_id, to).await {
            Ok((client, _, SendTarget::Group(group_id))) => {
                let plan = self.plan_group_send(account_id, &group_id, reply_to);
                crate::groups::supports_edit(plan.kind).then_some((client, group_id, plan))
            },
            _ => None,
        };

        let Some((client, group_id, plan)) = live else {
            return self
                .collect_then_send(account_id, to, reply_to, stream)
                .await;
        };

        let reply_event = reply_to.and_then(|id| EventId::parse(id).ok());
        let mut buffer = String::new();
        let mut published: Option<EventId> = None;
        let mut last_edit = tokio::time::Instant::now();
        let mut pending_edit = false;

        while let Some(event) = stream.recv().await {
            match event {
                StreamEvent::Delta(chunk) | StreamEvent::ProgressDelta(chunk) => {
                    buffer.push_str(&chunk);
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

            if buffer.trim().is_empty() {
                continue;
            }

            match published {
                // First non-empty chunk: publish so the channel sees a reply
                // forming instead of waiting for the whole turn.
                None => {
                    match crate::groups::send_group_message(
                        &client,
                        plan.kind,
                        &group_id,
                        &buffer,
                        reply_event,
                        plan.mention,
                    )
                    .await
                    {
                        Ok(id) => {
                            published = Some(id);
                            last_edit = tokio::time::Instant::now();
                        },
                        Err(e) => {
                            tracing::warn!(account_id, "failed to publish streamed reply: {e}");
                            return self
                                .collect_remaining_then_send(
                                    account_id, to, reply_to, stream, buffer,
                                )
                                .await;
                        },
                    }
                },
                // Throttle edits: every token would be an event on the relay.
                Some(target) => {
                    if last_edit.elapsed() >= STREAM_EDIT_INTERVAL {
                        if let Err(e) =
                            crate::groups::edit_group_message(&client, &group_id, target, &buffer)
                                .await
                        {
                            tracing::debug!(account_id, "streamed edit failed: {e}");
                        }
                        last_edit = tokio::time::Instant::now();
                        pending_edit = false;
                    } else {
                        pending_edit = true;
                    }
                },
            }
        }

        match published {
            // Always land the final text, even if the last chunks were
            // throttled — otherwise the message stays truncated.
            Some(target) => {
                if pending_edit
                    && let Err(e) =
                        crate::groups::edit_group_message(&client, &group_id, target, &buffer).await
                {
                    tracing::warn!(account_id, "final streamed edit failed: {e}");
                }
                // Counted here rather than after the match: the initial publish
                // went through `send_group_message`, which records no metrics,
                // and the edits that follow are all the same logical message.
                #[cfg(feature = "metrics")]
                counter!(nostr_metrics::MESSAGES_SENT_TOTAL).increment(1);
            },
            // `send_text` records its own metrics, so counting again here
            // would double it — and an empty stream sent nothing at all.
            None if !buffer.trim().is_empty() => {
                self.send_text(account_id, to, &buffer, reply_to).await?;
            },
            None => {},
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
        crate::{
            config::NostrAccountConfig,
            groups::{GroupMessageKind, buzz_stream_message_kind, group_chat_kind},
            reply_ctx::ReplyContexts,
            state::AccountState,
        },
        tokio_util::sync::CancellationToken,
    };

    use super::*;

    /// Build an outbound adapter with one account whose config names `groups`.
    /// No relay connection is made — `resolve` is pure routing logic.
    fn outbound_with_groups(groups: Vec<String>) -> NostrOutbound {
        outbound_with_config(NostrAccountConfig {
            groups,
            ..Default::default()
        })
    }

    fn outbound_with_config(config: NostrAccountConfig) -> NostrOutbound {
        let keys = Keys::generate();
        let client = Client::new(keys.clone());
        let state = AccountState {
            client,
            keys,
            config: Arc::new(RwLock::new(config)),
            cached_allowlist: Arc::new(RwLock::new(Vec::new())),
            cancel: CancellationToken::new(),
            otp: Arc::new(std::sync::Mutex::new(moltis_channels::otp::OtpState::new(
                300,
            ))),
            reply_ctx: Arc::new(std::sync::Mutex::new(ReplyContexts::new())),
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

    /// Record an inbound message so the reply can mirror it.
    fn record_inbound(outbound: &NostrOutbound, group: &str, kind: Kind) -> (EventId, PublicKey) {
        let event_id = EventId::from_byte_array(Keys::generate().public_key().to_bytes());
        let author = Keys::generate().public_key();
        let accounts = outbound.accounts.read().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = accounts.get("acct") {
            let mut ctxs = state.reply_ctx.lock().unwrap_or_else(|e| e.into_inner());
            ctxs.record(event_id, group, author, kind);
        }
        (event_id, author)
    }

    /// A reply to a Buzz kind:40002 message must go back as kind:40002 — a
    /// Buzz client filtering v2 would never see a kind:9 answer.
    #[test]
    fn reply_mirrors_buzz_dialect_and_tags_author() {
        let outbound = outbound_with_groups(vec!["buzz-general".into()]);
        let (event_id, author) =
            record_inbound(&outbound, "buzz-general", buzz_stream_message_kind());

        let plan = outbound.plan_group_send("acct", "buzz-general", Some(&event_id.to_hex()));
        assert_eq!(plan.kind, buzz_stream_message_kind());
        assert_eq!(plan.mention, Some(author));
    }

    #[test]
    fn reply_mirrors_nip29_dialect() {
        let outbound = outbound_with_groups(vec!["grp".into()]);
        let (event_id, author) = record_inbound(&outbound, "grp", group_chat_kind());

        let plan = outbound.plan_group_send("acct", "grp", Some(&event_id.to_hex()));
        assert_eq!(plan.kind, group_chat_kind());
        assert_eq!(plan.mention, Some(author));
    }

    /// With no specific message to answer, fall back to the dialect last seen
    /// in that group rather than the configured default.
    #[test]
    fn non_reply_uses_dialect_learned_from_group() {
        let outbound = outbound_with_groups(vec!["buzz-general".into()]);
        record_inbound(&outbound, "buzz-general", buzz_stream_message_kind());

        let plan = outbound.plan_group_send("acct", "buzz-general", None);
        assert_eq!(plan.kind, buzz_stream_message_kind());
        // Nobody specific to notify on a proactive send.
        assert_eq!(plan.mention, None);
    }

    /// Cold start with nothing learned falls back to the configured default.
    #[test]
    fn cold_start_uses_configured_dialect() {
        let nip29 = outbound_with_groups(vec!["grp".into()]);
        assert_eq!(
            nip29.plan_group_send("acct", "grp", None).kind,
            group_chat_kind(),
            "default config is the interoperable kind:9"
        );

        let buzz = outbound_with_config(NostrAccountConfig {
            groups: vec!["grp".into()],
            group_message_kind: GroupMessageKind::BuzzV2,
            ..Default::default()
        });
        assert_eq!(
            buzz.plan_group_send("acct", "grp", None).kind,
            buzz_stream_message_kind(),
            "buzz_v2 config must post kind:40002"
        );
    }

    /// An unparseable or unknown reply id must not panic or mis-tag.
    #[test]
    fn unknown_reply_id_falls_back_without_mention() {
        let outbound = outbound_with_groups(vec!["grp".into()]);
        let plan = outbound.plan_group_send("acct", "grp", Some("not-an-event-id"));
        assert_eq!(plan.kind, group_chat_kind());
        assert_eq!(plan.mention, None);
    }

    /// Streaming edits are a Buzz extension; a plain NIP-29 group must fall
    /// back to collecting and sending once.
    #[test]
    fn only_buzz_dialect_streams_via_edits() {
        let outbound = outbound_with_groups(vec!["grp".into()]);
        record_inbound(&outbound, "grp", buzz_stream_message_kind());
        assert!(crate::groups::supports_edit(
            outbound.plan_group_send("acct", "grp", None).kind
        ));

        let plain = outbound_with_groups(vec!["grp".into()]);
        record_inbound(&plain, "grp", group_chat_kind());
        assert!(!crate::groups::supports_edit(
            plain.plan_group_send("acct", "grp", None).kind
        ));
    }

    /// Reacting to a DM would leak a gift-wrapped conversation, so it no-ops.
    #[tokio::test]
    async fn reactions_are_ignored_for_dm_targets() {
        let outbound = outbound_with_groups(vec!["grp".into()]);
        let peer = Keys::generate().public_key().to_hex();
        let event_id = EventId::from_byte_array(Keys::generate().public_key().to_bytes());
        let result = outbound
            .add_reaction("acct", &peer, &event_id.to_hex(), "eyes")
            .await;
        assert!(result.is_ok(), "DM reaction must be a silent no-op");
    }

    /// A malformed message id must not panic or publish anything.
    #[tokio::test]
    async fn reactions_ignore_unparseable_message_id() {
        let outbound = outbound_with_groups(vec!["grp".into()]);
        assert!(
            outbound
                .add_reaction("acct", "grp", "not-an-event-id", "eyes")
                .await
                .is_ok()
        );
        assert!(
            outbound
                .remove_reaction("acct", "grp", "not-an-event-id", "eyes")
                .await
                .is_ok()
        );
    }

    /// Retracting a reaction we never recorded is a no-op, not an error.
    #[tokio::test]
    async fn removing_unknown_reaction_is_noop() {
        let outbound = outbound_with_groups(vec!["grp".into()]);
        let event_id = EventId::from_byte_array(Keys::generate().public_key().to_bytes());
        let result = outbound
            .remove_reaction("acct", "grp", &event_id.to_hex(), "eyes")
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn unknown_account_is_unavailable() {
        let outbound = outbound_with_groups(vec!["buzz-general".into()]);
        let resolved = outbound.resolve("missing", "buzz-general").await;
        assert!(resolved.is_err(), "unknown account must not resolve");
    }
}
