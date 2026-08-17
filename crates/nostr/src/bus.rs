//! Nostr relay subscription loop — inbound DM and group-chat pipeline.
//!
//! Subscribes to kind:4 (NIP-04 encrypted DMs) and kind:1059 (NIP-59 gift
//! wraps) addressed to the bot's pubkey, plus group chat (kind:9 NIP-29 and
//! kind:40002 Buzz) scoped to the configured groups via the `h` tag. Events flow
//! through dedup, self-message filtering, access control, and (for DMs)
//! decryption/unwrapping before being dispatched to the gateway via
//! `ChannelEventSink`.

use std::sync::{Arc, RwLock};

use {
    nostr_sdk::prelude::{
        Client, Event, Filter, Keys, Kind, PublicKey, RelayPoolNotification, Timestamp, ToBech32,
        nip04, nip44,
    },
    tokio_util::sync::CancellationToken,
};

use crate::{
    access::{self, AccessDenied},
    seen::SeenTracker,
    state::{SharedConfig, SharedOtp, SharedReplyContexts},
};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, nostr as nostr_metrics};

/// Maximum plaintext message size in bytes.
const MAX_MESSAGE_BYTES: usize = 64 * 1024;

/// Message sent to non-allowlisted senders as an OTP challenge prompt.
const OTP_CHALLENGE_MSG: &str = "You are not on the allowlist. A PIN challenge has been sent to the admin. Reply with the 6-digit code to gain access.";

/// Run the relay subscription loop for a single Nostr account.
///
/// Subscribes to NIP-04 DMs (kind:4) and NIP-59 gift wraps (kind:1059)
/// targeted at `bot_pubkey` and dispatches inbound messages to the gateway.
/// Runs until `cancel` is triggered or the relay pool shuts down (which
/// triggers an auto-disable request).
pub async fn run_subscription_loop(
    client: Client,
    keys: Keys,
    config: SharedConfig,
    cached_allowlist: Arc<RwLock<Vec<PublicKey>>>,
    otp: SharedOtp,
    reply_ctx: SharedReplyContexts,
    account_id: String,
    event_sink: Arc<dyn moltis_channels::ChannelEventSink>,
    cancel: CancellationToken,
) {
    let bot_pubkey = keys.public_key();
    let now_secs =
        u64::try_from(::time::OffsetDateTime::now_utc().unix_timestamp()).unwrap_or_default();
    let since = Timestamp::from(now_secs);

    // Two separate filters: kind:4 uses `since` (now), kind:1059 uses a wider
    // window because gift wrap timestamps are randomly tweaked by 0–2 days.
    let gift_wrap_since =
        Timestamp::from(now_secs.saturating_sub(crate::gift_wrap::TIMESTAMP_WINDOW_SECS));

    let nip04_filter = Filter::new()
        .kind(Kind::EncryptedDirectMessage)
        .pubkey(bot_pubkey)
        .since(since);

    let gift_wrap_filter = Filter::new()
        .kind(Kind::GiftWrap)
        .pubkey(bot_pubkey)
        .since(gift_wrap_since);

    let npub = bot_pubkey
        .to_bech32()
        .unwrap_or_else(|_| bot_pubkey.to_hex());
    tracing::info!(
        account_id,
        pubkey = %npub,
        "starting Nostr DM subscription (kind:4 + kind:1059)"
    );

    let mut seen = SeenTracker::new();

    let mut filters = vec![nip04_filter, gift_wrap_filter];

    // NIP-29 group chat (Buzz channels) — subscribe to kind:9 scoped to the
    // configured groups via the `h` tag. Relay subscriptions are fixed at
    // connect, so group membership changes take effect on account restart.
    let group_ids: Vec<String> = {
        let cfg = config.read().await;
        cfg.groups.clone()
    };
    if !group_ids.is_empty() {
        // Both dialects: standard NIP-29 kind:9 and Buzz kind:40002. A client
        // filtering only one never sees the other, so read both.
        let group_filter = Filter::new()
            .kinds(crate::groups::group_message_kinds())
            .custom_tags(crate::groups::h_tag(), group_ids.clone())
            .since(since);
        filters.push(group_filter);
        tracing::info!(
            account_id,
            groups = ?group_ids,
            "subscribing to group chat (kind:9 + kind:40002)"
        );
    }

    for filter in filters {
        if let Err(e) = client.subscribe(filter, None).await {
            tracing::error!(account_id, "failed to subscribe: {e}");
            return;
        }
    }

    let mut notifications = client.notifications();

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                tracing::info!(account_id, "Nostr subscription cancelled");
                break;
            }
            notification = notifications.recv() => {
                match notification {
                    Ok(RelayPoolNotification::Event { event, .. }) => {
                        handle_event(
                            &event,
                            &client,
                            &keys,
                            &bot_pubkey,
                            since,
                            &mut seen,
                            &config,
                            &cached_allowlist,
                            &otp,
                            &reply_ctx,
                            &account_id,
                            &event_sink,
                        ).await;
                    }
                    Ok(RelayPoolNotification::Shutdown) => {
                        tracing::warn!(account_id, "relay pool shutdown — requesting account disable");
                        event_sink
                            .request_disable_account("nostr", &account_id, "relay pool shutdown")
                            .await;
                        break;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(account_id, "notification channel error: {e}");
                    }
                }
            }
        }
    }

    tracing::info!(account_id, "Nostr subscription loop exited");
}

/// Process a single inbound event through the pipeline.
#[allow(clippy::too_many_arguments)]
async fn handle_event(
    event: &Event,
    client: &Client,
    keys: &Keys,
    bot_pubkey: &PublicKey,
    since: Timestamp,
    seen: &mut SeenTracker,
    config: &SharedConfig,
    cached_allowlist: &Arc<RwLock<Vec<PublicKey>>>,
    otp: &SharedOtp,
    reply_ctx: &SharedReplyContexts,
    account_id: &str,
    event_sink: &Arc<dyn moltis_channels::ChannelEventSink>,
) {
    // 0. Route group chat (kind:9 / kind:40002) separately from encrypted DMs.
    if crate::groups::is_group_message_kind(event.kind) {
        handle_group_event(
            event,
            client,
            bot_pubkey,
            since,
            seen,
            config,
            cached_allowlist,
            reply_ctx,
            account_id,
            event_sink,
        )
        .await;
        return;
    }

    // 1. Kind gate — accept kind:4 (legacy NIP-04) and kind:1059 (NIP-59 gift wrap)
    let is_gift_wrap = event.kind == Kind::GiftWrap;
    if event.kind != Kind::EncryptedDirectMessage && !is_gift_wrap {
        return;
    }

    // 2. Dedup check
    if seen.check_and_insert(&event.id) {
        return;
    }

    // 3. Extract sender, plaintext, and created_at based on kind.
    //    Gift wraps hide the real sender inside the sealed rumor; kind:4
    //    exposes it as event.pubkey.
    let (sender_pubkey, plaintext, event_time) = if is_gift_wrap {
        match crate::gift_wrap::unwrap_gift_wrap(keys, event).await {
            Ok(result) => result,
            Err(e) => {
                #[cfg(feature = "metrics")]
                counter!(nostr_metrics::DECRYPT_ERRORS_TOTAL).increment(1);
                tracing::warn!(
                    account_id,
                    event_id = %event.id,
                    "gift unwrap failed: {e}"
                );
                return;
            },
        }
    } else {
        // Kind:4 — decrypt NIP-04, fall back to NIP-44
        let text = match try_decrypt(keys, &event.pubkey, &event.content) {
            Some(t) => t,
            None => {
                #[cfg(feature = "metrics")]
                counter!(nostr_metrics::DECRYPT_ERRORS_TOTAL).increment(1);
                tracing::warn!(
                    account_id,
                    event_id = %event.id,
                    "decrypt failed (NIP-04 and NIP-44)"
                );
                return;
            },
        };
        (event.pubkey, text, event.created_at)
    };

    // 4. Skip self-messages
    if sender_pubkey == *bot_pubkey {
        return;
    }

    // 5. Skip stale events (gift wraps use rumor's created_at, not the
    //    randomly tweaked outer timestamp)
    if event_time < since {
        return;
    }

    let sender_hex = sender_pubkey.to_hex();
    let sender_npub = sender_pubkey
        .to_bech32()
        .unwrap_or_else(|_| sender_hex.clone());

    // 6. Read config fields (drop guard before any .await).
    let (dm_policy, otp_self_approval) = {
        let cfg = config.read().await;
        (cfg.dm_policy.clone(), cfg.otp_self_approval)
    };

    // 6a. OTP verification — if this sender has a pending challenge, check
    //     if the plaintext is a 6-digit code and verify it.
    //     This must run BEFORE the access-control gate because the sender is
    //     not yet on the allowlist when they reply with the OTP code.
    let has_pending = {
        let guard = otp.lock().unwrap_or_else(|e| e.into_inner());
        guard.has_pending(&sender_hex)
    };
    if has_pending {
        let trimmed = plaintext.trim();
        if trimmed.len() == 6 && trimmed.chars().all(|c| c.is_ascii_digit()) {
            handle_otp_verification(
                otp,
                client,
                keys,
                &sender_pubkey,
                account_id,
                &sender_hex,
                &sender_npub,
                trimmed,
                event_sink,
            )
            .await;
            return;
        }
        // Non-code reply while challenge pending — silently ignore.
        return;
    }

    // 6b. Normal access-control gate (scope the guard so it drops before any .await).
    let access_result = {
        let allowed = cached_allowlist.read().unwrap_or_else(|e| e.into_inner());
        access::check_dm_access(&sender_pubkey, &dm_policy, &allowed)
    };

    match &access_result {
        Ok(()) => {},
        Err(AccessDenied::Disabled) => {
            #[cfg(feature = "metrics")]
            counter!(nostr_metrics::ACCESS_CONTROL_DENIALS_TOTAL, "reason" => "disabled")
                .increment(1);
            tracing::debug!(account_id, sender = sender_hex, "DM rejected: disabled");
            return;
        },
        Err(AccessDenied::NotAllowlisted) => {
            #[cfg(feature = "metrics")]
            counter!(nostr_metrics::ACCESS_CONTROL_DENIALS_TOTAL, "reason" => "not_allowlisted")
                .increment(1);
            if otp_self_approval {
                handle_otp_challenge(
                    client,
                    keys,
                    &sender_pubkey,
                    otp,
                    account_id,
                    &sender_hex,
                    &sender_npub,
                    event_sink,
                )
                .await;
            } else {
                tracing::debug!(
                    account_id,
                    sender = sender_hex,
                    "DM rejected: not allowlisted"
                );
            }
            return;
        },
    }

    // 7. Size validation — truncate at a safe UTF-8 boundary
    let text = if plaintext.len() > MAX_MESSAGE_BYTES {
        tracing::warn!(account_id, len = plaintext.len(), "DM exceeds size limit");
        &plaintext[..plaintext.floor_char_boundary(MAX_MESSAGE_BYTES)]
    } else {
        &plaintext
    };

    // 8. Emit inbound event
    #[cfg(feature = "metrics")]
    counter!(nostr_metrics::MESSAGES_RECEIVED_TOTAL).increment(1);

    event_sink
        .emit(moltis_channels::ChannelEvent::InboundMessage {
            channel_type: moltis_channels::ChannelType::Nostr,
            account_id: account_id.to_string(),
            peer_id: sender_hex.clone(),
            username: Some(sender_npub.clone()),
            sender_name: None,
            message_count: None,
            access_granted: true,
        })
        .await;

    // 9. Intercept slash commands before dispatching to the LLM.
    let reply_to = moltis_channels::ChannelReplyTarget {
        ack_message_id: None,
        channel_type: moltis_channels::ChannelType::Nostr,
        account_id: account_id.to_string(),
        chat_id: sender_hex.clone(),
        message_id: Some(event.id.to_hex()),
        thread_id: None,
    };

    if let Some(cmd_text) = text.strip_prefix('/') {
        let cmd_name = cmd_text.split_whitespace().next().unwrap_or("");
        if moltis_channels::commands::is_channel_command(cmd_name, cmd_text) {
            let response = if cmd_name == "help" {
                Ok(moltis_channels::commands::help_text())
            } else {
                event_sink
                    .dispatch_command(cmd_text, reply_to, Some(&sender_hex))
                    .await
            };
            let reply_text = match response {
                Ok(msg) => msg,
                Err(e) => format!("Error: {e}"),
            };
            if let Err(e) =
                crate::gift_wrap::send_gift_wrapped_dm(client, keys, &sender_pubkey, &reply_text)
                    .await
            {
                tracing::warn!(account_id, "failed to send command response: {e}");
            }
            return;
        }
    }

    // 10. Dispatch to gateway
    let meta = moltis_channels::ChannelMessageMeta {
        channel_type: moltis_channels::ChannelType::Nostr,
        sender_name: None,
        username: Some(sender_npub),
        sender_id: Some(sender_hex.clone()),
        message_kind: Some(moltis_channels::ChannelMessageKind::Text),
        model: None,
        agent_id: None,
        audio_filename: None,
        documents: None,
    };

    event_sink.dispatch_to_chat(text, reply_to, meta).await;
}

/// Process an inbound group chat message (`kind:9` NIP-29 or `kind:40002`
/// Buzz, e.g. a Buzz channel).
///
/// Group messages are plaintext (the relay enforces membership), so there is
/// no decryption or OTP flow. The event is deduped, filtered for self-messages
/// and staleness, gated by group membership and mention mode, then dispatched
/// to the gateway with the group id as the session/chat id so replies route
/// back to the group.
#[allow(clippy::too_many_arguments)]
async fn handle_group_event(
    event: &Event,
    client: &Client,
    bot_pubkey: &PublicKey,
    since: Timestamp,
    seen: &mut SeenTracker,
    config: &SharedConfig,
    cached_allowlist: &Arc<RwLock<Vec<PublicKey>>>,
    reply_ctx: &SharedReplyContexts,
    account_id: &str,
    event_sink: &Arc<dyn moltis_channels::ChannelEventSink>,
) {
    // Dedup, skip self-messages, skip stale events.
    if seen.check_and_insert(&event.id) {
        return;
    }
    if event.pubkey == *bot_pubkey {
        return;
    }
    if event.created_at < since {
        return;
    }

    let Some(group_id) = crate::groups::extract_group_id(event) else {
        tracing::debug!(
            account_id,
            event_id = %event.id,
            "group message without `h` tag, ignoring"
        );
        return;
    };

    // Read group config fields (drop the guard before any .await).
    let (groups, mention_mode, ack_reactions) = {
        let cfg = config.read().await;
        (
            cfg.groups.clone(),
            cfg.group_mention_mode.clone(),
            cfg.group_ack_reactions,
        )
    };

    // Re-check membership against the configured join list. The relay is
    // untrusted and `nostr-sdk` does not verify that delivered events match our
    // subscription filters, so a `kind:9` for a group we never joined can still
    // arrive on this socket.
    if let Err(denied) = crate::groups::check_group_access(&group_id, &groups) {
        #[cfg(feature = "metrics")]
        counter!(nostr_metrics::ACCESS_CONTROL_DENIALS_TOTAL, "reason" => "group").increment(1);
        tracing::debug!(
            account_id,
            group = group_id,
            "group message rejected: {denied}"
        );
        return;
    }

    if !crate::groups::should_respond(&mention_mode, event, bot_pubkey) {
        tracing::trace!(
            account_id,
            group = group_id,
            "group message ignored by mention mode"
        );
        return;
    }

    // Size validation — truncate at a safe UTF-8 boundary.
    let text = if event.content.len() > MAX_MESSAGE_BYTES {
        tracing::warn!(
            account_id,
            len = event.content.len(),
            "group message exceeds size limit"
        );
        &event.content[..event.content.floor_char_boundary(MAX_MESSAGE_BYTES)]
    } else {
        event.content.as_str()
    };
    if text.trim().is_empty() {
        return;
    }

    let sender_hex = event.pubkey.to_hex();
    let sender_npub = event
        .pubkey
        .to_bech32()
        .unwrap_or_else(|_| sender_hex.clone());

    // Remember who wrote this and in which dialect, so the reply can `p`-tag
    // them and answer with the same kind. Outbound only receives the chat id
    // and this event id, so it cannot recover either on its own.
    {
        let mut ctxs = reply_ctx.lock().unwrap_or_else(|e| e.into_inner());
        ctxs.record(event.id, &group_id, event.pubkey, event.kind);
    }

    #[cfg(feature = "metrics")]
    counter!(nostr_metrics::MESSAGES_RECEIVED_TOTAL).increment(1);

    event_sink
        .emit(moltis_channels::ChannelEvent::InboundMessage {
            channel_type: moltis_channels::ChannelType::Nostr,
            account_id: account_id.to_string(),
            peer_id: group_id.clone(),
            username: Some(sender_npub.clone()),
            sender_name: None,
            message_count: None,
            access_granted: true,
        })
        .await;

    // Reply routes back to the group (chat_id = group id), threaded to this
    // message so NIP-29 clients render it as a reply.
    // Setting `ack_message_id` opts this message into the gateway's
    // acknowledgement reactions (👀 on receipt, ✅/❌ on completion).
    let reply_to = moltis_channels::ChannelReplyTarget {
        ack_message_id: ack_reactions.then(|| event.id.to_hex()),
        channel_type: moltis_channels::ChannelType::Nostr,
        account_id: account_id.to_string(),
        // Prefixed so the target carries its own kind: a group id can look
        // exactly like a pubkey, and this target outlives the config.
        chat_id: crate::groups::group_target(&group_id),
        message_id: Some(event.id.to_hex()),
        thread_id: None,
    };

    // Intercept slash commands before the LLM, matching DM behaviour so
    // `/help` and friends work the same way in a Buzz channel.
    if let Some(cmd_text) = text.strip_prefix('/') {
        let cmd_name = cmd_text.split_whitespace().next().unwrap_or("");
        if moltis_channels::commands::is_channel_command(cmd_name, cmd_text) {
            // `/sh` and `/sandbox` control code execution. Group membership is
            // the relay's call, not ours, so restrict them to keys the owner
            // nominated — and say so rather than ignoring the message.
            if crate::groups::is_operator_only_command(cmd_name) {
                let is_operator = {
                    let allowed = cached_allowlist.read().unwrap_or_else(|e| e.into_inner());
                    crate::groups::is_group_operator(&event.pubkey, &allowed)
                };
                if !is_operator {
                    tracing::warn!(
                        account_id,
                        group = group_id,
                        sender = sender_hex,
                        command = cmd_name,
                        "refused operator-only command from non-allowlisted group sender"
                    );
                    let refusal = format!("`/{cmd_name}` is restricted to this bot's operators.");
                    if let Err(e) =
                        crate::groups::authorized_publish(config, &group_id, async || {
                            crate::groups::send_group_message(
                                client,
                                event.kind,
                                &group_id,
                                &refusal,
                                Some(event.id),
                                Some(event.pubkey),
                            )
                            .await
                        })
                        .await
                    {
                        tracing::warn!(account_id, "command refusal not published: {e}");
                    }
                    return;
                }
            }
            let response = if cmd_name == "help" {
                Ok(moltis_channels::commands::help_text())
            } else {
                event_sink
                    .dispatch_command(cmd_text, reply_to, Some(&sender_hex))
                    .await
            };
            let reply_text = match response {
                Ok(msg) => msg,
                Err(e) => format!("Error: {e}"),
            };
            // Re-authorized rather than published outright: `dispatch_command`
            // can run for as long as the command takes (`/compact` rebuilds a
            // session), and the group may have been withdrawn in the meantime.
            // Same gate every other group publish uses.
            if let Err(e) = crate::groups::authorized_publish(config, &group_id, async || {
                crate::groups::send_group_message(
                    client,
                    event.kind,
                    &group_id,
                    &reply_text,
                    Some(event.id),
                    Some(event.pubkey),
                )
                .await
            })
            .await
            {
                tracing::warn!(account_id, "group command response not published: {e}");
            }
            return;
        }
    }

    let meta = moltis_channels::ChannelMessageMeta {
        channel_type: moltis_channels::ChannelType::Nostr,
        sender_name: None,
        username: Some(sender_npub),
        sender_id: Some(sender_hex),
        message_kind: Some(moltis_channels::ChannelMessageKind::Text),
        model: None,
        agent_id: None,
        audio_filename: None,
        documents: None,
    };

    event_sink.dispatch_to_chat(text, reply_to, meta).await;
}

/// Try to decrypt a DM (NIP-04, then NIP-44). Returns `None` on failure.
fn try_decrypt(keys: &Keys, sender: &PublicKey, content: &str) -> Option<String> {
    nip04::decrypt(keys.secret_key(), sender, content)
        .ok()
        .or_else(|| nip44::decrypt(keys.secret_key(), sender, content).ok())
}

/// Handle an OTP verification reply (sender sent a 6-digit code).
#[allow(clippy::too_many_arguments)]
async fn handle_otp_verification(
    otp: &SharedOtp,
    client: &Client,
    keys: &Keys,
    sender_pubkey: &PublicKey,
    account_id: &str,
    sender_hex: &str,
    sender_npub: &str,
    code: &str,
    event_sink: &Arc<dyn moltis_channels::ChannelEventSink>,
) {
    use moltis_channels::otp::{OtpVerifyResult, emit_otp_resolution};

    let result = {
        let mut guard = otp.lock().unwrap_or_else(|e| e.into_inner());
        guard.verify(sender_hex, code)
    };

    let (reply_text, resolution) = match result {
        OtpVerifyResult::Approved => {
            // Request the gateway to add this sender to the approved list.
            event_sink
                .request_sender_approval("nostr", account_id, sender_hex)
                .await;
            ("Access granted. You can now send messages.", "approved")
        },
        OtpVerifyResult::WrongCode { attempts_left } => (
            if attempts_left > 0 {
                "Wrong code. Please try again."
            } else {
                "Too many failed attempts. You are temporarily locked out."
            },
            "wrong_code",
        ),
        OtpVerifyResult::LockedOut => (
            "Too many failed attempts. You are temporarily locked out.",
            "locked_out",
        ),
        OtpVerifyResult::NoPending => ("No pending challenge.", "no_pending"),
        OtpVerifyResult::Expired => (
            "Challenge expired. Send another message to get a new code.",
            "expired",
        ),
    };

    // Send the reply to the sender via gift wrap.
    if let Err(e) =
        crate::gift_wrap::send_gift_wrapped_dm(client, keys, sender_pubkey, reply_text).await
    {
        tracing::warn!(account_id, "failed to send OTP verification reply: {e}");
    }

    // Emit resolution event for admin UI.
    emit_otp_resolution(
        Some(event_sink.as_ref()),
        moltis_channels::ChannelType::Nostr,
        account_id,
        sender_hex,
        Some(sender_npub),
        resolution,
    )
    .await;
}

/// Initiate an OTP challenge for a non-allowlisted sender.
///
/// Generates a 6-digit code via `OtpState::initiate()`, sends the challenge
/// prompt to the sender as an encrypted DM, and emits an `OtpChallenge` event
/// for the admin web UI.
#[allow(clippy::too_many_arguments)]
async fn handle_otp_challenge(
    client: &Client,
    keys: &Keys,
    sender_pubkey: &PublicKey,
    otp: &SharedOtp,
    account_id: &str,
    sender_hex: &str,
    sender_npub: &str,
    event_sink: &Arc<dyn moltis_channels::ChannelEventSink>,
) {
    use moltis_channels::otp::{OtpInitResult, emit_otp_challenge};

    let init_result = {
        let mut otp_guard = otp.lock().unwrap_or_else(|e| e.into_inner());
        otp_guard.initiate(sender_hex, Some(sender_npub.to_string()), None)
    };

    match init_result {
        OtpInitResult::Created(code) => {
            // Send challenge prompt to the sender via gift wrap.
            if let Err(e) = crate::gift_wrap::send_gift_wrapped_dm(
                client,
                keys,
                sender_pubkey,
                OTP_CHALLENGE_MSG,
            )
            .await
            {
                tracing::warn!(account_id, "failed to send OTP challenge DM: {e}");
            }

            // Emit OTP challenge event for the admin UI.
            let expires_at = ::time::OffsetDateTime::now_utc().unix_timestamp() + 300;

            emit_otp_challenge(
                Some(event_sink.as_ref()),
                moltis_channels::ChannelType::Nostr,
                account_id,
                sender_hex,
                Some(sender_npub),
                None,
                code,
                expires_at,
            )
            .await;

            #[cfg(feature = "metrics")]
            counter!(nostr_metrics::OTP_CHALLENGES_TOTAL).increment(1);
        },
        OtpInitResult::AlreadyPending | OtpInitResult::LockedOut => {
            // Silent — don't spam the sender.
        },
    }

    // Always emit InboundMessage with access_granted: false for the UI.
    event_sink
        .emit(moltis_channels::ChannelEvent::InboundMessage {
            channel_type: moltis_channels::ChannelType::Nostr,
            account_id: account_id.to_string(),
            peer_id: sender_hex.to_string(),
            username: Some(sender_npub.to_string()),
            sender_name: None,
            message_count: None,
            access_granted: false,
        })
        .await;
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::sync::Mutex;

    use {
        async_trait::async_trait,
        moltis_channels::{
            ChannelEvent, ChannelEventSink, ChannelMessageMeta, ChannelReplyTarget,
            Result as ChannelResult,
        },
        nostr_sdk::prelude::{EventBuilder, Keys},
    };

    use {
        super::*,
        crate::{config::NostrAccountConfig, groups, reply_ctx::ReplyContexts},
    };

    /// What a dispatched message looked like, for assertions.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Dispatched {
        text: String,
        chat_id: String,
        message_id: Option<String>,
    }

    /// Recording `ChannelEventSink` — captures dispatches instead of running
    /// the agent, so the inbound gate can be exercised without a gateway.
    #[derive(Default)]
    struct RecordingSink {
        dispatched: Mutex<Vec<Dispatched>>,
        commands: Mutex<Vec<String>>,
    }

    impl RecordingSink {
        fn dispatched(&self) -> Vec<Dispatched> {
            self.dispatched
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
        }

        /// Commands that reached the command handler. Distinguishes "refused by
        /// the operator gate" from "handled as a command" — both leave
        /// `dispatched` empty, so asserting on that alone proves nothing.
        fn commands(&self) -> Vec<String> {
            self.commands
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
        }
    }

    #[async_trait]
    impl ChannelEventSink for RecordingSink {
        async fn emit(&self, _event: ChannelEvent) {}

        async fn dispatch_to_chat(
            &self,
            text: &str,
            reply_to: ChannelReplyTarget,
            _meta: ChannelMessageMeta,
        ) {
            self.dispatched
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(Dispatched {
                    text: text.to_string(),
                    chat_id: reply_to.chat_id,
                    message_id: reply_to.message_id,
                });
        }

        async fn dispatch_command(
            &self,
            command: &str,
            _reply_to: ChannelReplyTarget,
            _sender_id: Option<&str>,
        ) -> ChannelResult<String> {
            self.commands
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(command.to_string());
            Ok(String::new())
        }

        async fn request_disable_account(
            &self,
            _channel_type: &str,
            _account_id: &str,
            _reason: &str,
        ) {
        }
    }

    /// Fixtures for driving `handle_group_event` without any network.
    struct Harness {
        client: Client,
        bot: Keys,
        seen: SeenTracker,
        config: SharedConfig,
        allowlist: Arc<RwLock<Vec<PublicKey>>>,
        reply_ctx: SharedReplyContexts,
        sink: Arc<RecordingSink>,
        since: Timestamp,
    }

    impl Harness {
        fn new(config: NostrAccountConfig) -> Self {
            let bot = Keys::generate();
            Self {
                client: Client::new(bot.clone()),
                bot,
                seen: SeenTracker::new(),
                config: Arc::new(tokio::sync::RwLock::new(config)),
                allowlist: Arc::new(RwLock::new(Vec::new())),
                reply_ctx: Arc::new(Mutex::new(ReplyContexts::new())),
                sink: Arc::new(RecordingSink::default()),
                since: Timestamp::from(0),
            }
        }

        fn with_groups(groups: Vec<&str>, mention_mode: MentionModeAlias) -> Self {
            Self::new(NostrAccountConfig {
                groups: groups.into_iter().map(String::from).collect(),
                group_mention_mode: mention_mode,
                ..Default::default()
            })
        }

        /// Build a signed group message from a third party.
        fn incoming(&self, group: &str, text: &str, kind: Kind, mention_bot: bool) -> Event {
            let author = Keys::generate();
            let mention = mention_bot.then(|| self.bot.public_key());
            EventBuilder::new(kind, text)
                .tags(groups::build_group_message_tags(group, None, mention))
                .sign_with_keys(&author)
                .expect("sign group event")
        }

        async fn handle(&mut self, event: &Event) {
            let sink: Arc<dyn ChannelEventSink> =
                Arc::clone(&self.sink) as Arc<dyn ChannelEventSink>;
            handle_group_event(
                event,
                &self.client,
                &self.bot.public_key(),
                self.since,
                &mut self.seen,
                &self.config,
                &self.allowlist,
                &self.reply_ctx,
                "acct",
                &sink,
            )
            .await;
        }
    }

    type MentionModeAlias = moltis_channels::gating::MentionMode;

    #[test]
    fn max_message_size_is_reasonable() {
        assert_eq!(MAX_MESSAGE_BYTES, 64 * 1024);
    }

    #[tokio::test]
    async fn dispatches_mentioned_message_from_joined_group() {
        let mut h = Harness::with_groups(vec!["buzz-general"], MentionModeAlias::Mention);
        let event = h.incoming("buzz-general", "hello bot", groups::group_chat_kind(), true);
        h.handle(&event).await;

        let got = h.sink.dispatched();
        assert_eq!(got.len(), 1, "mentioned message should reach the agent");
        assert_eq!(got[0].text, "hello bot");
        // The reply target is the prefixed group id, so outbound can classify
        // it without consulting config — a group id may look like a pubkey.
        assert_eq!(got[0].chat_id, "group:buzz-general");
        assert_eq!(
            groups::parse_group_target(&got[0].chat_id),
            Some("buzz-general")
        );
        assert_eq!(got[0].message_id, Some(event.id.to_hex()));
    }

    /// Buzz posts kind:40002; it must be accepted just like kind:9.
    #[tokio::test]
    async fn accepts_buzz_v2_message_kind() {
        let mut h = Harness::with_groups(vec!["buzz-general"], MentionModeAlias::Always);
        let event = h.incoming(
            "buzz-general",
            "from buzz",
            groups::buzz_stream_message_kind(),
            false,
        );
        h.handle(&event).await;
        assert_eq!(h.sink.dispatched().len(), 1);
    }

    /// The core security property: a relay can push a kind:9/40002 for a group
    /// we never joined, and it must never reach the agent.
    #[tokio::test]
    async fn drops_message_from_unjoined_group() {
        let mut h = Harness::with_groups(vec!["buzz-general"], MentionModeAlias::Always);
        let event = h.incoming(
            "attacker-group",
            "ignore me",
            groups::group_chat_kind(),
            true,
        );
        h.handle(&event).await;
        assert!(
            h.sink.dispatched().is_empty(),
            "unjoined group must not reach the agent"
        );
    }

    #[tokio::test]
    async fn drops_group_message_when_no_groups_configured() {
        let mut h = Harness::with_groups(vec![], MentionModeAlias::Always);
        let event = h.incoming("anything", "hi", groups::group_chat_kind(), true);
        h.handle(&event).await;
        assert!(h.sink.dispatched().is_empty());
    }

    #[tokio::test]
    async fn mention_mode_filters_unmentioned_messages() {
        let mut h = Harness::with_groups(vec!["grp"], MentionModeAlias::Mention);
        let event = h.incoming(
            "grp",
            "chatter between humans",
            groups::group_chat_kind(),
            false,
        );
        h.handle(&event).await;
        assert!(h.sink.dispatched().is_empty());
    }

    #[tokio::test]
    async fn mention_mode_none_never_responds() {
        let mut h = Harness::with_groups(vec!["grp"], MentionModeAlias::None);
        let event = h.incoming("grp", "hey bot", groups::group_chat_kind(), true);
        h.handle(&event).await;
        assert!(h.sink.dispatched().is_empty());
    }

    #[tokio::test]
    async fn ignores_message_without_h_tag() {
        let mut h = Harness::with_groups(vec!["grp"], MentionModeAlias::Always);
        let author = Keys::generate();
        let event = EventBuilder::new(groups::group_chat_kind(), "no group")
            .sign_with_keys(&author)
            .expect("sign");
        h.handle(&event).await;
        assert!(h.sink.dispatched().is_empty());
    }

    #[tokio::test]
    async fn deduplicates_replayed_events() {
        let mut h = Harness::with_groups(vec!["grp"], MentionModeAlias::Always);
        let event = h.incoming("grp", "once", groups::group_chat_kind(), false);
        h.handle(&event).await;
        h.handle(&event).await;
        assert_eq!(h.sink.dispatched().len(), 1, "replay must be deduped");
    }

    #[tokio::test]
    async fn ignores_own_messages() {
        let mut h = Harness::with_groups(vec!["grp"], MentionModeAlias::Always);
        let own = EventBuilder::new(groups::group_chat_kind(), "my own reply")
            .tags(groups::build_group_message_tags("grp", None, None))
            .sign_with_keys(&h.bot)
            .expect("sign");
        h.handle(&own).await;
        assert!(
            h.sink.dispatched().is_empty(),
            "bot must not answer itself and loop"
        );
    }

    #[tokio::test]
    async fn skips_blank_messages() {
        let mut h = Harness::with_groups(vec!["grp"], MentionModeAlias::Always);
        let event = h.incoming("grp", "   ", groups::group_chat_kind(), false);
        h.handle(&event).await;
        assert!(h.sink.dispatched().is_empty());
    }

    /// A channel command is answered by the command handler, not the model —
    /// the same interception DMs get. Publishing the response needs a relay,
    /// which this harness has none of, so the assertion is that the text never
    /// reached the agent.
    #[tokio::test]
    async fn slash_commands_are_intercepted_in_groups() {
        let mut h = Harness::with_groups(vec!["grp"], MentionModeAlias::Always);
        let event = h.incoming("grp", "/help", groups::group_chat_kind(), false);
        h.handle(&event).await;

        assert!(
            h.sink.dispatched().is_empty(),
            "/help must be handled as a command, not sent to the model"
        );
    }

    /// A slash that is not a known command is ordinary text and must still
    /// reach the agent, rather than being silently swallowed.
    #[tokio::test]
    async fn unknown_slash_text_still_reaches_the_agent() {
        let mut h = Harness::with_groups(vec!["grp"], MentionModeAlias::Always);
        let event = h.incoming(
            "grp",
            "/notacommand hello",
            groups::group_chat_kind(),
            false,
        );
        h.handle(&event).await;

        let got = h.sink.dispatched();
        assert_eq!(got.len(), 1, "unknown slash text is just a message");
        assert_eq!(got[0].text, "/notacommand hello");
        assert!(h.sink.commands().is_empty());
    }

    /// A non-execution command from an ordinary group member is not gated —
    /// the operator list only guards `/sh` and `/sandbox`.
    #[tokio::test]
    async fn ordinary_commands_are_open_to_group_members() {
        let mut h = Harness::with_groups(vec!["grp"], MentionModeAlias::Always);
        let event = h.incoming("grp", "/new", groups::group_chat_kind(), false);
        h.handle(&event).await;

        assert_eq!(h.sink.commands(), vec!["new".to_string()]);
        assert!(h.sink.dispatched().is_empty());
    }

    /// `/sh` grants shell execution, and group membership is the relay's call,
    /// so a member who is not on the account's allowlist must not reach it.
    #[tokio::test]
    async fn operator_only_commands_are_refused_from_group_strangers() {
        let mut h = Harness::with_groups(vec!["grp"], MentionModeAlias::Always);
        let event = h.incoming("grp", "/sh on", groups::group_chat_kind(), false);
        h.handle(&event).await;

        assert!(
            h.sink.dispatched().is_empty(),
            "/sh from a non-operator must not reach the model"
        );
        assert!(
            h.sink.commands().is_empty(),
            "/sh from a non-operator must not reach the command handler either"
        );
    }

    /// The same command from an allowlisted key does reach the command handler
    /// — the gate is about who sent it, not about the command being ignored.
    #[tokio::test]
    async fn operator_only_commands_are_allowed_from_operators() {
        let author = Keys::generate();
        let mut h = Harness::with_groups(vec!["grp"], MentionModeAlias::Always);
        {
            let mut allowed = h.allowlist.write().unwrap_or_else(|e| e.into_inner());
            allowed.push(author.public_key());
        }
        let event = EventBuilder::new(groups::group_chat_kind(), "/sh on")
            .tags(groups::build_group_message_tags("grp", None, None))
            .sign_with_keys(&author)
            .expect("sign");
        h.handle(&event).await;

        assert_eq!(
            h.sink.commands(),
            vec!["sh on".to_string()],
            "an operator's /sh must be handled as a command"
        );
        assert!(
            h.sink.dispatched().is_empty(),
            "commands are handled by the command path, not dispatched to the model"
        );
    }

    /// A command reply is published after `dispatch_command` returns, and that
    /// can take as long as the command does — long enough for the operator to
    /// withdraw the bot. The reply goes through the same authorization gate as
    /// every other group publish, so a revoked group refuses it rather than
    /// answering into a channel the bot has been removed from.
    #[tokio::test]
    async fn command_replies_are_refused_once_the_group_is_revoked() {
        let mut h = Harness::with_groups(vec!["grp"], MentionModeAlias::Always);
        let event = h.incoming("grp", "/new", groups::group_chat_kind(), false);

        // The command still reaches the handler...
        h.handle(&event).await;
        assert_eq!(h.sink.commands(), vec!["new".to_string()]);

        // ...and publishing the answer is gated on the live config, which the
        // harness has no relay for — so assert the gate itself refuses.
        h.config.write().await.groups.clear();
        let cfg = h.config.read().await;
        assert!(
            groups::check_group_send(&cfg.groups, &cfg.group_mention_mode, "grp").is_err(),
            "a revoked group must refuse the command reply"
        );
    }

    /// Accepted messages record who to `p`-tag and which dialect to answer in.
    #[tokio::test]
    async fn records_reply_context_for_accepted_message() {
        let mut h = Harness::with_groups(vec!["grp"], MentionModeAlias::Always);
        let event = h.incoming("grp", "hi", groups::buzz_stream_message_kind(), false);
        h.handle(&event).await;

        let ctxs = h.reply_ctx.lock().unwrap_or_else(|e| e.into_inner());
        let ctx = ctxs.get(&event.id).expect("context recorded");
        assert_eq!(ctx.author, event.pubkey);
        assert_eq!(ctx.kind, groups::buzz_stream_message_kind());
        assert_eq!(
            ctxs.kind_for_group("grp"),
            Some(groups::buzz_stream_message_kind())
        );
    }
}
