use std::sync::Arc;

use {
    tracing::{debug, info, warn},
    wacore::types::{events::Event, message::MessageInfo},
    wacore_binary::jid::{Jid, JidExt as _},
    waproto::whatsapp as wa,
    whatsapp_rust::client::Client,
};

use moltis_channels::{
    ChannelAttachment, ChannelEvent, ChannelMessageKind, ChannelMessageMeta, ChannelReplyTarget,
    ChannelType, message_log::MessageLogEntry,
};

use crate::{
    access::{self, AccessDenied},
    otp::{OTP_CHALLENGE_MSG, OtpInitResult, OtpVerifyResult},
    state::{AccountState, AccountStateMap, has_bot_watermark},
};

/// Process an incoming whatsapp-rust event for the given account.
pub async fn handle_event(
    event: Event,
    client: Arc<Client>,
    state: Arc<AccountState>,
    accounts: AccountStateMap,
) {
    match event {
        Event::PairingQrCode { code, .. } => {
            info!(account_id = %state.account_id, "QR code generated for pairing");

            // Store latest QR data so the REST endpoint can serve it.
            if let Ok(mut qr) = state.latest_qr.write() {
                *qr = Some(code.clone());
            }

            if let Some(ref sink) = state.event_sink {
                sink.emit(ChannelEvent::PairingQrCode {
                    channel_type: ChannelType::Whatsapp,
                    account_id: state.account_id.clone(),
                    qr_data: code,
                })
                .await;
            }
        },
        Event::Connected(_) => {
            info!(account_id = %state.account_id, "WhatsApp connected");
            state
                .connected
                .store(true, std::sync::atomic::Ordering::Relaxed);

            // Clear QR data once connected.
            if let Ok(mut qr) = state.latest_qr.write() {
                *qr = None;
            }

            let display_name = state.client.get_push_name().await;
            let display = if display_name.is_empty() {
                None
            } else {
                Some(display_name)
            };

            if let Some(ref sink) = state.event_sink {
                sink.emit(ChannelEvent::PairingComplete {
                    channel_type: ChannelType::Whatsapp,
                    account_id: state.account_id.clone(),
                    display_name: display,
                })
                .await;
            }
        },
        Event::PairError(err) => {
            warn!(account_id = %state.account_id, error = ?err, "WhatsApp pairing failed");
            if let Some(ref sink) = state.event_sink {
                sink.emit(ChannelEvent::PairingFailed {
                    channel_type: ChannelType::Whatsapp,
                    account_id: state.account_id.clone(),
                    reason: format!("{err:?}"),
                })
                .await;
            }
        },
        Event::Disconnected(_) => {
            info!(account_id = %state.account_id, "WhatsApp disconnected");
            state
                .connected
                .store(false, std::sync::atomic::Ordering::Relaxed);
        },
        Event::LoggedOut(_) => {
            warn!(account_id = %state.account_id, "WhatsApp logged out");
            state
                .connected
                .store(false, std::sync::atomic::Ordering::Relaxed);
            if let Some(ref sink) = state.event_sink {
                sink.emit(ChannelEvent::AccountDisabled {
                    channel_type: ChannelType::Whatsapp,
                    account_id: state.account_id.clone(),
                    reason: "logged out by WhatsApp".into(),
                })
                .await;
            }
        },
        Event::Message(msg, msg_info) => {
            handle_message(msg, msg_info, &client, &state, &accounts).await;
        },
        _ => {
            debug!(account_id = %state.account_id, event = ?std::mem::discriminant(&event), "unhandled WhatsApp event");
        },
    }
}

async fn handle_message(
    msg: Box<wa::Message>,
    info: MessageInfo,
    client: &Client,
    state: &AccountState,
    accounts: &AccountStateMap,
) {
    let sender_jid: &Jid = &info.source.sender;
    let chat_jid: &Jid = &info.source.chat;

    let peer_id = sender_jid.to_string();
    let chat_id = chat_jid.to_string();
    let username = sender_jid.user.clone();
    let sender_name = if info.push_name.is_empty() {
        None
    } else {
        Some(info.push_name.clone())
    };

    // Self-chat detection:
    // - Primary path: `is_from_me` (message sent from another device on our own account).
    // - Fallback path: sender/chat JIDs match the linked account owner.
    //
    // Some "Message Yourself" deliveries can arrive without `is_from_me = true`
    // but still use owner JIDs. Those should still bypass access control.
    //
    // We still prevent loops by dropping known bot echoes (recent sent IDs
    // and watermark).
    let mut is_owner_self_chat = false;
    let own_pn = state.client.get_pn().await;
    let own_lid = state.client.get_lid().await;
    let is_self_chat = is_owner_user(chat_jid, own_pn.as_ref(), own_lid.as_ref());
    let sender_is_owner = is_owner_user(sender_jid, own_pn.as_ref(), own_lid.as_ref());

    if info.source.is_from_me || (is_self_chat && sender_is_owner) {
        // Check text for bot watermark as secondary loop detection.
        let raw_text = msg
            .conversation
            .as_deref()
            .or_else(|| {
                msg.extended_text_message
                    .as_ref()
                    .and_then(|m| m.text.as_deref())
            })
            .unwrap_or("");

        if !is_self_chat || state.was_sent_by_us(&info.id) || has_bot_watermark(raw_text) {
            debug!(
                account_id = %state.account_id,
                is_self_chat,
                sender_is_owner,
                is_from_me = info.source.is_from_me,
                "ignoring self-sent message"
            );
            return;
        }
        debug!(
            account_id = %state.account_id,
            sender_is_owner,
            is_from_me = info.source.is_from_me,
            "processing self-chat message from another device"
        );
        is_owner_self_chat = true;
    }

    // Extract text from the message.
    let text = msg
        .conversation
        .as_deref()
        .or_else(|| {
            msg.extended_text_message
                .as_ref()
                .and_then(|m| m.text.as_deref())
        })
        .unwrap_or("");

    let message_kind = classify_message(&msg, text);

    // Access control. Self-chat messages from the account owner always bypass
    // access control — the owner is inherently authorized.
    let is_group = info.source.is_group;
    let group_id = if is_group {
        Some(chat_id.as_str())
    } else {
        None
    };
    let access_result = if is_owner_self_chat {
        Ok(())
    } else {
        access::check_access(&state.config, is_group, &peer_id, Some(&username), group_id)
    };
    let access_granted = access_result.is_ok();

    // Log the message (with access_granted reflecting the actual check).
    if let Some(ref log) = state.message_log {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let entry = MessageLogEntry {
            id: 0,
            account_id: state.account_id.clone(),
            channel_type: ChannelType::Whatsapp.to_string(),
            peer_id: peer_id.clone(),
            username: Some(username.clone()),
            sender_name: sender_name.clone(),
            chat_id: chat_id.clone(),
            chat_type: if is_group {
                "group"
            } else {
                "private"
            }
            .into(),
            body: text.to_string(),
            access_granted,
            created_at: now,
        };
        if let Err(e) = log.log(entry).await {
            warn!(account_id = %state.account_id, "failed to log message: {e}");
        }
    }

    // Emit inbound message event.
    if let Some(ref sink) = state.event_sink {
        sink.emit(ChannelEvent::InboundMessage {
            channel_type: ChannelType::Whatsapp,
            account_id: state.account_id.clone(),
            peer_id: peer_id.clone(),
            username: Some(username.clone()),
            sender_name: sender_name.clone(),
            message_count: None,
            access_granted,
        })
        .await;
    }

    // Handle access denial.
    if let Err(reason) = access_result {
        warn!(
            account_id = %state.account_id,
            %reason,
            peer_id,
            username,
            "access denied"
        );

        // OTP self-approval for non-allowlisted DM users.
        if reason == AccessDenied::NotOnAllowlist && !is_group && state.config.otp_self_approval {
            handle_otp_flow(
                accounts,
                &state.account_id,
                &peer_id,
                Some(&username),
                sender_name.as_deref(),
                text,
                chat_jid,
                state,
            )
            .await;
        }
        return;
    }

    // Check for slash commands.
    if let Some(cmd) = text.strip_prefix('/') {
        let reply_to = ChannelReplyTarget {
            channel_type: ChannelType::Whatsapp,
            account_id: state.account_id.clone(),
            chat_id: chat_id.clone(),
            message_id: Some(info.id.to_string()),
        };
        if let Some(ref sink) = state.event_sink {
            match sink.dispatch_command(cmd, reply_to).await {
                Ok(response) => {
                    let outbound_msg = wa::Message {
                        conversation: Some(response),
                        ..Default::default()
                    };
                    if let Err(e) = state.send_message(chat_jid.clone(), outbound_msg).await {
                        warn!(error = %e, "failed to send command response");
                    }
                },
                Err(e) => {
                    let error_msg = wa::Message {
                        conversation: Some(format!("Error: {e}")),
                        ..Default::default()
                    };
                    let _ = state.send_message(chat_jid.clone(), error_msg).await;
                },
            }
        }
        return;
    }

    let account_id = &state.account_id;
    let reply_to = ChannelReplyTarget {
        channel_type: ChannelType::Whatsapp,
        account_id: state.account_id.clone(),
        chat_id: chat_id.clone(),
        message_id: Some(info.id.to_string()),
    };
    let meta = ChannelMessageMeta {
        channel_type: ChannelType::Whatsapp,
        sender_name,
        username: Some(username),
        message_kind: Some(message_kind),
        model: state.config.model.clone(),
        audio_filename: None,
    };

    // Dispatch based on message kind.
    match message_kind {
        ChannelMessageKind::Text => {
            if let Some(ref sink) = state.event_sink {
                sink.dispatch_to_chat(text, reply_to, meta).await;
            }
        },
        ChannelMessageKind::Photo => {
            handle_photo(&msg, client, account_id, reply_to, meta, chat_jid, state).await;
        },
        ChannelMessageKind::Voice | ChannelMessageKind::Audio => {
            handle_voice_audio(
                &msg,
                client,
                account_id,
                message_kind,
                reply_to,
                meta,
                chat_jid,
                state,
            )
            .await;
        },
        ChannelMessageKind::Video => {
            handle_video(&msg, client, account_id, reply_to, meta, chat_jid, state).await;
        },
        ChannelMessageKind::Document => {
            handle_document(&msg, client, account_id, reply_to, meta, chat_jid, state).await;
        },
        ChannelMessageKind::Location => {
            handle_location(&msg, account_id, reply_to, meta, chat_jid, state).await;
        },
        ChannelMessageKind::Other => {
            let reply_msg = wa::Message {
                conversation: Some(
                    "Sorry, I can't understand that message type. Check logs for details.".into(),
                ),
                ..Default::default()
            };
            let _ = state.send_message(chat_jid.clone(), reply_msg).await;
        },
    }
}

// ============================================================================
// Media handlers
// ============================================================================

/// Handle an inbound photo/image message: download, optimize, dispatch with attachment.
#[allow(clippy::too_many_arguments)]
async fn handle_photo(
    msg: &wa::Message,
    client: &Client,
    account_id: &str,
    reply_to: ChannelReplyTarget,
    meta: ChannelMessageMeta,
    _chat_jid: &Jid,
    state: &AccountState,
) {
    let Some(ref img) = msg.image_message else {
        return;
    };
    let caption = img.caption.as_deref().unwrap_or("").to_string();
    let mime = img.mimetype.as_deref().unwrap_or("image/jpeg").to_string();

    match client.download(img.as_ref()).await {
        Ok(image_data) => {
            debug!(account_id, size = image_data.len(), %mime, "downloaded WhatsApp image");

            let (final_data, media_type) = match moltis_media::image_ops::optimize_for_llm(
                &image_data,
                None,
            ) {
                Ok(optimized) => {
                    if optimized.was_resized {
                        info!(
                            account_id,
                            original_size = image_data.len(),
                            final_size = optimized.data.len(),
                            original_dims = %format!("{}x{}", optimized.original_width, optimized.original_height),
                            final_dims = %format!("{}x{}", optimized.final_width, optimized.final_height),
                            "resized image for LLM"
                        );
                    }
                    (optimized.data, optimized.media_type)
                },
                Err(e) => {
                    warn!(account_id, error = %e, "failed to optimize image, using original");
                    (image_data, mime)
                },
            };

            let attachment = ChannelAttachment {
                media_type,
                data: final_data,
            };
            if let Some(ref sink) = state.event_sink {
                sink.dispatch_to_chat_with_attachments(&caption, vec![attachment], reply_to, meta)
                    .await;
            }
        },
        Err(e) => {
            warn!(account_id, error = %e, "failed to download WhatsApp image");
            let fallback = if caption.is_empty() {
                "[Photo - download failed]".to_string()
            } else {
                caption
            };
            if let Some(ref sink) = state.event_sink {
                sink.dispatch_to_chat(&fallback, reply_to, meta).await;
            }
        },
    }
}

/// Handle an inbound voice/audio message: download, transcribe (STT), dispatch as text.
#[allow(clippy::too_many_arguments)]
async fn handle_voice_audio(
    msg: &wa::Message,
    client: &Client,
    account_id: &str,
    kind: ChannelMessageKind,
    reply_to: ChannelReplyTarget,
    meta: ChannelMessageMeta,
    chat_jid: &Jid,
    state: &AccountState,
) {
    let Some(ref audio) = msg.audio_message else {
        return;
    };

    // Determine format from MIME type.
    let format = audio
        .mimetype
        .as_deref()
        .map(|m| match m {
            "audio/ogg" | "audio/ogg; codecs=opus" => "ogg",
            "audio/mpeg" | "audio/mp3" => "mp3",
            "audio/mp4" | "audio/m4a" | "audio/x-m4a" | "audio/aac" => "m4a",
            "audio/wav" | "audio/x-wav" => "wav",
            "audio/webm" => "webm",
            _ => "ogg", // WhatsApp voice messages default to OGG Opus
        })
        .unwrap_or("ogg")
        .to_string();

    let kind_label = if matches!(kind, ChannelMessageKind::Voice) {
        "voice"
    } else {
        "audio"
    };

    // Check STT availability.
    let stt_available = if let Some(ref sink) = state.event_sink {
        sink.voice_stt_available().await
    } else {
        false
    };

    if !stt_available {
        // No STT — send a guidance message.
        let reply_msg = wa::Message {
            conversation: Some(format!(
                "I received your {kind_label} message but voice transcription is not available. Please send a text message instead."
            )),
            ..Default::default()
        };
        let _ = state.send_message(chat_jid.clone(), reply_msg).await;
        return;
    }

    match client.download(audio.as_ref()).await {
        Ok(audio_data) => {
            debug!(account_id, size = audio_data.len(), %format, kind_label, "downloaded WhatsApp audio");

            if let Some(ref sink) = state.event_sink {
                match sink.transcribe_voice(&audio_data, &format).await {
                    Ok(transcribed) => {
                        sink.dispatch_to_chat(&transcribed, reply_to, meta).await;
                    },
                    Err(e) => {
                        warn!(account_id, error = %e, "voice transcription failed");
                        let fallback = format!(
                            "[{} message - transcription failed]",
                            capitalize(kind_label)
                        );
                        sink.dispatch_to_chat(&fallback, reply_to, meta).await;
                    },
                }
            }
        },
        Err(e) => {
            warn!(account_id, error = %e, "failed to download WhatsApp audio");
            let reply_msg = wa::Message {
                conversation: Some(format!(
                    "I received your {kind_label} message but couldn't download the audio. Please try again."
                )),
                ..Default::default()
            };
            let _ = state.send_message(chat_jid.clone(), reply_msg).await;
        },
    }
}

/// Handle an inbound video message: download and dispatch with caption.
#[allow(clippy::too_many_arguments)]
async fn handle_video(
    msg: &wa::Message,
    _client: &Client,
    _account_id: &str,
    reply_to: ChannelReplyTarget,
    meta: ChannelMessageMeta,
    _chat_jid: &Jid,
    state: &AccountState,
) {
    let Some(ref video) = msg.video_message else {
        return;
    };
    let caption = video.caption.as_deref().unwrap_or("").to_string();

    // Try to extract a thumbnail if available (jpeg_thumbnail field).
    // Video files can be large; for now dispatch the thumbnail as an image
    // attachment so the LLM can at least see the preview.
    if let Some(ref thumb) = video.jpeg_thumbnail
        && !thumb.is_empty()
    {
        let attachment = ChannelAttachment {
            media_type: "image/jpeg".to_string(),
            data: thumb.clone(),
        };
        let text = if caption.is_empty() {
            "[Video message - thumbnail shown]".to_string()
        } else {
            format!("{caption}\n[Video message - thumbnail shown]")
        };
        if let Some(ref sink) = state.event_sink {
            sink.dispatch_to_chat_with_attachments(&text, vec![attachment], reply_to, meta)
                .await;
        }
        return;
    }

    // No thumbnail available — send caption or placeholder text.
    let text = if caption.is_empty() {
        "[Video message received - video playback not supported]".to_string()
    } else {
        format!("{caption}\n[Video message - playback not supported]")
    };
    if let Some(ref sink) = state.event_sink {
        sink.dispatch_to_chat(&text, reply_to, meta).await;
    }
}

/// Handle an inbound document message: dispatch with caption.
#[allow(clippy::too_many_arguments)]
async fn handle_document(
    msg: &wa::Message,
    _client: &Client,
    account_id: &str,
    reply_to: ChannelReplyTarget,
    meta: ChannelMessageMeta,
    _chat_jid: &Jid,
    state: &AccountState,
) {
    let Some(ref doc) = msg.document_message else {
        return;
    };
    let caption = doc.caption.as_deref().unwrap_or("").to_string();
    let filename = doc.file_name.as_deref().unwrap_or("unknown");
    let mime = doc
        .mimetype
        .as_deref()
        .unwrap_or("application/octet-stream");

    info!(account_id, filename, mime, "received document message");

    let text = if caption.is_empty() {
        format!("[Document received: {filename} ({mime})]")
    } else {
        format!("{caption}\n[Document: {filename} ({mime})]")
    };
    if let Some(ref sink) = state.event_sink {
        sink.dispatch_to_chat(&text, reply_to, meta).await;
    }
}

/// Handle an inbound location or live location message.
async fn handle_location(
    msg: &wa::Message,
    account_id: &str,
    reply_to: ChannelReplyTarget,
    meta: ChannelMessageMeta,
    chat_jid: &Jid,
    state: &AccountState,
) {
    // Check for static location first, then live location.
    let (lat, lon, is_live) = if let Some(ref loc) = msg.location_message {
        let lat = loc.degrees_latitude.unwrap_or(0.0);
        let lon = loc.degrees_longitude.unwrap_or(0.0);
        let is_live = loc.is_live.unwrap_or(false);
        (lat, lon, is_live)
    } else if let Some(ref loc) = msg.live_location_message {
        let lat = loc.degrees_latitude.unwrap_or(0.0);
        let lon = loc.degrees_longitude.unwrap_or(0.0);
        (lat, lon, true)
    } else {
        return;
    };

    // Try to resolve a pending tool-triggered location request.
    let resolved = if let Some(ref sink) = state.event_sink {
        sink.update_location(&reply_to, lat, lon).await
    } else {
        false
    };

    if resolved {
        let confirmation = wa::Message {
            conversation: Some("Location updated.".into()),
            ..Default::default()
        };
        if let Err(e) = state.send_message(chat_jid.clone(), confirmation).await {
            warn!(account_id, error = %e, "failed to send location confirmation");
        }
        return;
    }

    if is_live {
        // Live location share — acknowledge silently. Subsequent updates will
        // continue to try resolving pending tool requests.
        debug!(account_id, lat, lon, "received live location update");
        return;
    }

    // Static location — dispatch to the LLM.
    let text = format!("I'm sharing my location: {lat}, {lon}");
    if let Some(ref sink) = state.event_sink {
        sink.dispatch_to_chat(&text, reply_to, meta).await;
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}

fn is_owner_user(jid: &Jid, own_pn: Option<&Jid>, own_lid: Option<&Jid>) -> bool {
    own_pn.is_some_and(|pn| pn.is_same_user_as(jid))
        || own_lid.is_some_and(|lid| lid.is_same_user_as(jid))
}

/// Classify the inbound message kind based on its content.
///
/// Media types take priority over text — an image with a caption is still `Photo`,
/// not `Text`. This ensures the media handler runs and can include the caption
/// alongside the attachment.
fn classify_message(msg: &wa::Message, text: &str) -> ChannelMessageKind {
    if msg.image_message.is_some() {
        ChannelMessageKind::Photo
    } else if msg.audio_message.is_some() {
        if msg
            .audio_message
            .as_ref()
            .is_some_and(|a| a.ptt.unwrap_or(false))
        {
            ChannelMessageKind::Voice
        } else {
            ChannelMessageKind::Audio
        }
    } else if msg.video_message.is_some() {
        ChannelMessageKind::Video
    } else if msg.document_message.is_some() {
        ChannelMessageKind::Document
    } else if msg.location_message.is_some() || msg.live_location_message.is_some() {
        ChannelMessageKind::Location
    } else if !text.is_empty() {
        ChannelMessageKind::Text
    } else {
        ChannelMessageKind::Other
    }
}

/// Handle OTP challenge/verification flow for a non-allowlisted DM user.
///
/// Called when `dm_policy = Allowlist`, the peer is not on the allowlist, and
/// `otp_self_approval` is enabled.
#[allow(clippy::too_many_arguments)]
async fn handle_otp_flow(
    accounts: &AccountStateMap,
    account_id: &str,
    peer_id: &str,
    username: Option<&str>,
    sender_name: Option<&str>,
    body: &str,
    chat_jid: &Jid,
    state: &AccountState,
) {
    let has_pending = {
        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
        accts
            .get(account_id)
            .map(|s| {
                let otp = s.otp.lock().unwrap_or_else(|e| e.into_inner());
                otp.has_pending(peer_id)
            })
            .unwrap_or(false)
    };

    if has_pending {
        let trimmed = body.trim();
        if trimmed.len() != 6 || !trimmed.chars().all(|c| c.is_ascii_digit()) {
            // Non-code message while OTP pending — silently ignore.
            return;
        }

        let result = {
            let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
            match accts.get(account_id) {
                Some(s) => {
                    let mut otp = s.otp.lock().unwrap_or_else(|e| e.into_inner());
                    otp.verify(peer_id, trimmed)
                },
                None => return,
            }
        };

        match result {
            OtpVerifyResult::Approved => {
                let reply = wa::Message {
                    conversation: Some("Access granted! You can now use this bot.".into()),
                    ..Default::default()
                };
                let _ = state.send_message(chat_jid.clone(), reply).await;

                // Emit OTP resolved event for the gateway to persist the allowlist change.
                if let Some(ref sink) = state.event_sink {
                    sink.emit(ChannelEvent::OtpResolved {
                        channel_type: ChannelType::Whatsapp,
                        account_id: account_id.to_string(),
                        peer_id: peer_id.to_string(),
                        username: username.map(String::from),
                        resolution: "approved".to_string(),
                    })
                    .await;
                }
            },
            OtpVerifyResult::WrongCode { attempts_left } => {
                let reply = wa::Message {
                    conversation: Some(format!(
                        "Wrong code. {attempts_left} attempt{} remaining.",
                        if attempts_left == 1 {
                            ""
                        } else {
                            "s"
                        }
                    )),
                    ..Default::default()
                };
                let _ = state.send_message(chat_jid.clone(), reply).await;
            },
            OtpVerifyResult::LockedOut => {
                let reply = wa::Message {
                    conversation: Some("Too many failed attempts. Please try again later.".into()),
                    ..Default::default()
                };
                let _ = state.send_message(chat_jid.clone(), reply).await;
            },
            OtpVerifyResult::Expired => {
                let reply = wa::Message {
                    conversation: Some(
                        "Your code has expired. Please send any message to get a new code.".into(),
                    ),
                    ..Default::default()
                };
                let _ = state.send_message(chat_jid.clone(), reply).await;
            },
            OtpVerifyResult::NoPending => {},
        }
        return;
    }

    // No pending challenge — initiate one.
    let init_result = {
        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
        match accts.get(account_id) {
            Some(s) => {
                let mut otp = s.otp.lock().unwrap_or_else(|e| e.into_inner());
                otp.initiate(
                    peer_id,
                    username.map(String::from),
                    sender_name.map(String::from),
                )
            },
            None => return,
        }
    };

    match init_result {
        OtpInitResult::Created(code) => {
            info!(account_id, peer_id, code, "OTP challenge issued");
            let reply = wa::Message {
                conversation: Some(OTP_CHALLENGE_MSG.to_string()),
                ..Default::default()
            };
            let _ = state.send_message(chat_jid.clone(), reply).await;

            // Compute expires_at as epoch seconds (5 minutes from now).
            let expires_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64
                + 300;

            if let Some(ref sink) = state.event_sink {
                sink.emit(ChannelEvent::OtpChallenge {
                    channel_type: ChannelType::Whatsapp,
                    account_id: account_id.to_string(),
                    peer_id: peer_id.to_string(),
                    username: username.map(String::from),
                    sender_name: sender_name.map(String::from),
                    code,
                    expires_at,
                })
                .await;
            }
        },
        OtpInitResult::AlreadyPending => {
            // Resend the challenge message.
            let reply = wa::Message {
                conversation: Some(OTP_CHALLENGE_MSG.to_string()),
                ..Default::default()
            };
            let _ = state.send_message(chat_jid.clone(), reply).await;
        },
        OtpInitResult::LockedOut => {
            let reply = wa::Message {
                conversation: Some("Too many failed attempts. Please try again later.".into()),
                ..Default::default()
            };
            let _ = state.send_message(chat_jid.clone(), reply).await;
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn owner_self_chat_detected_without_is_from_me_when_sender_and_chat_are_owner() {
        let own_lid: Jid = "259557842534599@lid".parse().unwrap();
        let chat_jid: Jid = "259557842534599@lid".parse().unwrap();
        let sender_jid: Jid = "259557842534599@lid".parse().unwrap();

        let is_self_chat = is_owner_user(&chat_jid, None, Some(&own_lid));
        let sender_is_owner = is_owner_user(&sender_jid, None, Some(&own_lid));

        assert!(is_self_chat);
        assert!(sender_is_owner);
    }

    #[test]
    fn non_self_chat_from_me_is_not_treated_as_owner_self_chat() {
        let own_lid: Jid = "259557842534599@lid".parse().unwrap();
        let group_chat_jid: Jid = "120363456789@g.us".parse().unwrap();

        let is_self_chat = is_owner_user(&group_chat_jid, None, Some(&own_lid));
        assert!(!is_self_chat);
    }

    #[test]
    fn sender_must_match_owner_when_is_from_me_is_false() {
        let own_lid: Jid = "259557842534599@lid".parse().unwrap();
        let chat_jid: Jid = "259557842534599@lid".parse().unwrap();
        let other_sender: Jid = "11111111111@s.whatsapp.net".parse().unwrap();

        let is_self_chat = is_owner_user(&chat_jid, None, Some(&own_lid));
        let sender_is_owner = is_owner_user(&other_sender, None, Some(&own_lid));

        assert!(is_self_chat);
        assert!(!sender_is_owner);
    }
}
