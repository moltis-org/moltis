//! Nostr relay subscription loop — inbound DM pipeline.
//!
//! Subscribes to kind:4 (NIP-04 encrypted DMs) addressed to the bot's pubkey.
//! Events flow through dedup, self-message filtering, access control, and
//! decryption before being dispatched to the gateway via `ChannelEventSink`.

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
    config::NostrAccountConfig,
    seen::SeenTracker,
    state::SharedOtp,
};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, nostr as nostr_metrics};

/// Maximum plaintext message size in bytes.
const MAX_MESSAGE_BYTES: usize = 64 * 1024;

/// Message sent to non-allowlisted senders as an OTP challenge prompt.
const OTP_CHALLENGE_MSG: &str = "You are not on the allowlist. A PIN challenge has been sent to the admin. Reply with the 6-digit code to gain access.";

/// Run the relay subscription loop for a single Nostr account.
///
/// Subscribes to NIP-04 DMs (kind:4) targeted at `bot_pubkey` and dispatches
/// inbound messages to the gateway. Runs until `cancel` is triggered or the
/// relay pool shuts down (which triggers an auto-disable request).
pub async fn run_subscription_loop(
    client: Client,
    keys: Keys,
    config: Arc<RwLock<NostrAccountConfig>>,
    cached_allowlist: Arc<RwLock<Vec<PublicKey>>>,
    otp: SharedOtp,
    account_id: String,
    event_sink: Arc<dyn moltis_channels::ChannelEventSink>,
    cancel: CancellationToken,
) {
    let bot_pubkey = keys.public_key();
    let now_secs =
        u64::try_from(::time::OffsetDateTime::now_utc().unix_timestamp()).unwrap_or_default();
    let since = Timestamp::from(now_secs);

    let filter = Filter::new()
        .kind(Kind::EncryptedDirectMessage)
        .pubkey(bot_pubkey)
        .since(since);

    let npub = bot_pubkey
        .to_bech32()
        .unwrap_or_else(|_| bot_pubkey.to_hex());
    tracing::info!(
        account_id,
        pubkey = %npub,
        "starting Nostr DM subscription"
    );

    let mut seen = SeenTracker::new();

    // Subscribe (single filter)
    if let Err(e) = client.subscribe(filter, None).await {
        tracing::error!(account_id, "failed to subscribe: {e}");
        return;
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
    config: &Arc<RwLock<NostrAccountConfig>>,
    cached_allowlist: &Arc<RwLock<Vec<PublicKey>>>,
    otp: &SharedOtp,
    account_id: &str,
    event_sink: &Arc<dyn moltis_channels::ChannelEventSink>,
) {
    // 1. Skip non-DM events
    if event.kind != Kind::EncryptedDirectMessage {
        return;
    }

    // 2. Dedup check
    if seen.check_and_insert(&event.id) {
        return;
    }

    // 3. Skip self-messages
    if event.pubkey == *bot_pubkey {
        return;
    }

    // 4. Skip stale events
    if event.created_at < since {
        return;
    }

    let sender_hex = event.pubkey.to_hex();
    let sender_npub = event
        .pubkey
        .to_bech32()
        .unwrap_or_else(|_| sender_hex.clone());

    // 5. Read config fields (drop guard before any .await).
    let (dm_policy, otp_self_approval) = {
        let cfg = config.read().unwrap_or_else(|e| e.into_inner());
        (cfg.dm_policy.clone(), cfg.otp_self_approval)
    };

    // 5a. OTP verification — if this sender has a pending challenge, decrypt
    //     first to check if the message is a 6-digit code, and verify it.
    //     This must run BEFORE the access-control gate because the sender is
    //     not yet on the allowlist when they reply with the OTP code.
    let has_pending = {
        let guard = otp.lock().unwrap_or_else(|e| e.into_inner());
        guard.has_pending(&sender_hex)
    };
    if has_pending {
        if let Some(plaintext) = try_decrypt(keys, &event.pubkey, &event.content) {
            let trimmed = plaintext.trim();
            if trimmed.len() == 6 && trimmed.chars().all(|c| c.is_ascii_digit()) {
                handle_otp_verification(
                    otp,
                    client,
                    keys,
                    &event.pubkey,
                    account_id,
                    &sender_hex,
                    &sender_npub,
                    trimmed,
                    event_sink,
                )
                .await;
                return;
            }
        }
        // Non-code reply while challenge pending — silently ignore.
        return;
    }

    // 5b. Normal access-control gate (scope the guard so it drops before any .await).
    let access_result = {
        let allowed = cached_allowlist.read().unwrap_or_else(|e| e.into_inner());
        access::check_dm_access(&event.pubkey, &dm_policy, &allowed)
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
                    &event.pubkey,
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

    // 6. Decrypt content — try NIP-04 first, fall back to NIP-44
    let plaintext = match nip04::decrypt(keys.secret_key(), &event.pubkey, &event.content) {
        Ok(text) => text,
        Err(_nip04_err) => match nip44::decrypt(keys.secret_key(), &event.pubkey, &event.content) {
            Ok(text) => text,
            Err(nip44_err) => {
                #[cfg(feature = "metrics")]
                counter!(nostr_metrics::DECRYPT_ERRORS_TOTAL).increment(1);
                tracing::warn!(
                    account_id,
                    event_id = %event.id,
                    "decrypt failed (NIP-04 and NIP-44): {nip44_err}"
                );
                return;
            },
        },
    };

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

    // 9. Dispatch to gateway
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

    let reply_to = moltis_channels::ChannelReplyTarget {
        channel_type: moltis_channels::ChannelType::Nostr,
        account_id: account_id.to_string(),
        chat_id: sender_hex,
        message_id: Some(event.id.to_hex()),
        thread_id: None,
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

    // Send the reply to the sender.
    if let Ok(encrypted) = nip04::encrypt(keys.secret_key(), sender_pubkey, reply_text) {
        let tag = nostr_sdk::prelude::Tag::public_key(*sender_pubkey);
        let builder =
            nostr_sdk::prelude::EventBuilder::new(Kind::EncryptedDirectMessage, &encrypted)
                .tag(tag);
        if let Err(e) = client.send_event_builder(builder).await {
            tracing::warn!(account_id, "failed to send OTP verification reply: {e}");
        }
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
            // Send challenge prompt to the sender via encrypted DM.
            if let Ok(encrypted) =
                nip04::encrypt(keys.secret_key(), sender_pubkey, OTP_CHALLENGE_MSG)
            {
                let tag = nostr_sdk::prelude::Tag::public_key(*sender_pubkey);
                let builder =
                    nostr_sdk::prelude::EventBuilder::new(Kind::EncryptedDirectMessage, &encrypted)
                        .tag(tag);
                if let Err(e) = client.send_event_builder(builder).await {
                    tracing::warn!(account_id, "failed to send OTP challenge DM: {e}");
                }
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
mod tests {
    use super::*;

    #[test]
    fn max_message_size_is_reasonable() {
        assert_eq!(MAX_MESSAGE_BYTES, 64 * 1024);
    }
}
