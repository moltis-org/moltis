//! Outbound message sending for Nostr channels.
//!
//! Implements `ChannelOutbound` and `ChannelStreamOutbound`. A send target is
//! either a `group:`-prefixed NIP-29 group id — published as a plaintext group
//! chat message (kind:9, or kind:40002 in a Buzz channel) — or a bare pubkey,
//! sent as a NIP-59 gift-wrapped DM (kind:1059). The prefix decides the route,
//! so classification never depends on mutable config; membership is then
//! re-checked so a group removed from `groups` fails closed. See
//! [`crate::groups::GROUP_TARGET_PREFIX`].

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
    async fn plan_group_send(
        &self,
        account_id: &str,
        group_id: &str,
        reply_to: Option<&str>,
    ) -> GroupSendPlan {
        // Clone the handles out first: the accounts map is a std lock, whose
        // guard must not be held across the config await below.
        let handles = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            accounts
                .get(account_id)
                .map(|s| (Arc::clone(&s.reply_ctx), Arc::clone(&s.config)))
        };
        let Some((reply_ctx, config)) = handles else {
            return GroupSendPlan {
                kind: crate::groups::group_chat_kind(),
                mention: None,
            };
        };

        let reply_event = reply_to.and_then(|id| EventId::parse(id).ok());
        let (ctx, learned) = {
            let ctxs = reply_ctx.lock().unwrap_or_else(|e| e.into_inner());
            (
                reply_event.and_then(|id| ctxs.get(&id)),
                ctxs.kind_for_group(group_id),
            )
        };
        if let Some(ctx) = ctx {
            return GroupSendPlan {
                kind: ctx.kind,
                mention: Some(ctx.author),
            };
        }

        let kind = match learned {
            Some(kind) => kind,
            None => Kind::from(config.read().await.group_message_kind),
        };
        GroupSendPlan {
            kind,
            mention: None,
        }
    }
}

/// How many times to attempt publishing a reaction retraction.
///
/// The gateway acknowledges a message with 👀 and replaces it at the end of the
/// turn; if the retraction is dropped the two pile up and the 👀 never leaves.
/// Nothing upstream retries — the reaction worker exits once the turn is done —
/// so the attempts have to happen here.
const RETRACTION_ATTEMPTS: u32 = 3;

/// Delay before the second retraction attempt, doubled for each one after.
const RETRACTION_RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_millis(100);

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
                StreamEvent::TaskUpdate(_) => {},
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

    /// Authorize a group publish and perform it without releasing the config
    /// lock in between — see [`groups::authorized_publish`](crate::groups::authorized_publish),
    /// which is shared with the command replies `bus` publishes directly.
    ///
    /// Fails closed if the account was deleted mid-turn.
    async fn authorized_group_publish<T, F>(
        &self,
        account_id: &str,
        group_id: &str,
        publish: F,
    ) -> ChannelResult<T>
    where
        F: AsyncFnOnce() -> Result<T, crate::error::Error>,
    {
        let config = self.config_handle(account_id)?;
        Ok(crate::groups::authorized_publish(&config, group_id, publish).await?)
    }

    /// The account's shared config handle, with the accounts map lock released.
    ///
    /// Taken before awaiting on the config so the `std::sync::RwLock` guarding
    /// the map is never held across an `.await`.
    fn config_handle(&self, account_id: &str) -> ChannelResult<crate::state::SharedConfig> {
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        let state = accounts.get(account_id).ok_or_else(|| {
            moltis_channels::Error::unavailable(format!("nostr account not found: {account_id}"))
        })?;
        Ok(Arc::clone(&state.config))
    }

    /// Take back a streamed reply that can no longer be completed.
    ///
    /// A Buzz reply is published early and revised as tokens arrive, so when
    /// permission is withdrawn mid-answer the relay is left holding whatever
    /// had been written by then — frequently a partial sentence. Leaving it is
    /// not neutral: it stays in the channel as the bot's answer, and it is the
    /// one message the operator can no longer have corrected.
    ///
    /// Withdrawing it is consistent with retracting an acknowledgement after
    /// revocation ([`ChannelOutbound::remove_reaction`]): removing the bot's
    /// own content is the opposite of speaking, so the send gate does not
    /// apply. Best-effort — NIP-09 is advisory — and failure is logged rather
    /// than surfaced, since the caller is already returning the refusal that
    /// caused it.
    async fn withdraw_partial_reply(
        &self,
        account_id: &str,
        client: &Client,
        group_id: &str,
        published: EventId,
    ) {
        if let Err(e) = self
            .retract_event(account_id, client, group_id, published)
            .await
        {
            tracing::warn!(
                account_id,
                group = %group_id,
                "truncated reply left in the channel, retraction failed: {e}"
            );
        }
    }

    /// Withdraw something we published into a group: a NIP-09 deletion, with
    /// transient failures retried.
    ///
    /// Ungated on purpose, like the callers that use it. Taking the bot's own
    /// content back out of a channel is the opposite of speaking in it, so a
    /// revoked group must not block it — that is what leaves a stale 👀 or a
    /// half-finished answer sitting there permanently.
    ///
    /// Retried here because nothing upstream will: the gateway's reaction
    /// worker ends the turn right after these calls and never returns. Only
    /// retryable failures are repeated — a relay *rejecting* a deletion has
    /// made a decision, not stumbled.
    ///
    /// NIP-09 is advisory, so a published deletion is a request the relay may
    /// still ignore. This maximises the chance the content goes away; it cannot
    /// promise it.
    async fn retract_event(
        &self,
        account_id: &str,
        client: &Client,
        group_id: &str,
        event_id: EventId,
    ) -> ChannelResult<()> {
        let mut backoff = RETRACTION_RETRY_BACKOFF;
        let mut last_error = None;
        for attempt in 1..=RETRACTION_ATTEMPTS {
            match crate::groups::delete_event(client, group_id, event_id).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    let retry = e.is_retryable() && attempt < RETRACTION_ATTEMPTS;
                    tracing::debug!(account_id, attempt, retry, "retraction failed: {e}");
                    last_error = Some(e);
                    if !retry {
                        break;
                    }
                    tokio::time::sleep(backoff).await;
                    backoff *= 2;
                },
            }
        }
        Err(last_error.map_or_else(
            || moltis_channels::Error::unavailable("retraction failed"),
            Into::into,
        ))
    }

    /// Resolve `channel_id` to a group **without authorizing a send**, for the
    /// reaction paths.
    ///
    /// [`Self::resolve`] answers two questions at once — "is this a group?" and
    /// "may we publish to it?" — and reactions need them separated. A DM target
    /// is a silent no-op (reacting to a gift-wrapped conversation would reveal
    /// it happened), whereas a revoked group is a real refusal the caller must
    /// report; conflating them turns the second into the first.
    ///
    /// Separating them also lets a retraction run after revocation. Withdrawing
    /// the bot should remove its presence, not freeze its last 👀 on a user's
    /// message — so [`ChannelOutbound::remove_reaction`] uses this and skips
    /// the gate, while [`ChannelOutbound::add_reaction`] uses it and then
    /// applies [`Self::authorized_group_publish`] like every other publish.
    ///
    /// Only ever resolves a group, never a DM, so it cannot become a way to
    /// send anything to a pubkey. A `group:`-prefixed target resolves whatever
    /// the config now says, since the prefix already proves it is a group; a
    /// legacy unprefixed one must still name a configured group, because
    /// nothing else distinguishes it from a public key.
    async fn resolve_group_target(
        &self,
        account_id: &str,
        channel_id: &str,
    ) -> Option<(Client, String)> {
        let (client, config) = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            let state = accounts.get(account_id)?;
            (state.client.clone(), Arc::clone(&state.config))
        };
        if let Some(group_id) = crate::groups::parse_group_target(channel_id) {
            return Some((client, group_id.to_string()));
        }
        let is_legacy_group = config.read().await.groups.iter().any(|g| g == channel_id);
        is_legacy_group.then(|| (client, channel_id.to_string()))
    }

    /// Look up account state and resolve the target: a `group:`-prefixed id is
    /// a NIP-29 group send; anything else is parsed as a DM pubkey. Returns the
    /// group id with the prefix stripped — that bare value is what belongs in
    /// the `h` tag.
    async fn resolve(
        &self,
        account_id: &str,
        to: &str,
    ) -> ChannelResult<(Client, Keys, SendTarget)> {
        // Clone the handles out before awaiting on the config: the accounts map
        // is a std lock and its guard must not cross an await.
        let (client, keys, config) = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            let state = accounts.get(account_id).ok_or_else(|| {
                moltis_channels::Error::unavailable(format!(
                    "nostr account not found: {account_id}"
                ))
            })?;
            (
                state.client.clone(),
                state.keys.clone(),
                Arc::clone(&state.config),
            )
        };
        let cfg = config.read().await;

        // A prefixed target is a group, full stop — never reinterpret it as a
        // pubkey. Group participation is then re-checked so a group turned off
        // in the config fails closed rather than being delivered anywhere.
        if let Some(group_id) = crate::groups::parse_group_target(to) {
            crate::groups::check_group_send(&cfg.groups, &cfg.group_mention_mode, group_id)?;
            return Ok((client, keys, SendTarget::Group(group_id.to_string())));
        }

        // Legacy: a target persisted before group targets carried the prefix.
        // If it names a group this account is still in, read it as that group.
        // The competing reading — a DM to a peer whose pubkey happens to equal
        // a configured group id — is far less likely, and misrouting a channel
        // reply into a stranger's DM is the more damaging way to be wrong.
        //
        // An unprefixed target for a group that has *also* been removed stays
        // ambiguous: the string carries no type and is indistinguishable from a
        // pubkey. That combination cannot arise from any released build, since
        // group chat ships for the first time in this change.
        let legacy_group_target = cfg.groups.iter().any(|g| g == to);
        if legacy_group_target {
            crate::groups::check_group_send(&cfg.groups, &cfg.group_mention_mode, to)?;
            tracing::debug!(
                account_id,
                group = to,
                "routing unprefixed legacy group target"
            );
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
                // Resolved before the guard is taken: `plan_group_send` reads
                // the config itself, and a nested read under a waiting writer
                // would deadlock.
                let plan = self.plan_group_send(account_id, &group_id, reply_to).await;
                self.authorized_group_publish(account_id, &group_id, async || {
                    crate::groups::send_group_message(
                        &client,
                        plan.kind,
                        &group_id,
                        text,
                        reply_event,
                        plan.mention,
                    )
                    .await
                    .inspect_err(|_e| {
                        #[cfg(feature = "metrics")]
                        counter!(nostr_metrics::MESSAGE_SEND_ERRORS_TOTAL, "reason" => "group")
                            .increment(1);
                    })
                })
                .await
                .inspect_err(|e| {
                    tracing::warn!(
                        account_id,
                        group = %group_id,
                        "group reply not published: {e}"
                    );
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
        // `channel_id` is the reply target, i.e. the *prefixed* `group:<id>`.
        // The `h` tag must carry the bare id, or a NIP-29 relay refuses the
        // write / files it under a group nobody watches.
        let Some((client, group_id)) = self.resolve_group_target(account_id, channel_id).await
        else {
            return Ok(());
        };
        let Ok(target) = EventId::parse(message_id) else {
            return Ok(());
        };

        let glyph = crate::groups::ack_emoji_glyph(emoji);

        // Clear any earlier acknowledgement whose retraction did not go
        // through. This is the retry path for `remove_reaction`: nothing else
        // revisits it, and we are about to talk to the relay anyway. Best
        // effort — if it fails again the id stays for the next attempt.
        let stale = {
            let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
            accounts.get(account_id).map_or_else(Vec::new, |state| {
                let ctxs = state.reply_ctx.lock().unwrap_or_else(|e| e.into_inner());
                ctxs.stale_reactions(target, glyph)
            })
        };
        for (stale_glyph, stale_id) in stale {
            if self
                .retract_event(account_id, &client, &group_id, stale_id)
                .await
                .is_ok()
            {
                let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
                if let Some(state) = accounts.get(account_id) {
                    let mut ctxs = state.reply_ctx.lock().unwrap_or_else(|e| e.into_inner());
                    ctxs.forget_reaction(target, &stale_glyph);
                }
            }
        }

        // A reaction is a publish into the group like any other, so it goes
        // through the same authorization-holding gate rather than trusting the
        // decision `resolve` made.
        let reaction = self
            .authorized_group_publish(account_id, &group_id, async || {
                crate::groups::send_reaction(&client, &group_id, target, glyph).await
            })
            .await?;

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
    ///
    /// Deliberately routed around the send authorization gate, unlike every
    /// other publish here — see [`Self::resolve_group_target`]. This
    /// one only ever withdraws something already in the channel, so refusing it
    /// after a revocation would leave the bot's 👀 stuck on a user's message
    /// forever, which is the opposite of what withdrawing the bot should do.
    async fn remove_reaction(
        &self,
        account_id: &str,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> ChannelResult<()> {
        let Some((client, group_id)) = self.resolve_group_target(account_id, channel_id).await
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
                let ctxs = state.reply_ctx.lock().unwrap_or_else(|e| e.into_inner());
                ctxs.reaction_id(target, glyph)
            })
        };
        let Some(reaction) = reaction else {
            return Ok(());
        };

        self.retract_event(account_id, &client, &group_id, reaction)
            .await?;
        // Forgotten only once the deletion is published: this is the sole
        // record of the reaction, so dropping it any earlier would leave a
        // failed retraction with nothing to retry from.
        let accounts = self.accounts.read().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = accounts.get(account_id) {
            let mut ctxs = state.reply_ctx.lock().unwrap_or_else(|e| e.into_inner());
            ctxs.forget_reaction(target, glyph);
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
                let plan = self.plan_group_send(account_id, &group_id, reply_to).await;
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
                StreamEvent::TaskUpdate(_) => {},
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
                    let published_id = self
                        .authorized_group_publish(account_id, &group_id, async || {
                            crate::groups::send_group_message(
                                &client,
                                plan.kind,
                                &group_id,
                                &buffer,
                                reply_event,
                                plan.mention,
                            )
                            .await
                        })
                        .await;
                    match published_id {
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
                        let result = self
                            .authorized_group_publish(account_id, &group_id, async || {
                                crate::groups::edit_group_message(
                                    &client, &group_id, target, &buffer,
                                )
                                .await
                            })
                            .await;
                        if let Err(ref e) = result {
                            tracing::warn!(account_id, "streamed edit not published: {e}");
                            // Revoked mid-answer: the reply is frozen partway
                            // through a sentence, and leaving it there is its
                            // own kind of speech. Withdraw it and stop.
                            self.withdraw_partial_reply(account_id, &client, &group_id, target)
                                .await;
                            return Err(result.err().unwrap_or_else(|| {
                                moltis_channels::Error::unavailable("group send revoked")
                            }));
                        }
                        last_edit = tokio::time::Instant::now();
                        // Keep the flag set when the edit failed: it means
                        // "the buffer has moved on since what the relay holds",
                        // so clearing it here would skip the final edit and
                        // leave the reply permanently truncated.
                        pending_edit = result.is_err();
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
                let revoked = if pending_edit {
                    self.authorized_group_publish(account_id, &group_id, async || {
                        crate::groups::edit_group_message(&client, &group_id, target, &buffer).await
                    })
                    .await
                    .inspect_err(|e| {
                        tracing::warn!(
                            account_id,
                            group = %group_id,
                            "final streamed edit not published: {e}"
                        );
                    })
                    .err()
                } else {
                    None
                };
                if revoked.is_some() {
                    // The published text is missing everything after the last
                    // successful edit, and no further edit is permitted, so it
                    // can never be completed. Take it back rather than leave a
                    // half-answer attributed to the bot.
                    self.withdraw_partial_reply(account_id, &client, &group_id, target)
                        .await;
                }
                // Counted here rather than after the match: the initial publish
                // went through `send_group_message`, which records no metrics,
                // and the edits that follow are all the same logical message.
                // Counted even when the final edit was refused — the message
                // itself did reach the relay, only its tail was withheld.
                #[cfg(feature = "metrics")]
                counter!(nostr_metrics::MESSAGES_SENT_TOTAL).increment(1);
                if let Some(e) = revoked {
                    return Err(e);
                }
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
            config: Arc::new(tokio::sync::RwLock::new(config)),
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
        let resolved = outbound
            .resolve("acct", &crate::groups::group_target("buzz-general"))
            .await;
        assert!(matches!(resolved, Ok((_, _, SendTarget::Group(ref g))) if g == "buzz-general"));
    }

    /// A target persisted before group targets carried the `group:` prefix must
    /// still reach the group, not be parsed as a pubkey — even when the group
    /// id is itself pubkey-shaped, which is exactly when the DM fallback would
    /// have leaked channel content to a stranger.
    #[tokio::test]
    async fn legacy_unprefixed_target_still_routes_to_a_configured_group() {
        let pubkey_shaped_id = Keys::generate().public_key().to_hex();
        let outbound = outbound_with_groups(vec![pubkey_shaped_id.clone()]);

        // No `group:` prefix — as a pre-upgrade session binding would have it.
        let resolved = outbound.resolve("acct", &pubkey_shaped_id).await;

        assert!(
            matches!(resolved, Ok((_, _, SendTarget::Group(ref g))) if *g == pubkey_shaped_id),
            "a legacy target naming a joined group must route to that group"
        );
    }

    /// The legacy path must not swallow ordinary DMs: a pubkey that is not a
    /// configured group still resolves as a DM recipient.
    #[tokio::test]
    async fn legacy_path_does_not_capture_ordinary_dms() {
        let outbound = outbound_with_groups(vec!["buzz-general".into()]);
        let peer = Keys::generate().public_key().to_hex();
        let resolved = outbound.resolve("acct", &peer).await;
        assert!(matches!(resolved, Ok((_, _, SendTarget::Dm(_)))));
    }

    #[tokio::test]
    async fn resolves_pubkey_as_dm_send() {
        let outbound = outbound_with_groups(vec!["buzz-general".into()]);
        let peer = Keys::generate().public_key().to_hex();
        let resolved = outbound.resolve("acct", &peer).await;
        assert!(matches!(resolved, Ok((_, _, SendTarget::Dm(_)))));
    }

    /// Turning group chat off means clearing `groups`, and outbound reads that
    /// same list — so a queued reply cannot keep publishing to a group the
    /// operator has since removed.
    #[tokio::test]
    async fn refuses_group_send_once_groups_cleared() {
        let outbound = outbound_with_groups(Vec::new());
        let resolved = outbound
            .resolve("acct", &crate::groups::group_target("buzz-general"))
            .await;
        assert!(
            resolved.is_err(),
            "group send must fail once the group is no longer configured"
        );
    }

    /// A group id may be 64 hex characters, which is also exactly what a public
    /// key looks like. If routing fell back to pubkey parsing for a removed
    /// group, the channel's reply would be gift-wrapped to whoever owns that
    /// key — so a stale group target must fail, never become a DM.
    #[tokio::test]
    async fn removed_group_whose_id_is_a_valid_pubkey_never_becomes_a_dm() {
        // A real, parseable public key used as the group's `h` tag.
        let pubkey_shaped_id = Keys::generate().public_key().to_hex();
        assert!(
            PublicKey::parse(&pubkey_shaped_id).is_ok(),
            "precondition: the group id really is a valid pubkey"
        );

        // Group removed from config while a reply target still points at it.
        let outbound = outbound_with_groups(Vec::new());
        let resolved = outbound
            .resolve("acct", &crate::groups::group_target(&pubkey_shaped_id))
            .await;

        assert!(
            !matches!(resolved, Ok((_, _, SendTarget::Dm(_)))),
            "a removed group must never be reinterpreted as a DM recipient"
        );
        assert!(resolved.is_err(), "it must fail closed");
    }

    /// The same id, still configured, routes as a group — proving the guard
    /// above rejects because of removal, not because of the id's shape.
    #[tokio::test]
    async fn pubkey_shaped_group_id_still_routes_as_a_group_when_configured() {
        let pubkey_shaped_id = Keys::generate().public_key().to_hex();
        let outbound = outbound_with_groups(vec![pubkey_shaped_id.clone()]);
        let resolved = outbound
            .resolve("acct", &crate::groups::group_target(&pubkey_shaped_id))
            .await;
        assert!(
            matches!(resolved, Ok((_, _, SendTarget::Group(ref g))) if *g == pubkey_shaped_id),
            "a configured group keeps routing as a group even when its id parses as a pubkey"
        );
    }

    /// `group_mention_mode = "none"` is documented as receive-only, so it has to
    /// stop the bot publishing as well as stop it answering. A reply target
    /// outlives the config that produced it, so a queued or resumed turn would
    /// otherwise keep talking in a channel the operator has withdrawn from.
    #[tokio::test]
    async fn refuses_group_send_when_mention_mode_is_none() {
        let outbound = outbound_with_config(NostrAccountConfig {
            groups: vec!["grp".into()],
            group_mention_mode: moltis_channels::gating::MentionMode::None,
            ..Default::default()
        });
        let resolved = outbound
            .resolve("acct", &crate::groups::group_target("grp"))
            .await;
        assert!(
            resolved.is_err(),
            "receive-only must not publish into the group"
        );

        // A DM to the same account is unaffected — the mode is about groups.
        let peer = Keys::generate().public_key().to_hex();
        assert!(matches!(
            outbound.resolve("acct", &peer).await,
            Ok((_, _, SendTarget::Dm(_)))
        ));
    }

    /// The legacy unprefixed path must not bypass the same gate.
    #[tokio::test]
    async fn legacy_target_is_refused_when_mention_mode_is_none() {
        let outbound = outbound_with_config(NostrAccountConfig {
            groups: vec!["grp".into()],
            group_mention_mode: moltis_channels::gating::MentionMode::None,
            ..Default::default()
        });
        assert!(outbound.resolve("acct", "grp").await.is_err());
    }

    /// The remaining modes keep publishing — the gate is about `none` only.
    #[tokio::test]
    async fn other_mention_modes_still_publish() {
        for mode in [
            moltis_channels::gating::MentionMode::Mention,
            moltis_channels::gating::MentionMode::Always,
        ] {
            let outbound = outbound_with_config(NostrAccountConfig {
                groups: vec!["grp".into()],
                group_mention_mode: mode.clone(),
                ..Default::default()
            });
            let resolved = outbound
                .resolve("acct", &crate::groups::group_target("grp"))
                .await;
            assert!(
                matches!(resolved, Ok((_, _, SendTarget::Group(_)))),
                "{mode:?} must still be able to reply"
            );
        }
    }

    /// Overwrite the live account config, as saving channel settings does.
    async fn rewrite_config(outbound: &NostrOutbound, config: NostrAccountConfig) {
        let handle = {
            let accounts = outbound.accounts.read().unwrap_or_else(|e| e.into_inner());
            accounts.get("acct").map(|s| Arc::clone(&s.config))
        };
        if let Some(handle) = handle {
            *handle.write().await = config;
        }
    }

    /// Run the authorization gate on its own, with a publish that does nothing —
    /// the check is what is under test, not the relay round trip.
    async fn gate(outbound: &NostrOutbound, group_id: &str) -> ChannelResult<()> {
        outbound
            .authorized_group_publish("acct", group_id, async || Ok(()))
            .await
    }

    /// A streamed reply resolves its group once and then publishes repeatedly
    /// over the life of the turn, so authorization has to be re-read per
    /// publish: `plugin::upsert_account` rewrites this very config in place,
    /// and a turn that started while the group was joined must not keep
    /// publishing after the operator withdrew the bot from it.
    #[tokio::test]
    async fn revoking_a_group_mid_stream_refuses_further_publishes() {
        let outbound = outbound_with_groups(vec!["grp".into()]);
        assert!(
            gate(&outbound, "grp").await.is_ok(),
            "authorized while the group is joined"
        );

        // Operator removes the group while the turn is still generating.
        rewrite_config(&outbound, NostrAccountConfig::default()).await;
        assert!(
            gate(&outbound, "grp").await.is_err(),
            "removing the group must stop an in-flight stream"
        );

        // Switching the group to receive-only must stop it too.
        rewrite_config(&outbound, NostrAccountConfig {
            groups: vec!["grp".into()],
            group_mention_mode: moltis_channels::gating::MentionMode::None,
            ..Default::default()
        })
        .await;
        assert!(
            gate(&outbound, "grp").await.is_err(),
            "receive-only must stop an in-flight stream"
        );
    }

    /// Deleting the account mid-stream leaves the stream holding a client for
    /// state that no longer exists; it must fail closed rather than publish.
    #[tokio::test]
    async fn removing_the_account_mid_stream_refuses_further_publishes() {
        let outbound = outbound_with_groups(vec!["grp".into()]);
        outbound
            .accounts
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove("acct");
        assert!(gate(&outbound, "grp").await.is_err());
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
    #[tokio::test]
    async fn reply_mirrors_buzz_dialect_and_tags_author() {
        let outbound = outbound_with_groups(vec!["buzz-general".into()]);
        let (event_id, author) =
            record_inbound(&outbound, "buzz-general", buzz_stream_message_kind());

        let plan = outbound
            .plan_group_send("acct", "buzz-general", Some(&event_id.to_hex()))
            .await;
        assert_eq!(plan.kind, buzz_stream_message_kind());
        assert_eq!(plan.mention, Some(author));
    }

    #[tokio::test]
    async fn reply_mirrors_nip29_dialect() {
        let outbound = outbound_with_groups(vec!["grp".into()]);
        let (event_id, author) = record_inbound(&outbound, "grp", group_chat_kind());

        let plan = outbound
            .plan_group_send("acct", "grp", Some(&event_id.to_hex()))
            .await;
        assert_eq!(plan.kind, group_chat_kind());
        assert_eq!(plan.mention, Some(author));
    }

    /// With no specific message to answer, fall back to the dialect last seen
    /// in that group rather than the configured default.
    #[tokio::test]
    async fn non_reply_uses_dialect_learned_from_group() {
        let outbound = outbound_with_groups(vec!["buzz-general".into()]);
        record_inbound(&outbound, "buzz-general", buzz_stream_message_kind());

        let plan = outbound.plan_group_send("acct", "buzz-general", None).await;
        assert_eq!(plan.kind, buzz_stream_message_kind());
        // Nobody specific to notify on a proactive send.
        assert_eq!(plan.mention, None);
    }

    /// Cold start with nothing learned falls back to the configured default.
    #[tokio::test]
    async fn cold_start_uses_configured_dialect() {
        let nip29 = outbound_with_groups(vec!["grp".into()]);
        assert_eq!(
            nip29.plan_group_send("acct", "grp", None).await.kind,
            group_chat_kind(),
            "default config is the interoperable kind:9"
        );

        let buzz = outbound_with_config(NostrAccountConfig {
            groups: vec!["grp".into()],
            group_message_kind: GroupMessageKind::BuzzV2,
            ..Default::default()
        });
        assert_eq!(
            buzz.plan_group_send("acct", "grp", None).await.kind,
            buzz_stream_message_kind(),
            "buzz_v2 config must post kind:40002"
        );
    }

    /// An unparseable or unknown reply id must not panic or mis-tag.
    #[tokio::test]
    async fn unknown_reply_id_falls_back_without_mention() {
        let outbound = outbound_with_groups(vec!["grp".into()]);
        let plan = outbound
            .plan_group_send("acct", "grp", Some("not-an-event-id"))
            .await;
        assert_eq!(plan.kind, group_chat_kind());
        assert_eq!(plan.mention, None);
    }

    /// Streaming edits are a Buzz extension; a plain NIP-29 group must fall
    /// back to collecting and sending once.
    #[tokio::test]
    async fn only_buzz_dialect_streams_via_edits() {
        let outbound = outbound_with_groups(vec!["grp".into()]);
        record_inbound(&outbound, "grp", buzz_stream_message_kind());
        assert!(crate::groups::supports_edit(
            outbound.plan_group_send("acct", "grp", None).await.kind
        ));

        let plain = outbound_with_groups(vec!["grp".into()]);
        record_inbound(&plain, "grp", group_chat_kind());
        assert!(!crate::groups::supports_edit(
            plain.plan_group_send("acct", "grp", None).await.kind
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

        // A revoked group is a different thing entirely: not-applicable versus
        // refused. Collapsing the two would hide a dropped acknowledgement.
        rewrite_config(&outbound, NostrAccountConfig::default()).await;
        assert!(
            outbound
                .add_reaction(
                    "acct",
                    &crate::groups::group_target("grp"),
                    &event_id.to_hex(),
                    "eyes"
                )
                .await
                .is_err(),
            "a revoked group must report the refusal, not look like a no-op"
        );
    }

    /// A malformed message id must not panic or publish anything.
    #[tokio::test]
    async fn reactions_ignore_unparseable_message_id() {
        let outbound = outbound_with_groups(vec!["grp".into()]);
        assert!(
            outbound
                .add_reaction(
                    "acct",
                    &crate::groups::group_target("grp"),
                    "not-an-event-id",
                    "eyes"
                )
                .await
                .is_ok()
        );
        assert!(
            outbound
                .remove_reaction(
                    "acct",
                    &crate::groups::group_target("grp"),
                    "not-an-event-id",
                    "eyes"
                )
                .await
                .is_ok()
        );
    }

    /// A retraction that cannot be published must say so and keep the id.
    ///
    /// Reporting success here would be the worst outcome: the gateway's
    /// reaction worker finishes the turn straight after this call, so a failure
    /// swallowed as `Ok` leaves the 👀 on the user's message permanently, with
    /// the only record of it thrown away. This outbound has no relay, so every
    /// attempt fails.
    #[tokio::test]
    async fn a_retraction_that_cannot_be_published_reports_it_and_keeps_the_id() {
        let outbound = outbound_with_groups(vec!["grp".into()]);
        let target = EventId::from_byte_array(Keys::generate().public_key().to_bytes());
        let reaction = EventId::from_byte_array(Keys::generate().public_key().to_bytes());
        {
            let accounts = outbound.accounts.read().unwrap_or_else(|e| e.into_inner());
            if let Some(state) = accounts.get("acct") {
                let mut ctxs = state.reply_ctx.lock().unwrap_or_else(|e| e.into_inner());
                ctxs.record_reaction(target, "\u{1F440}", reaction);
            }
        }

        let result = outbound
            .remove_reaction(
                "acct",
                &crate::groups::group_target("grp"),
                &target.to_hex(),
                "eyes",
            )
            .await;
        assert!(
            result.is_err(),
            "an unpublishable retraction must not report success"
        );

        // The id survives, so a later attempt still has something to work from.
        let accounts = outbound.accounts.read().unwrap_or_else(|e| e.into_inner());
        let state = accounts.get("acct").unwrap_or_else(|| unreachable!());
        let ctxs = state.reply_ctx.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(
            ctxs.reaction_id(target, "\u{1F440}"),
            Some(reaction),
            "a failed retraction must stay retryable"
        );
    }

    /// Retracting a reaction we never recorded is a no-op, not an error.
    #[tokio::test]
    async fn removing_unknown_reaction_is_noop() {
        let outbound = outbound_with_groups(vec!["grp".into()]);
        let event_id = EventId::from_byte_array(Keys::generate().public_key().to_bytes());
        let result = outbound
            .remove_reaction(
                "acct",
                &crate::groups::group_target("grp"),
                &event_id.to_hex(),
                "eyes",
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn unknown_account_is_unavailable() {
        let outbound = outbound_with_groups(vec!["buzz-general".into()]);
        let resolved = outbound
            .resolve("missing", &crate::groups::group_target("buzz-general"))
            .await;
        assert!(resolved.is_err(), "unknown account must not resolve");
    }
}
