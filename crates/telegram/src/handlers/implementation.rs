use std::sync::Arc;

use {
    teloxide::{
        prelude::*,
        types::{CallbackQuery, ThreadId},
    },
    tracing::{debug, info, warn},
};

use {
    moltis_channels::{
        ChannelAttachment, ChannelDocumentFile, ChannelEvent, ChannelMessageMeta, ChannelOutbound,
        ChannelReplyTarget, ChannelType, config_view::ChannelConfigView,
        message_log::MessageLogEntry,
    },
    moltis_common::types::ChatType,
};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, histogram, telegram as tg_metrics};

use crate::{
    access::{self, AccessDenied},
    state::AccountStateMap,
};

mod callbacks;
mod media;
mod otp;

use self::{callbacks::*, media::*, otp::handle_otp_flow};

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

fn reply_target_for_msg(account_id: &str, msg: &Message) -> ChannelReplyTarget {
    ChannelReplyTarget {
        channel_type: ChannelType::Telegram,
        account_id: account_id.to_string(),
        chat_id: msg.chat.id.0.to_string(),
        message_id: Some(msg.id.0.to_string()),
        thread_id: extract_thread_id(msg),
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
    //
    // If voice processing fails in any way (STT unconfigured, download error,
    // transcription error, empty transcription), `handle_voice_message` sends
    // a direct user-facing reply and returns `None`, so we bail out here
    // without dispatching a placeholder string to the LLM (see issue #632).
    let (body, attachments, voice_audio, documents): (
        String,
        Vec<ChannelAttachment>,
        Option<(Vec<u8>, String)>,
        Option<Vec<ChannelDocumentFile>>,
    ) = if let Some(voice_file) = extract_voice_file(&msg) {
        match handle_voice_message(
            bot,
            &msg,
            account_id,
            text.as_deref(),
            event_sink.as_ref(),
            outbound.as_ref(),
            &voice_file,
        )
        .await
        {
            Some(triple) => (triple.0, triple.1, triple.2, None),
            None => return Ok(()),
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
                (caption, vec![attachment], None, None)
            },
            Err(e) => {
                warn!(account_id, error = %e, "failed to download photo");
                (
                    text.clone()
                        .unwrap_or_else(|| "[Photo - download failed]".to_string()),
                    Vec::new(),
                    None,
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
            (body, Vec::new(), None, None)
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
                    let reply_target = reply_target_for_msg(account_id, &msg);
                    let saved_document = save_inbound_document(
                        event_sink.as_ref(),
                        &reply_target,
                        document_file.file_name.as_deref(),
                        &document_file.media_type,
                        &document_file.file_id,
                        &document_data,
                    )
                    .await;
                    let document_files = saved_document.as_ref().map(|saved| {
                        vec![channel_document_file(
                            saved,
                            document_file.file_name.as_deref(),
                            &document_file.media_type,
                        )]
                    });

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
                        (
                            build_document_body(&caption, &doc_label, None),
                            vec![attachment],
                            None,
                            document_files,
                        )
                    } else if is_pdf_document_type(&document_file.media_type) {
                        if let Some(extracted_text) =
                            extract_pdf_document_content(document_data).await
                        {
                            let body =
                                build_document_body(&caption, &doc_label, Some(&extracted_text));
                            (body, Vec::new(), None, document_files)
                        } else {
                            let body = build_document_body(&caption, &doc_label, None);
                            (body, Vec::new(), None, document_files)
                        }
                    } else if let Some(extracted_text) =
                        extract_text_document_content(&document_data, &document_file.media_type)
                    {
                        let body = build_document_body(&caption, &doc_label, Some(&extracted_text));
                        (body, Vec::new(), None, document_files)
                    } else {
                        let body = build_document_body(&caption, &doc_label, None);
                        (body, Vec::new(), None, document_files)
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

                    (body, Vec::new(), None, None)
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
        (text.unwrap_or_default(), Vec::new(), None, None)
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
        let reply_target = reply_target_for_msg(account_id, &msg);

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
                    let context_result = sink
                        .dispatch_command("context", reply_target.clone(), Some(&peer_id))
                        .await;
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
                    let list_result = sink
                        .dispatch_command("agent", reply_target.clone(), Some(&peer_id))
                        .await;
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
                    let list_result = sink
                        .dispatch_command("model", reply_target.clone(), Some(&peer_id))
                        .await;
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
                    let list_result = sink
                        .dispatch_command("sandbox", reply_target.clone(), Some(&peer_id))
                        .await;
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
                        .dispatch_command("sessions", reply_target.clone(), Some(&peer_id))
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
                    "Available commands:\n/new — Start a new session\n/sessions — List and switch this chat's sessions\n/attach — Attach an existing session to this chat\n/approvals — List pending exec approvals for this session\n/approve N — Approve a pending exec request\n/deny N — Deny a pending exec request\n/agent — Switch session agent\n/model — Switch provider/model\n/sandbox — Toggle sandbox and choose image\n/sh — Enable command mode (/sh off to exit)\n/clear — Clear session history\n/compact — Compact session (summarize)\n/context — Show session context info\n/help — Show this help".to_string()
                } else {
                    match sink
                        .dispatch_command(cmd_text, reply_target.clone(), Some(&peer_id))
                        .await
                    {
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
            sender_id: Some(peer_id.clone()),
            message_kind: message_kind(&msg),
            model: config
                .resolve_model(&msg.chat.id.0.to_string(), &peer_id)
                .map(String::from),
            agent_id: config
                .resolve_agent_id(&msg.chat.id.0.to_string(), &peer_id)
                .map(String::from),
            audio_filename,
            documents,
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
        "new" | "clear" | "compact" | "context" | "model" | "sandbox" | "sessions" | "attach"
        | "approvals" | "approve" | "deny" | "agent" | "help" => true,
        "sh" => {
            let args = cmd_text.strip_prefix(cmd).unwrap_or("").trim();
            args.is_empty() || matches!(args, "on" | "off" | "exit" | "status")
        },
        _ => false,
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
    let sender_id = query.from.id.0.to_string();
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
            match sink
                .dispatch_command(&cmd, reply_target, Some(&sender_id))
                .await
            {
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
        let response = match sink
            .dispatch_command(&cmd_text, reply_target, Some(&sender_id))
            .await
        {
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

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;
