use std::sync::Arc;

use {
    teloxide::{
        payloads::SendMessageSetters,
        prelude::*,
        types::{
            CallbackQuery, InlineKeyboardButton, InlineKeyboardMarkup, MediaKind, MessageKind,
            ParseMode, ThreadId,
        },
    },
    tracing::{debug, info, warn},
};

use {
    moltis_channels::{
        ChannelAttachment, ChannelEvent, ChannelMessageKind, ChannelMessageMeta, ChannelOutbound,
        ChannelReplyTarget, ChannelType,
        message_log::MessageLogEntry,
        otp::{approve_sender_via_otp, emit_otp_challenge, emit_otp_resolution},
    },
    moltis_common::types::ChatType,
};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, histogram, telegram as tg_metrics};

use crate::{
    access::{self, AccessDenied},
    otp::{OtpInitResult, OtpVerifyResult},
    state::AccountStateMap,
};

/// Parse a composite `to` address, falling back to `ChatId(0)` on invalid input.
/// Used by UI helpers (keyboards, cards) where propagating errors is impractical.
fn parse_chat_target_lossy(to: &str) -> (ChatId, Option<ThreadId>) {
    crate::topic::parse_chat_target(to).unwrap_or((ChatId(0), None))
}

/// Extract the forum-topic thread ID from a Telegram message, if present.
fn extract_thread_id(msg: &Message) -> Option<String> {
    msg.thread_id.map(|tid| tid.0.0.to_string())
}

/// Compose the outbound `to` address for a Telegram message, encoding the
/// forum-topic thread ID when present: `"chat_id:thread_id"`.
fn outbound_to_for_msg(msg: &Message) -> String {
    match extract_thread_id(msg) {
        Some(tid) => format!("{}:{}", msg.chat.id.0, tid),
        None => msg.chat.id.0.to_string(),
    }
}

/// Shared context injected into teloxide's dispatcher.
#[derive(Clone)]
pub struct HandlerContext {
    pub accounts: AccountStateMap,
    pub account_id: String,
}

/// Build the teloxide update handler.
pub fn build_handler() -> Handler<
    'static,
    DependencyMap,
    Result<(), Box<dyn std::error::Error + Send + Sync>>,
    teloxide::dispatching::DpHandlerDescription,
> {
    Update::filter_message().endpoint(handle_message)
}

/// Handle a single inbound Telegram message (called from manual polling loop).
pub async fn handle_message_direct(
    msg: Message,
    bot: &Bot,
    account_id: &str,
    accounts: &AccountStateMap,
) -> crate::Result<()> {
    #[cfg(feature = "metrics")]
    let start = std::time::Instant::now();

    #[cfg(feature = "metrics")]
    counter!(tg_metrics::MESSAGES_RECEIVED_TOTAL).increment(1);

    let text = extract_text(&msg);
    if text.is_none() && !has_media(&msg) {
        debug!(account_id, "ignoring non-text, non-media message");
        return Ok(());
    }

    let (config, bot_username, outbound, message_log, event_sink) = {
        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
        let state = match accts.get(account_id) {
            Some(s) => s,
            None => {
                warn!(account_id, "handler: account not found in state map");
                return Ok(());
            },
        };
        (
            state.config.clone(),
            state.bot_username.clone(),
            Arc::clone(&state.outbound),
            state.message_log.clone(),
            state.event_sink.clone(),
        )
    };

    let (chat_type, group_id) = classify_chat(&msg);
    let peer_id = msg
        .from
        .as_ref()
        .map(|u| u.id.0.to_string())
        .unwrap_or_default();
    let sender_name = msg.from.as_ref().and_then(|u| {
        let first = &u.first_name;
        let last = u.last_name.as_deref().unwrap_or("");
        let name = format!("{first} {last}").trim().to_string();
        if name.is_empty() {
            u.username.clone()
        } else {
            Some(name)
        }
    });

    let bot_mentioned = check_bot_mentioned(&msg, bot_username.as_deref());

    debug!(
        account_id,
        ?chat_type,
        peer_id,
        ?bot_mentioned,
        "checking access"
    );

    let username = msg.from.as_ref().and_then(|u| u.username.clone());
    let inbound_kind = message_kind(&msg);
    let text_len = text.as_ref().map_or(0, |body| body.len());
    info!(
        account_id,
        chat_id = msg.chat.id.0,
        message_id = msg.id.0,
        peer_id,
        username = ?username,
        sender_name = ?sender_name,
        kind = ?inbound_kind,
        has_media = has_media(&msg),
        has_text = text.is_some(),
        text_len,
        "telegram inbound message received"
    );

    // Access control
    let access_result = access::check_access(
        &config,
        &chat_type,
        &peer_id,
        username.as_deref(),
        group_id.as_deref(),
        bot_mentioned,
    );
    let access_granted = access_result.is_ok();

    // Log every inbound message (before returning on denial).
    if let Some(ref log) = message_log {
        let chat_type_str = match chat_type {
            ChatType::Dm => "dm",
            ChatType::Group => "group",
            ChatType::Channel => "channel",
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let entry = MessageLogEntry {
            id: 0,
            account_id: account_id.to_string(),
            channel_type: ChannelType::Telegram.to_string(),
            peer_id: peer_id.clone(),
            username: username.clone(),
            sender_name: sender_name.clone(),
            chat_id: msg.chat.id.0.to_string(),
            chat_type: chat_type_str.into(),
            body: text.clone().unwrap_or_default(),
            access_granted,
            created_at: now,
        };
        if let Err(e) = log.log(entry).await {
            warn!(account_id, "failed to log message: {e}");
        }
    }

    // Emit channel event for real-time UI updates.
    if let Some(ref sink) = event_sink {
        sink.emit(ChannelEvent::InboundMessage {
            channel_type: ChannelType::Telegram,
            account_id: account_id.to_string(),
            peer_id: peer_id.clone(),
            username: username.clone(),
            sender_name: sender_name.clone(),
            message_count: None,
            access_granted,
        })
        .await;
    }

    if let Err(reason) = access_result {
        warn!(account_id, %reason, peer_id, username = ?username, "handler: access denied");
        #[cfg(feature = "metrics")]
        counter!(tg_metrics::ACCESS_CONTROL_DENIALS_TOTAL).increment(1);

        // OTP self-approval for non-allowlisted DM users.
        if reason == AccessDenied::NotOnAllowlist
            && chat_type == ChatType::Dm
            && config.otp_self_approval
        {
            handle_otp_flow(
                accounts,
                account_id,
                &peer_id,
                username.as_deref(),
                sender_name.as_deref(),
                text.as_deref(),
                &msg,
                event_sink.as_deref(),
            )
            .await;
        }

        return Ok(());
    }

    debug!(account_id, "handler: access granted");

    // Check for voice/audio messages and transcribe them.
    // `voice_audio` carries the raw bytes + format so we can save them to the
    // session media directory once we have a reply target.
    let (body, attachments, voice_audio): (
        String,
        Vec<ChannelAttachment>,
        Option<(Vec<u8>, String)>,
    ) = if let Some(voice_file) = extract_voice_file(&msg) {
        // If STT is not configured, reply with guidance and do not dispatch to the LLM.
        if let Some(ref sink) = event_sink
            && !sink.voice_stt_available().await
        {
            if let Err(e) = outbound
                .send_text(
                    account_id,
                    &outbound_to_for_msg(&msg),
                    "I can't understand voice, you did not configure it, please visit Settings -> Voice",
                    None,
                )
                .await
            {
                warn!(account_id, "failed to send STT setup hint: {e}");
            }
            return Ok(());
        }

        // Try to transcribe the voice message
        if let Some(ref sink) = event_sink {
            match download_telegram_file(bot, &voice_file.file_id).await {
                Ok(audio_data) => {
                    debug!(
                        account_id,
                        file_id = %voice_file.file_id,
                        format = %voice_file.format,
                        size = audio_data.len(),
                        "downloaded voice file, transcribing"
                    );
                    let saved_audio = Some((audio_data.clone(), voice_file.format.clone()));
                    match sink.transcribe_voice(&audio_data, &voice_file.format).await {
                        Ok(transcribed) if transcribed.trim().is_empty() => {
                            warn!(
                                account_id,
                                audio_size = audio_data.len(),
                                "voice transcription returned empty text"
                            );
                            (
                                "[Voice message - could not transcribe]".to_string(),
                                Vec::new(),
                                saved_audio,
                            )
                        },
                        Ok(transcribed) => {
                            debug!(
                                account_id,
                                text_len = transcribed.len(),
                                "voice transcription successful"
                            );
                            // Combine with any caption if present
                            let caption = text.clone().unwrap_or_default();
                            let body = if caption.is_empty() {
                                transcribed
                            } else {
                                format!("{}\n\n[Voice message]: {}", caption, transcribed)
                            };
                            (body, Vec::new(), saved_audio)
                        },
                        Err(e) => {
                            warn!(account_id, error = %e, "voice transcription failed");
                            // Fall back to caption or indicate transcription failed
                            (
                                text.clone().unwrap_or_else(|| {
                                    "[Voice message - transcription unavailable]".to_string()
                                }),
                                Vec::new(),
                                saved_audio,
                            )
                        },
                    }
                },
                Err(e) => {
                    warn!(account_id, error = %e, "failed to download voice file");
                    (
                        text.clone()
                            .unwrap_or_else(|| "[Voice message - download failed]".to_string()),
                        Vec::new(),
                        None,
                    )
                },
            }
        } else {
            // No event sink, can't transcribe
            (
                text.clone()
                    .unwrap_or_else(|| "[Voice message]".to_string()),
                Vec::new(),
                None,
            )
        }
    } else if let Some(photo_file) = extract_photo_file(&msg) {
        // Handle photo messages - download and send as multimodal content
        match download_telegram_file(bot, &photo_file.file_id).await {
            Ok(image_data) => {
                debug!(
                    account_id,
                    file_id = %photo_file.file_id,
                    size = image_data.len(),
                    "downloaded photo"
                );

                // Optimize image for LLM consumption (resize if needed, compress)
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
                        (image_data, photo_file.media_type)
                    },
                };

                let attachment = ChannelAttachment {
                    media_type,
                    data: final_data,
                };
                // Use caption as text, or empty string if no caption
                let caption = text.clone().unwrap_or_default();
                (caption, vec![attachment], None)
            },
            Err(e) => {
                warn!(account_id, error = %e, "failed to download photo");
                (
                    text.clone()
                        .unwrap_or_else(|| "[Photo - download failed]".to_string()),
                    Vec::new(),
                    None,
                )
            },
        }
    } else if let Some(document_file) = extract_document_file(&msg) {
        // Handle documents/files - only download supported types to avoid
        // wasting bandwidth on files we cannot process.
        let caption = text.clone().unwrap_or_default();
        let doc_label = format_document_label(
            document_file.file_name.as_deref(),
            &document_file.media_type,
        );

        if !is_supported_document_type(&document_file.media_type) {
            debug!(
                account_id,
                media_type = %document_file.media_type,
                file_name = ?document_file.file_name,
                "skipping unsupported document type"
            );
            let body = if caption.is_empty() {
                doc_label
            } else {
                format!("{caption}\n{doc_label}")
            };
            (body, Vec::new(), None)
        } else {
            match download_telegram_file(bot, &document_file.file_id).await {
                Ok(document_data) => {
                    debug!(
                        account_id,
                        file_id = %document_file.file_id,
                        media_type = %document_file.media_type,
                        file_name = ?document_file.file_name,
                        size = document_data.len(),
                        "downloaded document"
                    );

                    if document_file.media_type.starts_with("image/") {
                        // Optimize image documents the same way as photo messages
                        let (final_data, media_type) =
                            match moltis_media::image_ops::optimize_for_llm(&document_data, None) {
                                Ok(optimized) => {
                                    if optimized.was_resized {
                                        info!(
                                            account_id,
                                            original_size = document_data.len(),
                                            final_size = optimized.data.len(),
                                            original_dims = %format!("{}x{}", optimized.original_width, optimized.original_height),
                                            final_dims = %format!("{}x{}", optimized.final_width, optimized.final_height),
                                            "resized document image for LLM"
                                        );
                                    }
                                    (optimized.data, optimized.media_type)
                                },
                                Err(e) => {
                                    warn!(account_id, error = %e, "failed to optimize document image, using original");
                                    (document_data, document_file.media_type.clone())
                                },
                            };
                        let attachment = ChannelAttachment {
                            media_type,
                            data: final_data,
                        };
                        (caption, vec![attachment], None)
                    } else if let Some(extracted_text) =
                        extract_text_document_content(&document_data, &document_file.media_type)
                    {
                        let body = if caption.is_empty() {
                            format!("{doc_label}\n\n{extracted_text}")
                        } else {
                            format!("{caption}\n\n{doc_label}\n\n{extracted_text}")
                        };
                        (body, Vec::new(), None)
                    } else {
                        let body = if caption.is_empty() {
                            doc_label
                        } else {
                            format!("{caption}\n{doc_label}")
                        };
                        (body, Vec::new(), None)
                    }
                },
                Err(e) => {
                    warn!(
                        account_id,
                        error = %e,
                        file_id = %document_file.file_id,
                        media_type = %document_file.media_type,
                        file_name = ?document_file.file_name,
                        "failed to download document"
                    );

                    let body = if caption.is_empty() {
                        format!("{doc_label}\n[Document - download failed]")
                    } else {
                        format!("{caption}\n{doc_label}\n[Document - download failed]")
                    };

                    (body, Vec::new(), None)
                },
            }
        }
    } else if let Some(loc_info) = extract_location(&msg) {
        let lat = loc_info.latitude;
        let lon = loc_info.longitude;

        // Handle location sharing: update stored location and resolve any pending tool request.
        let resolved = if let Some(ref sink) = event_sink {
            let reply_target = ChannelReplyTarget {
                channel_type: ChannelType::Telegram,
                account_id: account_id.to_string(),
                chat_id: msg.chat.id.0.to_string(),
                message_id: Some(msg.id.0.to_string()),
                thread_id: extract_thread_id(&msg),
            };
            sink.update_location(&reply_target, lat, lon).await
        } else {
            false
        };

        info!(
            account_id,
            chat_id = msg.chat.id.0,
            message_id = msg.id.0,
            lat,
            lon,
            is_live = loc_info.is_live,
            resolved_pending_request = resolved,
            "telegram location received"
        );

        if resolved {
            // Pending tool request was resolved — the LLM will respond via the tool flow.
            if let Err(e) = outbound
                .send_text_silent(
                    account_id,
                    &outbound_to_for_msg(&msg),
                    "Location updated.",
                    None,
                )
                .await
            {
                warn!(account_id, "failed to send location confirmation: {e}");
            }
            return Ok(());
        }

        if loc_info.is_live {
            // Live location share — acknowledge silently, subsequent updates arrive
            // as EditedMessage and are handled by handle_edited_location().
            if let Err(e) = outbound
                .send_text_silent(
                    account_id,
                    &outbound_to_for_msg(&msg),
                    "Live location tracking started. Your location will be updated automatically.",
                    None,
                )
                .await
            {
                warn!(account_id, "failed to send live location ack: {e}");
            }
            return Ok(());
        }

        // Static location share — dispatch to LLM so it can acknowledge.
        (
            format!("I'm sharing my location: {lat}, {lon}"),
            Vec::new(),
            None,
        )
    } else {
        // Log unhandled media types so we know when users are sending attachments we don't process
        if let Some(media_type) = describe_media_kind(&msg) {
            info!(
                account_id,
                peer_id, media_type, "received unhandled attachment type"
            );
        }
        (text.unwrap_or_default(), Vec::new(), None)
    };

    // Dispatch to the chat session (per-channel session key derived by the sink).
    // The reply target tells the gateway where to send the LLM response back.
    let has_content = !body.is_empty() || !attachments.is_empty();
    if !has_content {
        warn!(
            account_id,
            chat_id = msg.chat.id.0,
            message_id = msg.id.0,
            kind = ?inbound_kind,
            "telegram message produced empty body, skipping dispatch"
        );
    }
    if let Some(ref sink) = event_sink
        && has_content
    {
        let reply_target = ChannelReplyTarget {
            channel_type: ChannelType::Telegram,
            account_id: account_id.to_string(),
            chat_id: msg.chat.id.0.to_string(),
            message_id: Some(msg.id.0.to_string()),
            thread_id: extract_thread_id(&msg),
        };

        info!(
            account_id,
            chat_id = %reply_target.chat_id,
            message_id = ?reply_target.message_id,
            body_len = body.len(),
            attachment_count = attachments.len(),
            message_kind = ?inbound_kind,
            "telegram inbound dispatched to chat"
        );

        // Intercept slash commands before dispatching to the LLM.
        if body.starts_with('/') {
            let cmd_text = body.trim_start_matches('/');
            let cmd = cmd_text.split_whitespace().next().unwrap_or("");
            if should_intercept_slash_command(cmd, cmd_text) {
                // For /context, send a formatted card with inline keyboard.
                if cmd == "context" {
                    let context_result =
                        sink.dispatch_command("context", reply_target.clone()).await;
                    let bot = {
                        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
                        accts.get(account_id).map(|s| s.bot.clone())
                    };
                    if let Some(bot) = bot {
                        match context_result {
                            Ok(text) => {
                                send_context_card(&bot, &reply_target.outbound_to(), &text).await;
                            },
                            Err(e) => {
                                let _ = bot
                                    .send_message(
                                        ChatId(reply_target.chat_id.parse().unwrap_or(0)),
                                        format!("Error: {e}"),
                                    )
                                    .await;
                            },
                        }
                    }
                    return Ok(());
                }

                // For /model without args, send an inline keyboard to pick a model.
                if cmd == "agent" && cmd_text.trim() == "agent" {
                    let list_result = sink.dispatch_command("agent", reply_target.clone()).await;
                    let bot = {
                        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
                        accts.get(account_id).map(|s| s.bot.clone())
                    };
                    if let Some(bot) = bot {
                        match list_result {
                            Ok(text) => {
                                send_agent_keyboard(&bot, &reply_target.outbound_to(), &text).await;
                            },
                            Err(e) => {
                                let _ = bot
                                    .send_message(
                                        ChatId(reply_target.chat_id.parse().unwrap_or(0)),
                                        format!("Error: {e}"),
                                    )
                                    .await;
                            },
                        }
                    }
                    return Ok(());
                }

                // For /model without args, send an inline keyboard to pick a model.
                if cmd == "model" && cmd_text.trim() == "model" {
                    let list_result = sink.dispatch_command("model", reply_target.clone()).await;
                    let bot = {
                        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
                        accts.get(account_id).map(|s| s.bot.clone())
                    };
                    if let Some(bot) = bot {
                        match list_result {
                            Ok(text) => {
                                send_model_keyboard(&bot, &reply_target.outbound_to(), &text).await;
                            },
                            Err(e) => {
                                let _ = bot
                                    .send_message(
                                        ChatId(reply_target.chat_id.parse().unwrap_or(0)),
                                        format!("Error: {e}"),
                                    )
                                    .await;
                            },
                        }
                    }
                    return Ok(());
                }

                // For /sandbox without args, send toggle + image keyboard.
                if cmd == "sandbox" && cmd_text.trim() == "sandbox" {
                    let list_result = sink.dispatch_command("sandbox", reply_target.clone()).await;
                    let bot = {
                        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
                        accts.get(account_id).map(|s| s.bot.clone())
                    };
                    if let Some(bot) = bot {
                        match list_result {
                            Ok(text) => {
                                send_sandbox_keyboard(&bot, &reply_target.outbound_to(), &text)
                                    .await;
                            },
                            Err(e) => {
                                let _ = bot
                                    .send_message(
                                        ChatId(reply_target.chat_id.parse().unwrap_or(0)),
                                        format!("Error: {e}"),
                                    )
                                    .await;
                            },
                        }
                    }
                    return Ok(());
                }

                // For /sessions without args, send an inline keyboard instead of plain text.
                if cmd == "sessions" && cmd_text.trim() == "sessions" {
                    let list_result = sink
                        .dispatch_command("sessions", reply_target.clone())
                        .await;
                    let bot = {
                        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
                        accts.get(account_id).map(|s| s.bot.clone())
                    };
                    if let Some(bot) = bot {
                        match list_result {
                            Ok(text) => {
                                send_sessions_keyboard(&bot, &reply_target.outbound_to(), &text)
                                    .await;
                            },
                            Err(e) => {
                                let _ = bot
                                    .send_message(
                                        ChatId(reply_target.chat_id.parse().unwrap_or(0)),
                                        format!("Error: {e}"),
                                    )
                                    .await;
                            },
                        }
                    }
                    return Ok(());
                }

                let response = if cmd == "help" {
                    "Available commands:\n/new — Start a new session\n/sessions — List and switch sessions\n/agent — Switch session agent\n/model — Switch provider/model\n/sandbox — Toggle sandbox and choose image\n/sh — Enable command mode (/sh off to exit)\n/clear — Clear session history\n/compact — Compact session (summarize)\n/context — Show session context info\n/help — Show this help".to_string()
                } else {
                    match sink.dispatch_command(cmd_text, reply_target.clone()).await {
                        Ok(msg) => msg,
                        Err(e) => format!("Error: {e}"),
                    }
                };
                // Get the outbound Arc before awaiting (avoid holding RwLockReadGuard across await).
                let outbound = {
                    let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
                    accts.get(account_id).map(|s| Arc::clone(&s.outbound))
                };
                if let Some(outbound) = outbound
                    && let Err(e) = outbound
                        .send_text(account_id, &reply_target.outbound_to(), &response, None)
                        .await
                {
                    warn!(account_id, "failed to send command response: {e}");
                }
                return Ok(());
            }
        }

        // Save voice audio to the session media directory (best-effort).
        let audio_filename = if let Some((ref audio_data, ref format)) = voice_audio {
            let filename = format!("voice-tg-{}.{format}", msg.id.0);
            sink.save_channel_voice(audio_data, &filename, &reply_target)
                .await
        } else {
            None
        };

        let meta = ChannelMessageMeta {
            channel_type: ChannelType::Telegram,
            sender_name: sender_name.clone(),
            username: username.clone(),
            message_kind: message_kind(&msg),
            model: config.model.clone(),
            audio_filename,
        };

        if attachments.is_empty() {
            sink.dispatch_to_chat(&body, reply_target, meta).await;
        } else {
            sink.dispatch_to_chat_with_attachments(&body, attachments, reply_target, meta)
                .await;
        }
    }

    #[cfg(feature = "metrics")]
    histogram!(tg_metrics::POLLING_DURATION_SECONDS).record(start.elapsed().as_secs_f64());

    Ok(())
}

fn should_intercept_slash_command(cmd: &str, cmd_text: &str) -> bool {
    match cmd {
        "new" | "clear" | "compact" | "context" | "model" | "sandbox" | "sessions" | "agent"
        | "help" => true,
        "sh" => {
            let args = cmd_text.strip_prefix(cmd).unwrap_or("").trim();
            args.is_empty() || matches!(args, "on" | "off" | "exit" | "status")
        },
        _ => false,
    }
}

/// OTP challenge message sent to the Telegram user.
///
/// **Security invariant:** this message must NEVER contain the actual
/// verification code.  The code is only visible to the bot owner in the
/// web UI (Channels → Senders).  Leaking it here would let any
/// unauthenticated user self-approve without admin awareness.
pub(crate) const OTP_CHALLENGE_MSG: &str = "To use this bot, please enter the verification code.\n\nAsk the bot owner for the code \u{2014} it is visible in the web UI under <b>Channels \u{2192} Senders</b>.\n\nThe code expires in 5 minutes.";

/// Handle OTP challenge/verification flow for a non-allowlisted DM user.
///
/// Called when `dm_policy = Allowlist`, the peer is not on the allowlist, and
/// `otp_self_approval` is enabled. Manages the full lifecycle:
/// - First message: issue a 6-digit OTP challenge
/// - Code reply: verify and auto-approve on match
/// - Non-code messages while pending: silently ignored (flood protection)
#[allow(clippy::too_many_arguments)]
async fn handle_otp_flow(
    accounts: &AccountStateMap,
    account_id: &str,
    peer_id: &str,
    username: Option<&str>,
    sender_name: Option<&str>,
    text: Option<&str>,
    msg: &Message,
    event_sink: Option<&dyn moltis_channels::ChannelEventSink>,
) {
    let chat_id = msg.chat.id;

    // Resolve bot early (needed for sending messages).
    let bot = {
        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
        accts.get(account_id).map(|s| s.bot.clone())
    };
    let bot = match bot {
        Some(b) => b,
        None => return,
    };

    // Check current OTP state.
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
        // A challenge is already pending. Check if the user sent a 6-digit code.
        let body = text.unwrap_or("").trim();
        let is_code = body.len() == 6 && body.chars().all(|c| c.is_ascii_digit());

        if !is_code {
            // Silent ignore — flood protection.
            return;
        }

        // Verify the code.
        let result = {
            let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
            match accts.get(account_id) {
                Some(s) => {
                    let mut otp = s.otp.lock().unwrap_or_else(|e| e.into_inner());
                    otp.verify(peer_id, body)
                },
                None => return,
            }
        };

        match result {
            OtpVerifyResult::Approved => {
                let identifier = username.unwrap_or(peer_id);
                approve_sender_via_otp(
                    event_sink,
                    ChannelType::Telegram,
                    account_id,
                    identifier,
                    peer_id,
                    username,
                )
                .await;

                let _ = bot
                    .send_message(chat_id, "Verified! You now have access to this bot.")
                    .await;

                #[cfg(feature = "metrics")]
                counter!(tg_metrics::OTP_VERIFICATIONS_TOTAL, "result" => "approved").increment(1);
            },
            OtpVerifyResult::WrongCode { attempts_left } => {
                let _ = bot
                    .send_message(
                        chat_id,
                        format!(
                            "Incorrect code. {attempts_left} attempt{} remaining.",
                            if attempts_left == 1 {
                                ""
                            } else {
                                "s"
                            }
                        ),
                    )
                    .await;

                #[cfg(feature = "metrics")]
                counter!(tg_metrics::OTP_VERIFICATIONS_TOTAL, "result" => "wrong_code")
                    .increment(1);
            },
            OtpVerifyResult::LockedOut => {
                let _ = bot
                    .send_message(chat_id, "Too many failed attempts. Please try again later.")
                    .await;

                emit_otp_resolution(
                    event_sink,
                    ChannelType::Telegram,
                    account_id,
                    peer_id,
                    username,
                    "locked_out",
                )
                .await;

                #[cfg(feature = "metrics")]
                counter!(tg_metrics::OTP_VERIFICATIONS_TOTAL, "result" => "locked_out")
                    .increment(1);
            },
            OtpVerifyResult::Expired => {
                let _ = bot
                    .send_message(
                        chat_id,
                        "Your code has expired. Send any message to get a new one.",
                    )
                    .await;

                emit_otp_resolution(
                    event_sink,
                    ChannelType::Telegram,
                    account_id,
                    peer_id,
                    username,
                    "expired",
                )
                .await;

                #[cfg(feature = "metrics")]
                counter!(tg_metrics::OTP_VERIFICATIONS_TOTAL, "result" => "expired").increment(1);
            },
            OtpVerifyResult::NoPending => {
                // Shouldn't happen since we checked has_pending, but handle gracefully.
            },
        }
    } else {
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
                let _ = bot
                    .send_message(chat_id, OTP_CHALLENGE_MSG)
                    .parse_mode(ParseMode::Html)
                    .await;

                // Emit OTP challenge event for the admin UI.
                let expires_at = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64
                    + 300;

                emit_otp_challenge(
                    event_sink,
                    ChannelType::Telegram,
                    account_id,
                    peer_id,
                    username,
                    sender_name,
                    code,
                    expires_at,
                )
                .await;

                #[cfg(feature = "metrics")]
                counter!(tg_metrics::OTP_CHALLENGES_TOTAL).increment(1);
            },
            OtpInitResult::AlreadyPending | OtpInitResult::LockedOut => {
                // Silent ignore.
            },
        }
    }
}

/// Handle an edited message — only processes live location updates.
///
/// Telegram sends live location updates as `EditedMessage` with `MediaKind::Location`.
/// We silently update the cached location without dispatching to the LLM or
/// re-checking access (the user was already approved on the initial share).
pub async fn handle_edited_location(
    msg: Message,
    account_id: &str,
    accounts: &AccountStateMap,
) -> crate::Result<()> {
    let Some(loc_info) = extract_location(&msg) else {
        // Not a location edit — ignore (could be a text edit, etc.).
        return Ok(());
    };
    let lat = loc_info.latitude;
    let lon = loc_info.longitude;

    debug!(
        account_id,
        lat,
        lon,
        chat_id = msg.chat.id.0,
        "live location update"
    );
    info!(
        account_id,
        chat_id = msg.chat.id.0,
        message_id = msg.id.0,
        lat,
        lon,
        "telegram live location update received"
    );

    let event_sink = {
        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
        accts.get(account_id).and_then(|s| s.event_sink.clone())
    };

    if let Some(ref sink) = event_sink {
        let reply_target = ChannelReplyTarget {
            channel_type: ChannelType::Telegram,
            account_id: account_id.to_string(),
            chat_id: msg.chat.id.0.to_string(),
            message_id: Some(msg.id.0.to_string()),
            thread_id: extract_thread_id(&msg),
        };
        sink.update_location(&reply_target, lat, lon).await;
    }

    Ok(())
}

/// Handle a single inbound Telegram message (teloxide dispatcher endpoint).
async fn handle_message(
    msg: Message,
    bot: Bot,
    ctx: Arc<HandlerContext>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    handle_message_direct(msg, &bot, &ctx.account_id, &ctx.accounts).await?;
    Ok(())
}

/// Send a sessions list as an inline keyboard.
///
/// Parses the text response from `dispatch_command("sessions")` to extract
/// session labels, then sends an inline keyboard with one button per session.
async fn send_sessions_keyboard(bot: &Bot, to: &str, sessions_text: &str) {
    let (chat, thread_id) = parse_chat_target_lossy(to);

    // Parse numbered lines like "1. Session label (5 msgs) *"
    let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for line in sessions_text.lines() {
        let trimmed = line.trim();
        // Match lines starting with a number followed by ". "
        if let Some(dot_pos) = trimmed.find(". ")
            && let Ok(n) = trimmed[..dot_pos].parse::<usize>()
        {
            let label_part = &trimmed[dot_pos + 2..];
            let is_active = label_part.ends_with('*');
            let display = if is_active {
                format!("● {}", label_part.trim_end_matches('*').trim())
            } else {
                format!("○ {label_part}")
            };
            buttons.push(vec![InlineKeyboardButton::callback(
                display,
                format!("sessions_switch:{n}"),
            )]);
        }
    }

    if buttons.is_empty() {
        let mut req = bot.send_message(chat, sessions_text);
        if let Some(tid) = thread_id {
            req = req.message_thread_id(tid);
        }
        let _ = req.await;
        return;
    }

    let keyboard = InlineKeyboardMarkup::new(buttons);
    let mut req = bot
        .send_message(chat, "Select a session:")
        .reply_markup(keyboard);
    if let Some(tid) = thread_id {
        req = req.message_thread_id(tid);
    }
    let _ = req.await;
}

/// Send agent selection as an inline keyboard.
///
/// Parses numbered lines like:
/// `1. 🤖 Main [main] (default) *`
async fn send_agent_keyboard(bot: &Bot, to: &str, agents_text: &str) {
    let (chat, thread_id) = parse_chat_target_lossy(to);
    let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for line in agents_text.lines() {
        let trimmed = line.trim();
        if let Some(dot_pos) = trimmed.find(". ")
            && let Ok(n) = trimmed[..dot_pos].parse::<usize>()
        {
            let label_part = &trimmed[dot_pos + 2..];
            let is_active = label_part.ends_with('*');
            let display = if is_active {
                format!("● {}", label_part.trim_end_matches('*').trim())
            } else {
                format!("○ {label_part}")
            };
            buttons.push(vec![InlineKeyboardButton::callback(
                display,
                format!("agent_switch:{n}"),
            )]);
        }
    }

    if buttons.is_empty() {
        let mut req = bot.send_message(chat, agents_text);
        if let Some(tid) = thread_id {
            req = req.message_thread_id(tid);
        }
        let _ = req.await;
        return;
    }

    let keyboard = InlineKeyboardMarkup::new(buttons);
    let mut req = bot
        .send_message(chat, "Select an agent:")
        .reply_markup(keyboard);
    if let Some(tid) = thread_id {
        req = req.message_thread_id(tid);
    }
    let _ = req.await;
}

/// Send context info as a formatted HTML card with blockquote sections.
///
/// Parses the markdown context response from `dispatch_command("context")`
/// and renders it as a structured Telegram HTML message.
async fn send_context_card(bot: &Bot, to: &str, context_text: &str) {
    let (chat, thread_id) = parse_chat_target_lossy(to);

    // Parse "**Key:** value" lines from the markdown response into a map.
    let mut fields: Vec<(&str, String)> = Vec::new();
    for line in context_text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("**")
            && let Some(end) = rest.find("**")
        {
            let label = &rest[..end];
            let raw_value = rest[end + 2..].trim();
            // Strip markdown backticks from value
            let value = raw_value.replace('`', "");
            fields.push((label, escape_html_simple(&value)));
        }
    }

    let get = |key: &str| -> String {
        fields
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };

    let session = get("Session:");
    let messages = get("Messages:");
    let provider = get("Provider:");
    let model = get("Model:");
    let sandbox = get("Sandbox:");
    let plugins_raw = get("Plugins:");
    let tokens = get("Tokens:");

    // Format plugins as individual lines
    let plugins_section = if plugins_raw == "none" || plugins_raw.is_empty() {
        "  <i>none</i>".to_string()
    } else {
        plugins_raw
            .split(", ")
            .map(|p| format!("  ▸ {p}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    // Sandbox indicator
    let sandbox_icon = if sandbox.starts_with("on") {
        "🟢"
    } else {
        "⚫"
    };

    let html = format!(
        "\
<b>📋 Session Context</b>

<blockquote><b>🤖 Model</b>
{provider} · <code>{model}</code>

<b>{sandbox_icon} Sandbox</b>
{sandbox}

<b>🧩 Plugins</b>
{plugins_section}</blockquote>

<code>Session   {session}
Messages  {messages}
Tokens    {tokens}</code>"
    );

    let mut req = bot.send_message(chat, html).parse_mode(ParseMode::Html);
    if let Some(tid) = thread_id {
        req = req.message_thread_id(tid);
    }
    let _ = req.await;
}

/// Send model selection as an inline keyboard.
///
/// If the response starts with `providers:`, show a provider picker first.
/// Otherwise show the model list directly.
async fn send_model_keyboard(bot: &Bot, to: &str, text: &str) {
    let (chat, thread_id) = parse_chat_target_lossy(to);

    let is_provider_list = text.starts_with("providers:");

    let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "providers:" {
            continue;
        }
        if let Some(dot_pos) = trimmed.find(". ")
            && let Ok(n) = trimmed[..dot_pos].parse::<usize>()
        {
            let label_part = &trimmed[dot_pos + 2..];
            let is_active = label_part.ends_with('*');
            let clean = label_part.trim_end_matches('*').trim();
            let display = if is_active {
                format!("● {clean}")
            } else {
                format!("○ {clean}")
            };

            if is_provider_list {
                // Extract provider name (before the parenthesized count).
                let provider_name = clean.rfind(" (").map(|i| &clean[..i]).unwrap_or(clean);
                buttons.push(vec![InlineKeyboardButton::callback(
                    display,
                    format!("model_provider:{provider_name}"),
                )]);
            } else {
                buttons.push(vec![InlineKeyboardButton::callback(
                    display,
                    format!("model_switch:{n}"),
                )]);
            }
        }
    }

    if buttons.is_empty() {
        let mut req = bot.send_message(chat, "No models available.");
        if let Some(tid) = thread_id {
            req = req.message_thread_id(tid);
        }
        let _ = req.await;
        return;
    }

    let heading = if is_provider_list {
        "🤖 Select a provider:"
    } else {
        "🤖 Select a model:"
    };

    let keyboard = InlineKeyboardMarkup::new(buttons);
    let mut req = bot.send_message(chat, heading).reply_markup(keyboard);
    if let Some(tid) = thread_id {
        req = req.message_thread_id(tid);
    }
    let _ = req.await;
}

/// Send sandbox status with toggle button and image picker.
///
/// First line is `status:on` or `status:off`. Remaining lines are numbered
/// images, with `*` marking the current one.
async fn send_sandbox_keyboard(bot: &Bot, to: &str, text: &str) {
    let (chat, thread_id) = parse_chat_target_lossy(to);

    let mut is_on = false;
    let mut image_buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(status) = trimmed.strip_prefix("status:") {
            is_on = status == "on";
            continue;
        }
        if let Some(dot_pos) = trimmed.find(". ")
            && let Ok(n) = trimmed[..dot_pos].parse::<usize>()
        {
            let label_part = &trimmed[dot_pos + 2..];
            let is_active = label_part.ends_with('*');
            let clean = label_part.trim_end_matches('*').trim();
            let display = if is_active {
                format!("● {clean}")
            } else {
                format!("○ {clean}")
            };
            image_buttons.push(vec![InlineKeyboardButton::callback(
                display,
                format!("sandbox_image:{n}"),
            )]);
        }
    }

    // Toggle button at the top.
    let toggle_label = if is_on {
        "🟢 Sandbox ON — tap to disable"
    } else {
        "⚫ Sandbox OFF — tap to enable"
    };
    let toggle_action = if is_on {
        "sandbox_toggle:off"
    } else {
        "sandbox_toggle:on"
    };

    let mut buttons = vec![vec![InlineKeyboardButton::callback(
        toggle_label.to_string(),
        toggle_action.to_string(),
    )]];
    buttons.extend(image_buttons);

    let keyboard = InlineKeyboardMarkup::new(buttons);
    let mut req = bot
        .send_message(chat, "⚙️ Sandbox settings:")
        .reply_markup(keyboard);
    if let Some(tid) = thread_id {
        req = req.message_thread_id(tid);
    }
    let _ = req.await;
}

fn escape_html_simple(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Handle a Telegram callback query (inline keyboard button press).
pub async fn handle_callback_query(
    query: CallbackQuery,
    _bot: &Bot,
    account_id: &str,
    accounts: &AccountStateMap,
) -> crate::Result<()> {
    let data = match query.data {
        Some(ref d) => d.as_str(),
        None => return Ok(()),
    };

    // Answer the callback to dismiss the loading spinner.
    let bot = {
        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
        accts.get(account_id).map(|s| s.bot.clone())
    };

    // Determine which command this callback is for.
    let cmd_text = if let Some(n_str) = data.strip_prefix("sessions_switch:") {
        Some(format!("sessions {n_str}"))
    } else if let Some(n_str) = data.strip_prefix("agent_switch:") {
        Some(format!("agent {n_str}"))
    } else if let Some(n_str) = data.strip_prefix("model_switch:") {
        Some(format!("model {n_str}"))
    } else if let Some(val) = data.strip_prefix("sandbox_toggle:") {
        Some(format!("sandbox {val}"))
    } else if let Some(n_str) = data.strip_prefix("sandbox_image:") {
        Some(format!("sandbox image {n_str}"))
    } else if data.starts_with("model_provider:") {
        // Handled separately below — no simple cmd_text.
        None
    } else {
        if let Some(ref bot) = bot {
            let _ = bot.answer_callback_query(&query.id).await;
        }
        return Ok(());
    };

    let chat_id = query
        .message
        .as_ref()
        .map(|m| m.chat().id.0.to_string())
        .unwrap_or_default();

    if chat_id.is_empty() {
        return Ok(());
    }

    let (event_sink, outbound) = {
        let accts = accounts.read().unwrap_or_else(|e| e.into_inner());
        let state = match accts.get(account_id) {
            Some(s) => s,
            None => return Ok(()),
        };
        (state.event_sink.clone(), Arc::clone(&state.outbound))
    };

    let callback_thread_id = query
        .message
        .as_ref()
        .and_then(|m| m.regular_message())
        .and_then(|m| m.thread_id)
        .map(|tid| tid.0.0.to_string());
    let reply_target = ChannelReplyTarget {
        channel_type: ChannelType::Telegram,
        account_id: account_id.to_string(),
        chat_id: chat_id.clone(),
        message_id: None, // Callback queries don't have a message to reply-thread to.
        thread_id: callback_thread_id,
    };
    let outbound_to = reply_target.outbound_to().into_owned();

    // Provider selection → fetch models for that provider and show a new keyboard.
    if let Some(provider_name) = data.strip_prefix("model_provider:") {
        if let Some(ref bot) = bot {
            let _ = bot.answer_callback_query(&query.id).await;
        }
        if let Some(ref sink) = event_sink {
            let cmd = format!("model provider:{provider_name}");
            match sink.dispatch_command(&cmd, reply_target).await {
                Ok(text) => {
                    if let Some(ref b) = bot {
                        send_model_keyboard(b, &outbound_to, &text).await;
                    }
                },
                Err(e) => {
                    if let Err(err) = outbound
                        .send_text(account_id, &outbound_to, &format!("Error: {e}"), None)
                        .await
                    {
                        warn!(account_id, "failed to send callback response: {err}");
                    }
                },
            }
        }
        return Ok(());
    }

    let Some(cmd_text) = cmd_text else {
        return Ok(());
    };

    if let Some(ref sink) = event_sink {
        let response = match sink.dispatch_command(&cmd_text, reply_target).await {
            Ok(msg) => msg,
            Err(e) => format!("Error: {e}"),
        };

        // Answer callback query with the response text (shows as toast).
        if let Some(ref bot) = bot {
            let _ = bot.answer_callback_query(&query.id).text(&response).await;
        }

        // Also send as a regular message for visibility.
        if let Err(e) = outbound
            .send_text(account_id, &outbound_to, &response, None)
            .await
        {
            warn!(account_id, "failed to send callback response: {e}");
        }
    } else if let Some(ref bot) = bot {
        let _ = bot.answer_callback_query(&query.id).await;
    }

    Ok(())
}

/// Extract text content from a message.
fn extract_text(msg: &Message) -> Option<String> {
    match &msg.kind {
        MessageKind::Common(common) => match &common.media_kind {
            MediaKind::Text(t) => Some(t.text.clone()),
            MediaKind::Photo(p) => p.caption.clone(),
            MediaKind::Document(d) => d.caption.clone(),
            MediaKind::Audio(a) => a.caption.clone(),
            MediaKind::Voice(v) => v.caption.clone(),
            MediaKind::Video(vid) => vid.caption.clone(),
            MediaKind::Animation(a) => a.caption.clone(),
            _ => None,
        },
        _ => None,
    }
}

/// Check if the message contains media (photo, document, etc.).
fn has_media(msg: &Message) -> bool {
    match &msg.kind {
        MessageKind::Common(common) => !matches!(common.media_kind, MediaKind::Text(_)),
        _ => false,
    }
}

/// Extract a file ID reference from a message for later download.
#[allow(dead_code)]
fn extract_media_url(msg: &Message) -> Option<String> {
    match &msg.kind {
        MessageKind::Common(common) => match &common.media_kind {
            MediaKind::Photo(p) => p.photo.last().map(|ps| format!("tg://file/{}", ps.file.id)),
            MediaKind::Document(d) => Some(format!("tg://file/{}", d.document.file.id)),
            MediaKind::Audio(a) => Some(format!("tg://file/{}", a.audio.file.id)),
            MediaKind::Voice(v) => Some(format!("tg://file/{}", v.voice.file.id)),
            MediaKind::Sticker(s) => Some(format!("tg://file/{}", s.sticker.file.id)),
            _ => None,
        },
        _ => None,
    }
}

/// Voice/audio file info for transcription.
struct VoiceFileInfo {
    file_id: String,
    /// Format hint: "ogg" for voice messages, "mp3"/"m4a" for audio files
    format: String,
}

/// Extract voice or audio file info from a message.
fn extract_voice_file(msg: &Message) -> Option<VoiceFileInfo> {
    match &msg.kind {
        MessageKind::Common(common) => match &common.media_kind {
            MediaKind::Voice(v) => Some(VoiceFileInfo {
                file_id: v.voice.file.id.clone(),
                format: "ogg".to_string(), // Telegram voice messages are OGG Opus
            }),
            MediaKind::Audio(a) => {
                // Audio files can be various formats, try to detect from mime_type
                let format = a
                    .audio
                    .mime_type
                    .as_ref()
                    .map(|m| {
                        match m.as_ref() {
                            "audio/mpeg" | "audio/mp3" => "mp3",
                            "audio/mp4" | "audio/m4a" | "audio/x-m4a" => "m4a",
                            "audio/ogg" | "audio/opus" => "ogg",
                            "audio/wav" | "audio/x-wav" => "wav",
                            "audio/webm" => "webm",
                            _ => "mp3", // Default fallback
                        }
                    })
                    .unwrap_or("mp3")
                    .to_string();
                Some(VoiceFileInfo {
                    file_id: a.audio.file.id.clone(),
                    format,
                })
            },
            _ => None,
        },
        _ => None,
    }
}

/// Photo file info for vision.
struct PhotoFileInfo {
    file_id: String,
    /// MIME type for the image (e.g., "image/jpeg").
    media_type: String,
}

/// Extract photo file info from a message.
/// Returns the largest photo size for best quality.
fn extract_photo_file(msg: &Message) -> Option<PhotoFileInfo> {
    match &msg.kind {
        MessageKind::Common(common) => match &common.media_kind {
            MediaKind::Photo(p) => {
                // Get the largest photo size (last in the array)
                p.photo.last().map(|ps| PhotoFileInfo {
                    file_id: ps.file.id.clone(),
                    media_type: "image/jpeg".to_string(), // Telegram photos are JPEG
                })
            },
            _ => None,
        },
        _ => None,
    }
}

/// Document/file info for generic file handling.
struct DocumentFileInfo {
    file_id: String,
    /// MIME type for the file (e.g., "text/html", "application/pdf").
    media_type: String,
    /// Optional original filename supplied by Telegram.
    file_name: Option<String>,
}

/// Extract document file info from a message.
/// The returned `media_type` is already normalized (lowercased, parameters stripped).
fn extract_document_file(msg: &Message) -> Option<DocumentFileInfo> {
    match &msg.kind {
        MessageKind::Common(common) => match &common.media_kind {
            MediaKind::Document(d) => {
                let raw = d
                    .document
                    .mime_type
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "application/octet-stream".to_string());
                Some(DocumentFileInfo {
                    file_id: d.document.file.id.clone(),
                    media_type: normalize_media_type(&raw),
                    file_name: d.document.file_name.clone(),
                })
            },
            _ => None,
        },
        _ => None,
    }
}

const MAX_INLINE_DOCUMENT_BYTES: usize = 64 * 1024;
const MAX_INLINE_DOCUMENT_CHARS: usize = 24_000;

fn format_document_label(file_name: Option<&str>, media_type: &str) -> String {
    match file_name {
        Some(name) if !name.trim().is_empty() => format!("[Document: {name} ({media_type})]"),
        _ => format!("[Document: {media_type}]"),
    }
}

/// Normalize a MIME type by stripping parameters (e.g. `; charset=utf-8`) and
/// lower-casing. Returns the base media type only.
fn normalize_media_type(media_type: &str) -> String {
    media_type
        .split(';')
        .next()
        .unwrap_or(media_type)
        .trim()
        .to_ascii_lowercase()
}

/// Check whether a normalized media type is inlineable as text.
/// Expects input from `normalize_media_type` (lowercased, no parameters).
fn should_inline_document_text(media_type: &str) -> bool {
    matches!(
        media_type,
        "text/html"
            | "text/plain"
            | "text/markdown"
            | "text/x-markdown"
            | "text/xml"
            | "application/json"
            | "application/xml"
    ) || media_type.ends_with("+json")
        || media_type.ends_with("+xml")
}

/// Returns `true` for document types we can actually process (text inlining or
/// image attachment). Used to skip downloading unsupported files.
/// Expects input from `normalize_media_type`.
fn is_supported_document_type(media_type: &str) -> bool {
    media_type.starts_with("image/") || should_inline_document_text(media_type)
}

/// Expects a normalized `media_type` (from `normalize_media_type`).
fn extract_text_document_content(data: &[u8], media_type: &str) -> Option<String> {
    if data.is_empty() || !should_inline_document_text(media_type) {
        return None;
    }

    let mut truncated = false;

    // Byte-limit: truncate to the last valid UTF-8 boundary within the cap
    // so we never inject U+FFFD from slicing a multi-byte sequence.
    let bounded = if data.len() > MAX_INLINE_DOCUMENT_BYTES {
        truncated = true;
        let slice = &data[..MAX_INLINE_DOCUMENT_BYTES];
        match std::str::from_utf8(slice) {
            Ok(_) => slice,
            Err(e) => &slice[..e.valid_up_to()],
        }
    } else {
        data
    };

    // Lossy-convert for files with stray invalid bytes in the middle.
    let lossy = String::from_utf8_lossy(bounded);
    let trimmed = lossy.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Char-limit: find the byte offset of the Nth char boundary in one pass.
    let mut text =
        if let Some((byte_idx, _)) = trimmed.char_indices().nth(MAX_INLINE_DOCUMENT_CHARS) {
            truncated = true;
            trimmed[..byte_idx].to_string()
        } else {
            trimmed.to_string()
        };

    if truncated {
        text.push_str("\n\n[Document content truncated]");
    }

    Some(text)
}

/// Extracted location info from a Telegram message.
struct LocationInfo {
    latitude: f64,
    longitude: f64,
    /// Whether this is a live location share (has `live_period` set).
    is_live: bool,
}

/// Extract location coordinates from a message.
fn extract_location(msg: &Message) -> Option<LocationInfo> {
    match &msg.kind {
        MessageKind::Common(common) => match &common.media_kind {
            MediaKind::Location(loc) => Some(LocationInfo {
                latitude: loc.location.latitude,
                longitude: loc.location.longitude,
                is_live: loc.location.live_period.is_some(),
            }),
            _ => None,
        },
        _ => None,
    }
}

/// Describe a media kind for logging purposes.
fn describe_media_kind(msg: &Message) -> Option<&'static str> {
    match &msg.kind {
        MessageKind::Common(common) => match &common.media_kind {
            MediaKind::Text(_) => None,
            MediaKind::Animation(_) => Some("animation/GIF"),
            MediaKind::Audio(_) => Some("audio"),
            MediaKind::Contact(_) => Some("contact"),
            MediaKind::Document(_) => Some("document"),
            MediaKind::Game(_) => Some("game"),
            MediaKind::Location(_) => Some("location"),
            MediaKind::Photo(_) => Some("photo"),
            MediaKind::Poll(_) => Some("poll"),
            MediaKind::Sticker(_) => Some("sticker"),
            MediaKind::Venue(_) => Some("venue"),
            MediaKind::Video(_) => Some("video"),
            MediaKind::VideoNote(_) => Some("video note"),
            MediaKind::Voice(_) => Some("voice"),
            _ => Some("unknown media"),
        },
        _ => None,
    }
}

fn message_kind(msg: &Message) -> Option<ChannelMessageKind> {
    match &msg.kind {
        MessageKind::Common(common) => Some(common.media_kind.to_channel_message_kind()),
        _ => None,
    }
}

trait ToChannelMessageKind {
    fn to_channel_message_kind(&self) -> ChannelMessageKind;
}

impl ToChannelMessageKind for MediaKind {
    fn to_channel_message_kind(&self) -> ChannelMessageKind {
        match self {
            MediaKind::Text(_) => ChannelMessageKind::Text,
            MediaKind::Voice(_) => ChannelMessageKind::Voice,
            MediaKind::Audio(_) => ChannelMessageKind::Audio,
            MediaKind::Photo(_) => ChannelMessageKind::Photo,
            MediaKind::Document(_) => ChannelMessageKind::Document,
            MediaKind::Video(_) | MediaKind::VideoNote(_) => ChannelMessageKind::Video,
            MediaKind::Location(_) => ChannelMessageKind::Location,
            _ => ChannelMessageKind::Other,
        }
    }
}

/// Download a file from Telegram by file ID.
async fn download_telegram_file(bot: &Bot, file_id: &str) -> crate::Result<Vec<u8>> {
    // Get file info from Telegram
    let file = bot.get_file(file_id).await?;

    // Build the download URL from the bot's API base (respects custom/self-hosted endpoints).
    let token = bot.token();
    let base = bot.api_url();
    let url = format!("{base}file/bot{token}/{}", file.path);

    // Download using reqwest
    let response = reqwest::get(&url).await?;
    if !response.status().is_success() {
        return Err(crate::Error::message(format!(
            "failed to download file: HTTP {}",
            response.status()
        )));
    }

    let data = response.bytes().await?.to_vec();
    Ok(data)
}

/// Classify the chat type.
fn classify_chat(msg: &Message) -> (ChatType, Option<String>) {
    match msg.chat.kind {
        teloxide::types::ChatKind::Private(_) => (ChatType::Dm, None),
        teloxide::types::ChatKind::Public(ref p) => {
            let group_id = msg.chat.id.0.to_string();
            match p.kind {
                teloxide::types::PublicChatKind::Channel(_) => (ChatType::Channel, Some(group_id)),
                _ => (ChatType::Group, Some(group_id)),
            }
        },
    }
}

/// Check if the bot was @mentioned in the message.
fn check_bot_mentioned(msg: &Message, bot_username: Option<&str>) -> bool {
    let text = extract_text(msg).unwrap_or_default();
    if let Some(username) = bot_username {
        text.contains(&format!("@{username}"))
    } else {
        false
    }
}

/// Build a session key.
#[allow(dead_code)]
fn build_session_key(
    account_id: &str,
    chat_type: &ChatType,
    peer_id: &str,
    group_id: Option<&str>,
) -> String {
    match chat_type {
        ChatType::Dm => format!("telegram:{account_id}:dm:{peer_id}"),
        ChatType::Group | ChatType::Channel => {
            let gid = group_id.unwrap_or("unknown");
            format!("telegram:{account_id}:group:{gid}")
        },
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {
        super::*,
        std::{
            collections::HashMap,
            sync::{Arc, Mutex},
        },
    };

    use {
        async_trait::async_trait,
        axum::{Json, Router, body::Bytes, extract::State, http::Uri, routing::post},
        moltis_channels::{
            ChannelEvent, ChannelEventSink, ChannelMessageMeta, ChannelReplyTarget,
            Error as ChannelError, Result, gating::DmPolicy,
        },
        secrecy::Secret,
        serde::{Deserialize, Serialize},
        serde_json::json,
        tokio::sync::oneshot,
        tokio_util::sync::CancellationToken,
    };

    use crate::{
        config::TelegramAccountConfig,
        otp::OtpState,
        outbound::TelegramOutbound,
        state::{AccountState, AccountStateMap},
    };

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum TelegramApiMethod {
        SendMessage,
        SendChatAction,
        GetFile,
        Other(String),
    }

    impl TelegramApiMethod {
        fn from_path(path: &str) -> Self {
            let method = path.rsplit('/').next().unwrap_or_default();
            match method {
                "SendMessage" | "sendMessage" => Self::SendMessage,
                "SendChatAction" | "sendChatAction" => Self::SendChatAction,
                "GetFile" | "getFile" => Self::GetFile,
                _ => Self::Other(method.to_string()),
            }
        }
    }

    #[derive(Debug, Clone)]
    enum CapturedTelegramRequest {
        SendMessage(SendMessageRequest),
        SendChatAction(SendChatActionRequest),
        Other {
            method: TelegramApiMethod,
            raw_body: String,
        },
    }

    #[derive(Debug, Clone, Deserialize)]
    struct SendMessageRequest {
        chat_id: i64,
        text: String,
        #[serde(default)]
        parse_mode: Option<String>,
    }

    #[derive(Debug, Clone, Deserialize)]
    struct SendChatActionRequest {
        chat_id: i64,
        action: String,
    }

    #[derive(Debug, Serialize)]
    struct TelegramApiResponse {
        ok: bool,
        result: TelegramApiResult,
    }

    #[derive(Debug, Serialize)]
    #[serde(untagged)]
    enum TelegramApiResult {
        Message(TelegramMessageResult),
        File(TelegramFileResult),
        Bool(bool),
    }

    #[derive(Debug, Serialize)]
    struct TelegramFileResult {
        file_id: String,
        file_unique_id: String,
        file_path: String,
    }

    #[derive(Debug, Serialize)]
    struct TelegramChat {
        id: i64,
        #[serde(rename = "type")]
        chat_type: String,
    }

    #[derive(Debug, Serialize)]
    struct TelegramMessageResult {
        message_id: i64,
        date: i64,
        chat: TelegramChat,
        text: String,
    }

    #[derive(Clone)]
    struct MockTelegramApi {
        requests: Arc<Mutex<Vec<CapturedTelegramRequest>>>,
    }

    async fn telegram_api_handler(
        State(state): State<MockTelegramApi>,
        uri: Uri,
        body: Bytes,
    ) -> Json<TelegramApiResponse> {
        let method = TelegramApiMethod::from_path(uri.path());
        let raw_body = String::from_utf8_lossy(&body).to_string();

        let captured = match method.clone() {
            TelegramApiMethod::SendMessage => {
                match serde_json::from_slice::<SendMessageRequest>(&body) {
                    Ok(req) => CapturedTelegramRequest::SendMessage(req),
                    Err(_) => CapturedTelegramRequest::Other { method, raw_body },
                }
            },
            TelegramApiMethod::SendChatAction => {
                match serde_json::from_slice::<SendChatActionRequest>(&body) {
                    Ok(req) => CapturedTelegramRequest::SendChatAction(req),
                    Err(_) => CapturedTelegramRequest::Other { method, raw_body },
                }
            },
            TelegramApiMethod::GetFile | TelegramApiMethod::Other(_) => {
                CapturedTelegramRequest::Other { method, raw_body }
            },
        };

        state.requests.lock().expect("lock requests").push(captured);

        match TelegramApiMethod::from_path(uri.path()) {
            TelegramApiMethod::SendMessage => Json(TelegramApiResponse {
                ok: true,
                result: TelegramApiResult::Message(TelegramMessageResult {
                    message_id: 1,
                    date: 0,
                    chat: TelegramChat {
                        id: 42,
                        chat_type: "private".to_string(),
                    },
                    text: "ok".to_string(),
                }),
            }),
            TelegramApiMethod::GetFile => Json(TelegramApiResponse {
                ok: true,
                result: TelegramApiResult::File(TelegramFileResult {
                    file_id: "test-file-id".to_string(),
                    file_unique_id: "test-unique-id".to_string(),
                    file_path: "voice/test-voice.ogg".to_string(),
                }),
            }),
            TelegramApiMethod::SendChatAction | TelegramApiMethod::Other(_) => {
                Json(TelegramApiResponse {
                    ok: true,
                    result: TelegramApiResult::Bool(true),
                })
            },
        }
    }

    #[derive(Default)]
    struct MockSink {
        dispatch_calls: std::sync::atomic::AtomicUsize,
        dispatched_texts: Mutex<Vec<String>>,
        dispatched_with_attachments: Mutex<Vec<DispatchedAttachment>>,
        stt_available: bool,
        transcription_result: Mutex<Option<Result<String>>>,
    }

    #[derive(Debug, Clone)]
    struct DispatchedAttachment {
        text: String,
        media_types: Vec<String>,
        sizes: Vec<usize>,
    }

    impl MockSink {
        fn with_stt(transcription: Result<String>) -> Self {
            Self {
                stt_available: true,
                transcription_result: Mutex::new(Some(transcription)),
                ..Default::default()
            }
        }
    }

    #[async_trait]
    impl ChannelEventSink for MockSink {
        async fn emit(&self, _event: ChannelEvent) {}

        async fn dispatch_to_chat(
            &self,
            text: &str,
            _reply_to: ChannelReplyTarget,
            _meta: ChannelMessageMeta,
        ) {
            self.dispatch_calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.dispatched_texts
                .lock()
                .expect("lock")
                .push(text.to_string());
        }

        async fn dispatch_to_chat_with_attachments(
            &self,
            text: &str,
            attachments: Vec<ChannelAttachment>,
            _reply_to: ChannelReplyTarget,
            _meta: ChannelMessageMeta,
        ) {
            let media_types = attachments
                .iter()
                .map(|attachment| attachment.media_type.clone())
                .collect();
            let sizes = attachments
                .iter()
                .map(|attachment| attachment.data.len())
                .collect();
            self.dispatched_with_attachments
                .lock()
                .expect("lock")
                .push(DispatchedAttachment {
                    text: text.to_string(),
                    media_types,
                    sizes,
                });
        }

        async fn dispatch_command(
            &self,
            _command: &str,
            _reply_to: ChannelReplyTarget,
        ) -> Result<String> {
            Ok(String::new())
        }

        async fn request_disable_account(
            &self,
            _channel_type: &str,
            _account_id: &str,
            _reason: &str,
        ) {
        }

        async fn transcribe_voice(&self, _audio_data: &[u8], _format: &str) -> Result<String> {
            self.transcription_result
                .lock()
                .expect("lock")
                .take()
                .unwrap_or_else(|| {
                    Err(ChannelError::unavailable(
                        "transcribe should not be called when STT unavailable",
                    ))
                })
        }

        async fn voice_stt_available(&self) -> bool {
            self.stt_available
        }
    }

    #[test]
    fn session_key_dm() {
        let key = build_session_key("bot1", &ChatType::Dm, "user123", None);
        assert_eq!(key, "telegram:bot1:dm:user123");
    }

    #[test]
    fn session_key_group() {
        let key = build_session_key("bot1", &ChatType::Group, "user123", Some("-100999"));
        assert_eq!(key, "telegram:bot1:group:-100999");
    }

    #[test]
    fn intercepts_shell_mode_control_commands_only() {
        assert!(should_intercept_slash_command("sh", "sh"));
        assert!(should_intercept_slash_command("sh", "sh on"));
        assert!(should_intercept_slash_command("sh", "sh off"));
        assert!(should_intercept_slash_command("sh", "sh exit"));
        assert!(should_intercept_slash_command("sh", "sh status"));
    }

    #[test]
    fn shell_command_payloads_are_not_intercepted() {
        assert!(!should_intercept_slash_command("sh", "sh uname -a"));
        assert!(!should_intercept_slash_command("sh", "sh ls -la"));
    }

    /// Security: the OTP challenge message sent to the Telegram user must
    /// NEVER contain the verification code.  The code should only be visible
    /// to the admin in the web UI.  If this test fails, unauthenticated users
    /// can self-approve without admin involvement.
    #[test]
    fn security_otp_challenge_message_does_not_contain_code() {
        let msg = OTP_CHALLENGE_MSG;

        // Must not contain any 6-digit numeric sequences (OTP codes are 6 digits).
        let has_six_digits = msg
            .as_bytes()
            .windows(6)
            .any(|w| w.iter().all(|b| b.is_ascii_digit()));
        assert!(
            !has_six_digits,
            "OTP challenge message must not contain a 6-digit code: {msg}"
        );

        // Must not contain format placeholders that could interpolate a code.
        assert!(
            !msg.contains("{code}") && !msg.contains("{0}"),
            "OTP challenge message must not contain format placeholders: {msg}"
        );

        // Must contain instructions pointing to the web UI.
        assert!(
            msg.contains("Channels") && msg.contains("Senders"),
            "OTP challenge message must tell the user where to find the code"
        );
    }

    #[test]
    fn voice_messages_are_marked_with_voice_message_kind() {
        let msg: Message = serde_json::from_value(json!({
            "message_id": 1,
            "date": 1,
            "chat": { "id": 42, "type": "private", "first_name": "Alice" },
            "from": {
                "id": 1001,
                "is_bot": false,
                "first_name": "Alice",
                "username": "alice"
            },
            "voice": {
                "file_id": "voice-file-id",
                "file_unique_id": "voice-unique-id",
                "duration": 1,
                "mime_type": "audio/ogg",
                "file_size": 123
            }
        }))
        .expect("deserialize voice message");

        assert!(matches!(
            message_kind(&msg),
            Some(ChannelMessageKind::Voice)
        ));
    }

    #[test]
    fn extract_document_file_from_message() {
        let msg: Message = serde_json::from_value(json!({
            "message_id": 2,
            "date": 1,
            "chat": { "id": 42, "type": "private", "first_name": "Alice" },
            "from": {
                "id": 1001,
                "is_bot": false,
                "first_name": "Alice",
                "username": "alice"
            },
            "caption": "please review",
            "document": {
                "file_id": "doc-file-id",
                "file_unique_id": "doc-unique-id",
                "file_name": "pinned.html",
                "mime_type": "text/html",
                "file_size": 512
            }
        }))
        .expect("deserialize document message");

        let document = extract_document_file(&msg).expect("document should be extracted");
        assert_eq!(document.file_id, "doc-file-id");
        assert_eq!(document.media_type, "text/html");
        assert_eq!(document.file_name.as_deref(), Some("pinned.html"));
    }

    #[test]
    fn extract_document_file_defaults_media_type_when_missing() {
        let msg: Message = serde_json::from_value(json!({
            "message_id": 3,
            "date": 1,
            "chat": { "id": 42, "type": "private", "first_name": "Alice" },
            "from": {
                "id": 1001,
                "is_bot": false,
                "first_name": "Alice",
                "username": "alice"
            },
            "document": {
                "file_id": "doc-file-id",
                "file_unique_id": "doc-unique-id",
                "file_name": "payload.bin",
                "file_size": 128
            }
        }))
        .expect("deserialize document message");

        let document = extract_document_file(&msg).expect("document should be extracted");
        assert_eq!(document.media_type, "application/octet-stream");
    }

    #[test]
    fn should_inline_markdown_document_types() {
        assert!(should_inline_document_text("text/markdown"));
        assert!(should_inline_document_text("text/x-markdown"));
    }

    #[test]
    fn should_inline_text_xml() {
        assert!(should_inline_document_text("text/xml"));
    }

    #[test]
    fn should_inline_after_normalizing_mime_parameters() {
        // should_inline_document_text expects pre-normalized input;
        // verify the full normalize → check pipeline works.
        assert!(should_inline_document_text(&normalize_media_type(
            "text/plain; charset=utf-8"
        )));
        assert!(should_inline_document_text(&normalize_media_type(
            "application/json; charset=utf-8"
        )));
        assert!(should_inline_document_text(&normalize_media_type(
            "text/html; charset=iso-8859-1"
        )));
    }

    #[test]
    fn normalize_media_type_strips_params() {
        assert_eq!(
            normalize_media_type("text/plain; charset=utf-8"),
            "text/plain"
        );
        assert_eq!(normalize_media_type("TEXT/HTML"), "text/html");
        assert_eq!(normalize_media_type("application/json"), "application/json");
    }

    #[test]
    fn is_supported_document_type_checks() {
        assert!(is_supported_document_type("image/png"));
        assert!(is_supported_document_type("text/plain"));
        assert!(is_supported_document_type("application/json"));
        assert!(!is_supported_document_type("application/pdf"));
        assert!(!is_supported_document_type("application/octet-stream"));
    }

    #[test]
    fn extract_text_document_content_utf8_boundary() {
        // 3-byte UTF-8 char: € = [0xE2, 0x82, 0xAC]
        let mut data = vec![b'A'; MAX_INLINE_DOCUMENT_BYTES - 1];
        // Place a 3-byte char straddling the boundary
        data.push(0xE2);
        data.push(0x82);
        data.push(0xAC);
        let result =
            extract_text_document_content(&data, "text/plain").expect("should produce text");
        // Should not end with the replacement character
        assert!(!result.contains('\u{FFFD}'));
    }

    #[test]
    fn extract_text_document_content_cjk_no_replacement_char() {
        // CJK chars are 3 bytes each. Build a buffer that exceeds the byte
        // limit but stays under the char limit (~21K chars < 24K cap), so the
        // char-limit branch is NOT taken. U+FFFD must still not appear.
        // U+4E00 (一) = [0xE4, 0xB8, 0x80]
        let cjk = [0xE4u8, 0xB8, 0x80];
        let char_count = MAX_INLINE_DOCUMENT_BYTES.div_ceil(3); // enough to exceed byte cap
        assert!(
            char_count < MAX_INLINE_DOCUMENT_CHARS,
            "test requires char count below cap"
        );
        let data: Vec<u8> = cjk.iter().copied().cycle().take(char_count * 3).collect();
        assert!(data.len() > MAX_INLINE_DOCUMENT_BYTES);

        let result =
            extract_text_document_content(&data, "text/plain").expect("should produce text");
        assert!(
            !result.contains('\u{FFFD}'),
            "U+FFFD found in CJK truncation result"
        );
        assert!(result.contains("[Document content truncated]"));
    }

    #[tokio::test]
    async fn document_html_is_inlined_into_chat_body() {
        use axum::{http::Method, routing::any};

        async fn combined_handler(
            method: Method,
            State(state): State<MockTelegramApi>,
            uri: Uri,
            body: Bytes,
        ) -> axum::response::Response {
            use axum::response::IntoResponse;
            if method == Method::GET {
                return Bytes::from_static(b"<html><body><h1>Pinned</h1></body></html>")
                    .into_response();
            }
            telegram_api_handler(State(state), uri, body)
                .await
                .into_response()
        }

        let recorded_requests = Arc::new(Mutex::new(Vec::<CapturedTelegramRequest>::new()));
        let mock_api = MockTelegramApi {
            requests: Arc::clone(&recorded_requests),
        };
        let app = Router::new()
            .route("/{*path}", any(combined_handler))
            .with_state(mock_api);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("serve mock telegram api");
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let api_url = reqwest::Url::parse(&format!("http://{addr}/")).expect("parse api url");
        let bot = Bot::new("test-token").set_api_url(api_url);

        let accounts: AccountStateMap = Arc::new(std::sync::RwLock::new(HashMap::new()));
        let outbound = Arc::new(TelegramOutbound {
            accounts: Arc::clone(&accounts),
        });
        let sink = Arc::new(MockSink::default());
        let account_id = "test-account";

        {
            let mut map = accounts.write().expect("accounts write lock");
            map.insert(account_id.to_string(), AccountState {
                bot: bot.clone(),
                bot_username: Some("test_bot".into()),
                account_id: account_id.to_string(),
                config: TelegramAccountConfig {
                    token: Secret::new("test-token".to_string()),
                    dm_policy: DmPolicy::Open,
                    ..Default::default()
                },
                outbound: Arc::clone(&outbound),
                cancel: CancellationToken::new(),
                message_log: None,
                event_sink: Some(Arc::clone(&sink) as Arc<dyn ChannelEventSink>),
                otp: Mutex::new(OtpState::new(300)),
            });
        }

        let msg: Message = serde_json::from_value(json!({
            "message_id": 9,
            "date": 1,
            "chat": { "id": 42, "type": "private", "first_name": "Alice" },
            "from": {
                "id": 1001,
                "is_bot": false,
                "first_name": "Alice",
                "username": "alice"
            },
            "caption": "Please review this",
            "document": {
                "file_id": "doc-file-id",
                "file_unique_id": "doc-unique-id",
                "file_name": "pinned.html",
                "mime_type": "text/html",
                "file_size": 512
            }
        }))
        .expect("deserialize document message");

        handle_message_direct(msg, &bot, account_id, &accounts)
            .await
            .expect("handle message");

        assert_eq!(
            sink.dispatch_calls
                .load(std::sync::atomic::Ordering::Relaxed),
            1,
            "text/html documents should be dispatched as text content"
        );

        {
            let texts = sink.dispatched_texts.lock().expect("lock");
            assert_eq!(texts.len(), 1);
            assert!(texts[0].contains("Please review this"));
            assert!(texts[0].contains("[Document: pinned.html (text/html)]"));
            assert!(texts[0].contains("<h1>Pinned</h1>"));
        }

        {
            let attachments = sink.dispatched_with_attachments.lock().expect("lock");
            assert!(
                attachments.is_empty(),
                "text/html documents should not be sent as image attachments"
            );
        }

        let _ = shutdown_tx.send(());
        server.await.expect("server join");
    }

    #[tokio::test]
    async fn document_image_is_dispatched_as_attachment() {
        use axum::{http::Method, routing::any};

        async fn combined_handler(
            method: Method,
            State(state): State<MockTelegramApi>,
            uri: Uri,
            body: Bytes,
        ) -> axum::response::Response {
            use axum::response::IntoResponse;
            if method == Method::GET {
                return Bytes::from_static(b"fake-png-data").into_response();
            }
            telegram_api_handler(State(state), uri, body)
                .await
                .into_response()
        }

        let recorded_requests = Arc::new(Mutex::new(Vec::<CapturedTelegramRequest>::new()));
        let mock_api = MockTelegramApi {
            requests: Arc::clone(&recorded_requests),
        };
        let app = Router::new()
            .route("/{*path}", any(combined_handler))
            .with_state(mock_api);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("serve mock telegram api");
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let api_url = reqwest::Url::parse(&format!("http://{addr}/")).expect("parse api url");
        let bot = Bot::new("test-token").set_api_url(api_url);

        let accounts: AccountStateMap = Arc::new(std::sync::RwLock::new(HashMap::new()));
        let outbound = Arc::new(TelegramOutbound {
            accounts: Arc::clone(&accounts),
        });
        let sink = Arc::new(MockSink::default());
        let account_id = "test-account";

        {
            let mut map = accounts.write().expect("accounts write lock");
            map.insert(account_id.to_string(), AccountState {
                bot: bot.clone(),
                bot_username: Some("test_bot".into()),
                account_id: account_id.to_string(),
                config: TelegramAccountConfig {
                    token: Secret::new("test-token".to_string()),
                    dm_policy: DmPolicy::Open,
                    ..Default::default()
                },
                outbound: Arc::clone(&outbound),
                cancel: CancellationToken::new(),
                message_log: None,
                event_sink: Some(Arc::clone(&sink) as Arc<dyn ChannelEventSink>),
                otp: Mutex::new(OtpState::new(300)),
            });
        }

        let msg: Message = serde_json::from_value(json!({
            "message_id": 10,
            "date": 1,
            "chat": { "id": 42, "type": "private", "first_name": "Alice" },
            "from": {
                "id": 1001,
                "is_bot": false,
                "first_name": "Alice",
                "username": "alice"
            },
            "caption": "What is in this image?",
            "document": {
                "file_id": "doc-image-file-id",
                "file_unique_id": "doc-image-unique-id",
                "file_name": "screenshot.png",
                "mime_type": "image/png",
                "file_size": 512
            }
        }))
        .expect("deserialize document message");

        handle_message_direct(msg, &bot, account_id, &accounts)
            .await
            .expect("handle message");

        assert_eq!(
            sink.dispatch_calls
                .load(std::sync::atomic::Ordering::Relaxed),
            0,
            "image documents should dispatch through attachment pathway"
        );

        {
            let attachments = sink.dispatched_with_attachments.lock().expect("lock");
            assert_eq!(attachments.len(), 1);
            assert_eq!(attachments[0].text, "What is in this image?");
            assert_eq!(attachments[0].media_types, vec!["image/png".to_string()]);
            assert_eq!(attachments[0].sizes, vec![13]);
        }

        let _ = shutdown_tx.send(());
        server.await.expect("server join");
    }

    #[tokio::test]
    async fn voice_not_configured_replies_with_setup_hint_and_skips_dispatch() {
        let recorded_requests = Arc::new(Mutex::new(Vec::<CapturedTelegramRequest>::new()));
        let mock_api = MockTelegramApi {
            requests: Arc::clone(&recorded_requests),
        };
        let app = Router::new()
            .route("/{*path}", post(telegram_api_handler))
            .with_state(mock_api);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("local addr");
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("serve mock telegram api");
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let api_url = reqwest::Url::parse(&format!("http://{addr}/")).expect("parse api url");
        let bot = Bot::new("test-token").set_api_url(api_url);

        let accounts: AccountStateMap = Arc::new(std::sync::RwLock::new(HashMap::new()));
        let outbound = Arc::new(TelegramOutbound {
            accounts: Arc::clone(&accounts),
        });
        let sink = Arc::new(MockSink::default());
        let account_id = "test-account";

        {
            let mut map = accounts.write().expect("accounts write lock");
            map.insert(account_id.to_string(), AccountState {
                bot: bot.clone(),
                bot_username: Some("test_bot".into()),
                account_id: account_id.to_string(),
                config: TelegramAccountConfig {
                    token: Secret::new("test-token".to_string()),
                    dm_policy: DmPolicy::Open,
                    ..Default::default()
                },
                outbound: Arc::clone(&outbound),
                cancel: CancellationToken::new(),
                message_log: None,
                event_sink: Some(Arc::clone(&sink) as Arc<dyn ChannelEventSink>),
                otp: Mutex::new(OtpState::new(300)),
            });
        }

        let msg: Message = serde_json::from_value(json!({
            "message_id": 1,
            "date": 1,
            "chat": { "id": 42, "type": "private", "first_name": "Alice" },
            "from": {
                "id": 1001,
                "is_bot": false,
                "first_name": "Alice",
                "username": "alice"
            },
            "voice": {
                "file_id": "voice-file-id",
                "file_unique_id": "voice-unique-id",
                "duration": 1,
                "mime_type": "audio/ogg",
                "file_size": 123
            }
        }))
        .expect("deserialize voice message");
        assert!(
            extract_voice_file(&msg).is_some(),
            "message should contain voice media"
        );

        handle_message_direct(msg, &bot, account_id, &accounts)
            .await
            .expect("handle message");

        {
            let requests = recorded_requests.lock().expect("requests lock");
            assert!(
                requests.iter().any(|request| {
                    if let CapturedTelegramRequest::SendMessage(body) = request {
                        body.chat_id == 42
                            && body.parse_mode.as_deref() == Some("HTML")
                            && body
                                .text
                                .contains("I can't understand voice, you did not configure it")
                    } else {
                        false
                    }
                }),
                "expected voice setup hint to be sent, requests={requests:?}"
            );
            assert!(
                requests.iter().any(|request| {
                    if let CapturedTelegramRequest::SendChatAction(action) = request {
                        action.chat_id == 42 && action.action == "typing"
                    } else {
                        false
                    }
                }),
                "expected typing action before reply, requests={requests:?}"
            );
            assert!(
                requests.iter().all(|request| {
                    if let CapturedTelegramRequest::Other { method, raw_body } = request {
                        !matches!(
                            method,
                            TelegramApiMethod::SendMessage | TelegramApiMethod::SendChatAction
                        ) || raw_body.is_empty()
                    } else {
                        true
                    }
                }),
                "unexpected untyped request capture for known method, requests={requests:?}"
            );
        }
        assert_eq!(
            sink.dispatch_calls
                .load(std::sync::atomic::Ordering::Relaxed),
            0,
            "voice message should not be dispatched to chat when STT is unavailable"
        );

        let _ = shutdown_tx.send(());
        server.await.expect("server join");
    }

    /// Regression test: when STT is available but transcription returns an empty
    /// string (e.g. noisy environment), the voice message must still be
    /// dispatched to chat and the audio saved, rather than silently dropped.
    #[tokio::test]
    async fn voice_empty_transcription_still_dispatches_to_chat() {
        use axum::{http::Method, routing::any};

        async fn combined_handler(
            method: Method,
            State(state): State<MockTelegramApi>,
            uri: Uri,
            body: Bytes,
        ) -> axum::response::Response {
            use axum::response::IntoResponse;
            if method == Method::GET {
                // File download endpoint — return dummy audio bytes.
                return Bytes::from_static(b"fake-ogg-audio-data").into_response();
            }
            // Delegate POST to the normal mock Telegram API handler.
            let resp = telegram_api_handler(State(state), uri, body).await;
            resp.into_response()
        }

        let recorded_requests = Arc::new(Mutex::new(Vec::<CapturedTelegramRequest>::new()));
        let mock_api = MockTelegramApi {
            requests: Arc::clone(&recorded_requests),
        };
        let app = Router::new()
            .route("/{*path}", any(combined_handler))
            .with_state(mock_api);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("serve mock telegram api");
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let api_url = reqwest::Url::parse(&format!("http://{addr}/")).expect("parse api url");
        let bot = Bot::new("test-token").set_api_url(api_url);

        let accounts: AccountStateMap = Arc::new(std::sync::RwLock::new(HashMap::new()));
        let outbound = Arc::new(TelegramOutbound {
            accounts: Arc::clone(&accounts),
        });
        // STT available, transcription returns empty string.
        let sink = Arc::new(MockSink::with_stt(Ok(String::new())));
        let account_id = "test-account";

        {
            let mut map = accounts.write().expect("accounts write lock");
            map.insert(account_id.to_string(), AccountState {
                bot: bot.clone(),
                bot_username: Some("test_bot".into()),
                account_id: account_id.to_string(),
                config: TelegramAccountConfig {
                    token: Secret::new("test-token".to_string()),
                    dm_policy: DmPolicy::Open,
                    ..Default::default()
                },
                outbound: Arc::clone(&outbound),
                cancel: CancellationToken::new(),
                message_log: None,
                event_sink: Some(Arc::clone(&sink) as Arc<dyn ChannelEventSink>),
                otp: Mutex::new(OtpState::new(300)),
            });
        }

        let msg: Message = serde_json::from_value(json!({
            "message_id": 1,
            "date": 1,
            "chat": { "id": 42, "type": "private", "first_name": "Alice" },
            "from": {
                "id": 1001,
                "is_bot": false,
                "first_name": "Alice",
                "username": "alice"
            },
            "voice": {
                "file_id": "voice-file-id",
                "file_unique_id": "voice-unique-id",
                "duration": 1,
                "mime_type": "audio/ogg",
                "file_size": 123
            }
        }))
        .expect("deserialize voice message");

        handle_message_direct(msg, &bot, account_id, &accounts)
            .await
            .expect("handle message");

        assert_eq!(
            sink.dispatch_calls
                .load(std::sync::atomic::Ordering::Relaxed),
            1,
            "voice message with empty transcription must still be dispatched to chat"
        );

        {
            let texts = sink.dispatched_texts.lock().expect("lock");
            assert!(
                texts[0].contains("could not transcribe"),
                "dispatched text should indicate transcription was empty, got: {}",
                texts[0]
            );
        }

        let _ = shutdown_tx.send(());
        server.await.expect("server join");
    }

    #[test]
    fn extract_location_from_message() {
        let msg: Message = serde_json::from_value(json!({
            "message_id": 1,
            "date": 1,
            "chat": { "id": 42, "type": "private", "first_name": "Alice" },
            "from": {
                "id": 1001,
                "is_bot": false,
                "first_name": "Alice",
                "username": "alice"
            },
            "location": {
                "latitude": 48.8566,
                "longitude": 2.3522
            }
        }))
        .expect("deserialize location message");

        let loc = extract_location(&msg);
        assert!(loc.is_some(), "should extract location from message");
        let info = loc.unwrap();
        assert!((info.latitude - 48.8566).abs() < 1e-4);
        assert!((info.longitude - 2.3522).abs() < 1e-4);
        assert!(!info.is_live, "static location should not be live");
    }

    #[test]
    fn extract_location_returns_none_for_text() {
        let msg: Message = serde_json::from_value(json!({
            "message_id": 1,
            "date": 1,
            "chat": { "id": 42, "type": "private", "first_name": "Alice" },
            "from": {
                "id": 1001,
                "is_bot": false,
                "first_name": "Alice"
            },
            "text": "hello"
        }))
        .expect("deserialize text message");

        assert!(extract_location(&msg).is_none());
    }

    #[test]
    fn location_messages_are_marked_with_location_message_kind() {
        let msg: Message = serde_json::from_value(json!({
            "message_id": 1,
            "date": 1,
            "chat": { "id": 42, "type": "private", "first_name": "Alice" },
            "from": {
                "id": 1001,
                "is_bot": false,
                "first_name": "Alice"
            },
            "location": {
                "latitude": 48.8566,
                "longitude": 2.3522
            }
        }))
        .expect("deserialize location message");

        assert!(matches!(
            message_kind(&msg),
            Some(ChannelMessageKind::Location)
        ));
    }

    #[test]
    fn extract_location_detects_live_period() {
        let msg: Message = serde_json::from_value(json!({
            "message_id": 1,
            "date": 1,
            "chat": { "id": 42, "type": "private", "first_name": "Alice" },
            "from": {
                "id": 1001,
                "is_bot": false,
                "first_name": "Alice"
            },
            "location": {
                "latitude": 48.8566,
                "longitude": 2.3522,
                "live_period": 3600
            }
        }))
        .expect("deserialize live location message");

        let info = extract_location(&msg).expect("should extract live location");
        assert!(info.is_live, "location with live_period should be live");
        assert!((info.latitude - 48.8566).abs() < 1e-4);
    }
}
