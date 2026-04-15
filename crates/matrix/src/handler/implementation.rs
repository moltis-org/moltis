use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use {
    matrix_sdk::{
        Room,
        encryption::VerificationState,
        media::{MediaFormat, MediaRequestParameters},
        ruma::{
            OwnedUserId,
            events::room::{
                encrypted::OriginalSyncRoomEncryptedEvent,
                member::StrippedRoomMemberEvent,
                message::{
                    AudioMessageEventContent, LocationMessageEventContent, MessageType,
                    OriginalSyncRoomMessageEvent,
                },
            },
        },
    },
    tracing::{debug, info, warn},
};

use {
    moltis_channels::{
        ChannelEvent, ChannelType,
        config_view::ChannelConfigView,
        gating::{self, DmPolicy, GroupPolicy},
        message_log::{MessageLog, MessageLogEntry},
        otp::{
            OtpInitResult, OtpVerifyResult, approve_sender_via_otp, emit_otp_challenge,
            emit_otp_resolution,
        },
        plugin::{ChannelEventSink, ChannelMessageKind, ChannelMessageMeta, ChannelReplyTarget},
    },
    moltis_common::types::ChatType,
    time::OffsetDateTime,
};

use crate::{
    access,
    config::{AutoJoinPolicy, MatrixAccountConfig},
    state::AccountStateMap,
    verification,
};

const UTD_NOTICE_COOLDOWN: Duration = Duration::from_secs(300);

const HELP_TEXT: &str = "Available commands:\n\
    /new — Start a new session\n\
    /sessions — List and switch sessions\n\
    /agent — Switch session agent\n\
    /model — Switch provider/model\n\
    /sandbox — Toggle sandbox and choose image\n\
    /sh — Enable command mode (/sh off to exit)\n\
    /clear — Clear session history\n\
    /compact — Compact session (summarize)\n\
    /context — Show session context info\n\
    /peek — Show current thinking/tool status\n\
    /stop — Abort the current running agent\n\
    /help — Show this help";

fn should_ignore_initial_sync_history(accounts: &AccountStateMap, account_id: &str) -> bool {
    let guard = accounts.read().unwrap_or_else(|error| error.into_inner());
    guard
        .get(account_id)
        .is_some_and(|state| !state.initial_sync_complete())
}

#[tracing::instrument(skip(ev, room, accounts, bot_user_id), fields(account_id, room = %room.room_id()))]
pub async fn handle_room_message(
    ev: OriginalSyncRoomMessageEvent,
    room: Room,
    account_id: String,
    accounts: AccountStateMap,
    bot_user_id: OwnedUserId,
) {
    if ev.sender == bot_user_id {
        return;
    }
    if should_ignore_initial_sync_history(&accounts, &account_id) {
        debug!(
            account_id,
            "ignoring Matrix history during initial sync catch-up"
        );
        return;
    }

    let room_id = room.room_id().to_string();
    let sender_id = ev.sender.to_string();
    let event_id = ev.event_id.to_string();

    let Some(kind) = inbound_message_kind(&ev.content.msgtype) else {
        return;
    };
    let body = inbound_message_body(&ev.content.msgtype);

    if body.is_empty() && matches!(kind, ChannelMessageKind::Text) {
        return;
    }

    record_message_received();

    // Snapshot config+state without holding lock across .await
    let (config, message_log, event_sink) = {
        let guard = accounts.read().unwrap_or_else(|e| e.into_inner());
        match guard.get(&account_id) {
            Some(s) => (
                s.config.clone(),
                s.message_log.clone(),
                s.event_sink.clone(),
            ),
            None => {
                warn!(account_id, "account state not found");
                return;
            },
        }
    };

    let direct_flag = match room.is_direct().await {
        Ok(is_direct) => is_direct,
        Err(error) => {
            warn!(
                account_id,
                room = %room_id,
                "failed to determine Matrix DM state, falling back to member-count heuristic: {error}"
            );
            false
        },
    };
    let active_members = room.active_members_count();
    let joined_members = room.joined_members_count();
    let chat_type = infer_chat_type(direct_flag, active_members, joined_members);

    debug!(
        account_id,
        room = %room_id,
        sender = %sender_id,
        direct_flag,
        active_members,
        joined_members,
        chat_type = ?chat_type,
        "matrix inbound message"
    );

    let bot_mentioned = is_bot_mentioned(&ev, &bot_user_id, &body);

    if matches!(ev.content.msgtype, MessageType::VerificationRequest(_)) {
        verification::handle_room_verification_request(
            room.clone(),
            account_id.clone(),
            sender_id.clone(),
            event_id.clone(),
            Arc::clone(&accounts),
        )
        .await;
        return;
    }

    if matches!(kind, ChannelMessageKind::Text)
        && verification::maybe_handle_confirmation_message(
            &body,
            &room,
            &account_id,
            &sender_id,
            &accounts,
        )
        .await
    {
        return;
    }

    let sender_name = room
        .get_member_no_sync(&ev.sender)
        .await
        .ok()
        .flatten()
        .and_then(|m| m.display_name().map(|s| s.to_string()));
    let chat_type_str = if matches!(chat_type, ChatType::Dm) {
        "dm"
    } else {
        "group"
    };

    if let Err(reason) = checked_chat_type(
        &config,
        &sender_id,
        &room_id,
        direct_flag,
        active_members,
        joined_members,
        bot_mentioned,
    ) {
        if matches!(chat_type, ChatType::Dm)
            && matches!(reason, access::AccessDenied::NotOnAllowlist)
            && config.otp_self_approval
            && config.dm_policy == DmPolicy::Allowlist
        {
            log_inbound_message(&message_log, MessageLogEntry {
                id: 0,
                account_id: account_id.clone(),
                channel_type: "matrix".into(),
                peer_id: sender_id.clone(),
                username: Some(sender_id.clone()),
                sender_name: sender_name.clone(),
                chat_id: room_id.clone(),
                chat_type: chat_type_str.into(),
                body: body.clone(),
                access_granted: false,
                created_at: unix_now(),
            })
            .await;
            emit_inbound_message_event(
                &event_sink,
                &account_id,
                &sender_id,
                sender_name.clone(),
                false,
            )
            .await;
            info!(
                account_id,
                room = %room_id,
                sender = %sender_id,
                chat_type = ?chat_type,
                "matrix inbound entering OTP self-approval flow"
            );
            handle_otp(
                &body,
                &sender_id,
                &account_id,
                &accounts,
                &event_sink,
                &room,
            )
            .await;
            return;
        }
        info!(
            account_id,
            room = %room_id,
            sender = %sender_id,
            chat_type = ?chat_type,
            %reason,
            "matrix inbound access denied"
        );
        log_inbound_message(&message_log, MessageLogEntry {
            id: 0,
            account_id: account_id.clone(),
            channel_type: "matrix".into(),
            peer_id: sender_id.clone(),
            username: Some(sender_id.clone()),
            sender_name: sender_name.clone(),
            chat_id: room_id.clone(),
            chat_type: chat_type_str.into(),
            body: body.clone(),
            access_granted: false,
            created_at: unix_now(),
        })
        .await;
        emit_inbound_message_event(
            &event_sink,
            &account_id,
            &sender_id,
            sender_name.clone(),
            false,
        )
        .await;
        return;
    }

    if let Some(emoji) = &config.ack_reaction {
        let room_clone = room.clone();
        let event_id_clone = ev.event_id.clone();
        let emoji_clone = emoji.clone();
        tokio::spawn(async move {
            use matrix_sdk::ruma::events::{reaction::ReactionEventContent, relation::Annotation};
            let annotation = Annotation::new(event_id_clone, emoji_clone);
            let content = ReactionEventContent::new(annotation);
            if let Err(e) = room_clone.send(content).await {
                warn!("failed to send ack reaction: {e}");
            }
        });
    }

    log_inbound_message(&message_log, MessageLogEntry {
        id: 0,
        account_id: account_id.clone(),
        channel_type: "matrix".into(),
        peer_id: sender_id.clone(),
        username: Some(sender_id.clone()),
        sender_name: sender_name.clone(),
        chat_id: room_id.clone(),
        chat_type: chat_type_str.into(),
        body: body.clone(),
        access_granted: true,
        created_at: unix_now(),
    })
    .await;

    emit_inbound_message_event(
        &event_sink,
        &account_id,
        &sender_id,
        sender_name.clone(),
        true,
    )
    .await;

    let reply_to = ChannelReplyTarget {
        channel_type: ChannelType::Matrix,
        account_id: account_id.clone(),
        chat_id: room_id.clone(),
        thread_id: None,
        message_id: if config.reply_to_message {
            Some(event_id.clone())
        } else {
            None
        },
    };

    let meta = ChannelMessageMeta {
        channel_type: ChannelType::Matrix,
        sender_name: sender_name.clone(),
        username: Some(sender_id.clone()),
        sender_id: Some(sender_id.clone()),
        message_kind: Some(kind),
        model: config.resolve_model(&room_id, &sender_id).map(String::from),
        agent_id: config
            .resolve_agent_id(&room_id, &sender_id)
            .map(String::from),
        audio_filename: None,
        documents: None,
    };

    if let Some(sink) = &event_sink {
        match &ev.content.msgtype {
            MessageType::Audio(audio) => {
                handle_audio_message(
                    audio,
                    &room,
                    &account_id,
                    &event_id,
                    sink.as_ref(),
                    reply_to,
                    meta,
                )
                .await;
                return;
            },
            MessageType::Location(location) => {
                handle_location_message(
                    location,
                    &room,
                    &account_id,
                    &event_id,
                    sink.as_ref(),
                    reply_to,
                    meta,
                )
                .await;
                return;
            },
            _ => {},
        }

        if matches!(kind, ChannelMessageKind::Text)
            && let Some((latitude, longitude)) = extract_location_coordinates(&body)
        {
            let resolved = sink
                .resolve_pending_location(&reply_to, latitude, longitude)
                .await;
            if resolved {
                info!(
                    account_id,
                    room = %room_id,
                    sender = %sender_id,
                    latitude,
                    longitude,
                    "matrix location text resolved pending request"
                );
                if let Err(error) = send_text(&room, "Location updated.").await {
                    warn!(
                        account_id,
                        room = %room_id,
                        "failed to send Matrix location confirmation: {error}"
                    );
                }
                return;
            }
        }

        // Intercept slash commands before dispatching to LLM.
        if matches!(kind, ChannelMessageKind::Text)
            && let Some(cmd_text) = body.strip_prefix('/')
        {
            let cmd_name = cmd_text.split_whitespace().next().unwrap_or("");
            let response = if cmd_name == "help" {
                Ok(HELP_TEXT.to_string())
            } else {
                sink.dispatch_command(cmd_text, reply_to, Some(&sender_id))
                    .await
            };
            let text = match response {
                Ok(msg) => msg,
                Err(e) => format!("Error: {e}"),
            };
            if let Err(error) = send_text(&room, &text).await {
                warn!(
                    account_id,
                    room = %room_id,
                    "failed to send Matrix command response: {error}"
                );
            }
            return;
        }

        sink.dispatch_to_chat(&body, reply_to, meta).await;
    }
}

#[tracing::instrument(skip(ev, room, accounts, bot_user_id), fields(account_id, room = %room.room_id(), event_id = %ev.event_id))]
pub async fn handle_room_encrypted_event(
    ev: OriginalSyncRoomEncryptedEvent,
    room: Room,
    account_id: String,
    accounts: AccountStateMap,
    bot_user_id: OwnedUserId,
) {
    if ev.sender == bot_user_id {
        return;
    }
    if should_ignore_initial_sync_history(&accounts, &account_id) {
        debug!(
            account_id,
            room = %room.room_id(),
            "ignoring Matrix encrypted history during initial sync catch-up"
        );
        return;
    }

    let room_id = room.room_id().to_string();
    let sender_id = ev.sender.to_string();
    let verification_state = room.client().encryption().verification_state().get();

    warn!(
        account_id,
        room = %room_id,
        sender = %sender_id,
        ?verification_state,
        "matrix encrypted event could not be decrypted yet"
    );

    let should_notify = {
        let guard = accounts.read().unwrap_or_else(|error| error.into_inner());
        let Some(state) = guard.get(&account_id) else {
            return;
        };
        let mut verification = state
            .verification
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        update_utd_notice_window(
            &mut verification.recent_utd_notice_by_room,
            &room_id,
            Instant::now(),
        )
    };

    if !should_notify {
        return;
    }

    if let Err(error) = send_text(&room, utd_notice_message(verification_state)).await {
        warn!(
            account_id,
            room = %room_id,
            "failed to send Matrix undecryptable-event notice: {error}"
        );
    }
}

pub async fn handle_poll_response(
    room: Room,
    account_id: String,
    accounts: AccountStateMap,
    sender_id: String,
    callback_data: Option<String>,
) {
    let Some(callback_data) = callback_data else {
        return;
    };
    if should_ignore_initial_sync_history(&accounts, &account_id) {
        debug!(
            account_id,
            "ignoring Matrix poll response during initial sync catch-up"
        );
        return;
    }

    let room_id = room.room_id().to_string();
    let (config, event_sink, bot_user_id) = {
        let guard = accounts.read().unwrap_or_else(|e| e.into_inner());
        let Some(state) = guard.get(&account_id) else {
            warn!(account_id, "account state not found");
            return;
        };

        (
            state.config.clone(),
            state.event_sink.clone(),
            state.bot_user_id.clone(),
        )
    };

    if sender_id == bot_user_id {
        return;
    }

    let direct_flag = match room.is_direct().await {
        Ok(is_direct) => is_direct,
        Err(error) => {
            warn!(
                account_id,
                room = %room_id,
                "failed to determine Matrix DM state for poll response, falling back to member-count heuristic: {error}"
            );
            false
        },
    };
    let active_members = room.active_members_count();
    let joined_members = room.joined_members_count();
    let chat_type = match checked_chat_type(
        &config,
        &sender_id,
        &room_id,
        direct_flag,
        active_members,
        joined_members,
        true,
    ) {
        Ok(chat_type) => chat_type,
        Err(reason) => {
            info!(
                account_id,
                room = %room_id,
                sender = %sender_id,
                direct_flag,
                active_members,
                joined_members,
                %reason,
                "matrix poll response access denied"
            );
            return;
        },
    };

    info!(
        account_id,
        room = %room_id,
        sender = %sender_id,
        direct_flag,
        active_members,
        joined_members,
        chat_type = ?chat_type,
        "matrix poll response"
    );

    let Some(sink) = event_sink else {
        return;
    };

    record_message_received();

    let reply_to = ChannelReplyTarget {
        channel_type: ChannelType::Matrix,
        account_id: account_id.clone(),
        chat_id: room_id,
        thread_id: None,
        message_id: None,
    };

    if let Err(error) = sink.dispatch_interaction(&callback_data, reply_to).await {
        debug!(
            account_id,
            callback_data, "matrix poll interaction dispatch failed: {error}"
        );
    }
}

fn should_auto_join_invite(
    config: &MatrixAccountConfig,
    inviter_id: &str,
    room_id: &str,
    is_direct: bool,
) -> bool {
    if is_direct {
        // DM invites are gated by dm_policy, not room_policy.
        let dm_allowed = match config.dm_policy {
            DmPolicy::Disabled => false,
            DmPolicy::Open => true,
            DmPolicy::Allowlist => gating::is_allowed(inviter_id, &config.user_allowlist),
        };
        if !dm_allowed {
            return false;
        }
    } else {
        let room_allowed = match config.room_policy {
            GroupPolicy::Disabled => false,
            GroupPolicy::Open => true,
            GroupPolicy::Allowlist => {
                !config.room_allowlist.is_empty()
                    && gating::is_allowed(room_id, &config.room_allowlist)
            },
        };
        if !room_allowed {
            return false;
        }
    }

    match config.auto_join {
        AutoJoinPolicy::Always => true,
        AutoJoinPolicy::Off => false,
        AutoJoinPolicy::Allowlist => {
            gating::is_allowed(inviter_id, &config.user_allowlist)
                || gating::is_allowed(room_id, &config.room_allowlist)
        },
    }
}

fn checked_chat_type(
    config: &MatrixAccountConfig,
    sender_id: &str,
    room_id: &str,
    direct_flag: bool,
    active_members: u64,
    joined_members: u64,
    bot_mentioned: bool,
) -> Result<ChatType, access::AccessDenied> {
    let chat_type = infer_chat_type(direct_flag, active_members, joined_members);
    access::check_access(config, &chat_type, sender_id, room_id, bot_mentioned)?;
    Ok(chat_type)
}

fn is_bot_mentioned(
    event: &OriginalSyncRoomMessageEvent,
    bot_user_id: &OwnedUserId,
    body: &str,
) -> bool {
    event
        .content
        .mentions
        .as_ref()
        .is_some_and(|mentions| mentions.room || mentions.user_ids.contains(bot_user_id))
        || body.contains(bot_user_id.as_str())
}

pub(crate) fn first_selection(selections: &[String]) -> Option<String> {
    selections.first().cloned()
}

fn inbound_message_kind(msgtype: &MessageType) -> Option<ChannelMessageKind> {
    Some(match msgtype {
        MessageType::Text(_) | MessageType::Notice(_) => ChannelMessageKind::Text,
        MessageType::Image(_) => ChannelMessageKind::Photo,
        MessageType::Audio(audio) => infer_audio_kind(audio),
        MessageType::Video(_) => ChannelMessageKind::Video,
        MessageType::File(_) => ChannelMessageKind::Document,
        MessageType::Location(_) => ChannelMessageKind::Location,
        _ => return None,
    })
}

fn inbound_message_body(msgtype: &MessageType) -> String {
    match msgtype {
        MessageType::Text(text) => text.body.clone(),
        MessageType::Notice(notice) => notice.body.clone(),
        MessageType::Audio(audio) => audio_caption(audio).unwrap_or_default(),
        MessageType::Location(location) => location.plain_text_representation().to_string(),
        _ => String::new(),
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_audio_message(
    audio: &AudioMessageEventContent,
    room: &Room,
    account_id: &str,
    event_id: &str,
    sink: &dyn ChannelEventSink,
    reply_to: ChannelReplyTarget,
    mut meta: ChannelMessageMeta,
) {
    if !sink.voice_stt_available().await {
        if let Err(error) = send_text(
            room,
            "I received your audio message but voice transcription is not available. Please visit Settings -> Voice.",
        )
        .await
        {
            warn!(account_id, "failed to send STT setup hint: {error}");
        }
        return;
    }

    let format = audio_format(audio);
    let request = MediaRequestParameters {
        source: audio.source.clone(),
        format: MediaFormat::File,
    };

    let audio_data =
        match room
            .client()
            .media()
            .get_media_content(&request, true)
            .await
        {
            Ok(audio_data) => audio_data,
            Err(error) => {
                warn!(
                    account_id,
                    event_id, "failed to download Matrix audio: {error}"
                );
                if let Err(send_error) = send_text(
                room,
                "I received your audio message but couldn't download the audio. Please try again.",
            )
            .await
            {
                warn!(account_id, "failed to send Matrix audio download error: {send_error}");
            }
                return;
            },
        };

    meta.message_kind = Some(infer_audio_kind(audio));
    let filename = saved_audio_filename(
        event_id,
        audio.filename.as_deref(),
        inferred_filename(audio.body.as_str()),
        format,
    );
    meta.audio_filename = sink
        .save_channel_voice(&audio_data, &filename, &reply_to)
        .await;

    match sink.transcribe_voice(&audio_data, format).await {
        Ok(transcribed) => {
            let transcribed = transcribed.trim();
            let body = if transcribed.is_empty() {
                format!(
                    "[{} message - could not transcribe]",
                    audio_kind_label(meta.message_kind)
                )
            } else if let Some(caption) = audio_caption(audio) {
                format!("{caption}\n\n[Audio message]: {transcribed}")
            } else {
                transcribed.to_string()
            };

            sink.dispatch_to_chat(&body, reply_to, meta).await;
        },
        Err(error) => {
            warn!(
                account_id,
                event_id, "Matrix audio transcription failed: {error}"
            );
            let fallback = audio_caption(audio).unwrap_or_else(|| {
                format!(
                    "[{} message - transcription unavailable]",
                    audio_kind_label(meta.message_kind)
                )
            });
            sink.dispatch_to_chat(&fallback, reply_to, meta).await;
        },
    }
}

async fn handle_location_message(
    location: &LocationMessageEventContent,
    room: &Room,
    account_id: &str,
    event_id: &str,
    sink: &dyn ChannelEventSink,
    reply_to: ChannelReplyTarget,
    meta: ChannelMessageMeta,
) {
    let Some((latitude, longitude)) = parse_geo_uri(location.geo_uri()) else {
        warn!(
            account_id,
            event_id,
            geo_uri = location.geo_uri(),
            "received Matrix location with invalid geo URI"
        );
        let body = location.plain_text_representation().trim().to_string();
        if !body.is_empty() {
            sink.dispatch_to_chat(&body, reply_to, meta).await;
        }
        return;
    };

    let resolved = sink.update_location(&reply_to, latitude, longitude).await;
    info!(
        account_id,
        event_id,
        latitude,
        longitude,
        resolved_pending_request = resolved,
        "Matrix location received"
    );

    if resolved {
        if let Err(error) = send_text(room, "Location updated.").await {
            warn!(
                account_id,
                "failed to send Matrix location confirmation: {error}"
            );
        }
        return;
    }

    sink.dispatch_to_chat(
        &location_dispatch_body(location, latitude, longitude),
        reply_to,
        meta,
    )
    .await;
}

fn audio_kind_label(kind: Option<ChannelMessageKind>) -> &'static str {
    match kind {
        Some(ChannelMessageKind::Voice) => "voice",
        _ => "audio",
    }
}

fn infer_audio_kind(audio: &AudioMessageEventContent) -> ChannelMessageKind {
    match audio_format(audio) {
        "ogg" | "opus" => ChannelMessageKind::Voice,
        _ => ChannelMessageKind::Audio,
    }
}

fn audio_caption(audio: &AudioMessageEventContent) -> Option<String> {
    audio
        .caption()
        .map(str::trim)
        .filter(|caption| !caption.is_empty())
        .map(ToOwned::to_owned)
}

fn audio_format(audio: &AudioMessageEventContent) -> &'static str {
    audio_format_from_metadata(
        audio
            .info
            .as_ref()
            .and_then(|info| info.mimetype.as_deref()),
        audio
            .filename
            .as_deref()
            .or_else(|| inferred_filename(audio.body.as_str())),
    )
}

fn audio_format_from_metadata(mimetype: Option<&str>, filename: Option<&str>) -> &'static str {
    if let Some(mimetype) = mimetype
        && let Some(format) = audio_format_from_mimetype(mimetype)
    {
        return format;
    }

    filename
        .and_then(|filename| {
            std::path::Path::new(filename)
                .extension()
                .and_then(|ext| ext.to_str())
        })
        .and_then(audio_format_from_extension)
        .unwrap_or("ogg")
}

fn audio_format_from_mimetype(mimetype: &str) -> Option<&'static str> {
    Some(match mimetype {
        "audio/ogg" | "audio/ogg; codecs=opus" => "ogg",
        "audio/opus" => "opus",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" | "audio/aac" => "m4a",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/webm" => "webm",
        "audio/flac" | "audio/x-flac" => "flac",
        _ => return None,
    })
}

fn audio_format_from_extension(extension: &str) -> Option<&'static str> {
    Some(match extension.to_ascii_lowercase().as_str() {
        "ogg" => "ogg",
        "opus" => "opus",
        "mp3" => "mp3",
        "m4a" | "aac" => "m4a",
        "wav" => "wav",
        "webm" => "webm",
        "flac" => "flac",
        _ => return None,
    })
}

fn inferred_filename(body: &str) -> Option<&str> {
    let candidate = body.trim();
    if candidate.is_empty() || candidate.contains('\n') {
        return None;
    }

    let extension = std::path::Path::new(candidate)
        .extension()
        .and_then(|ext| ext.to_str())?;
    audio_format_from_extension(extension).map(|_| candidate)
}

fn update_utd_notice_window(
    notices: &mut std::collections::HashMap<String, Instant>,
    room_id: &str,
    now: Instant,
) -> bool {
    let Some(previous) = notices.get(room_id).copied() else {
        notices.insert(room_id.to_string(), now);
        return true;
    };

    if now.duration_since(previous) < UTD_NOTICE_COOLDOWN {
        return false;
    }

    notices.insert(room_id.to_string(), now);
    true
}

fn utd_notice_message(verification_state: VerificationState) -> &'static str {
    match verification_state {
        VerificationState::Verified => {
            "I received an encrypted Matrix message but could not decrypt it yet. This Moltis device is likely missing room keys for older history. Resend the message after verification, or share keys from Element if needed."
        },
        VerificationState::Unknown | VerificationState::Unverified => {
            "I received an encrypted Matrix message but could not decrypt it yet. This Moltis device likely still needs verification or room keys. Start a fresh Element verification with the bot, then send `verify show` as a normal message in this same Matrix chat if you need the emoji prompt again. After verification finishes, resend the message."
        },
    }
}

fn otp_request_message() -> &'static str {
    "To use this bot, please enter the verification code.\n\n\
     Ask the bot owner for the code, it is visible in the Moltis web UI under Channels -> Senders.\n\n\
     The code expires in 5 minutes."
}

fn saved_audio_filename(
    event_id: &str,
    filename: Option<&str>,
    body_filename: Option<&str>,
    format: &str,
) -> String {
    let candidate = filename
        .or(body_filename)
        .map(str::trim)
        .filter(|name| !name.is_empty());
    if let Some(candidate) = candidate {
        let cleaned = candidate
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(candidate)
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                    ch
                } else {
                    '_'
                }
            })
            .collect::<String>();
        if !cleaned.is_empty() {
            if std::path::Path::new(&cleaned).extension().is_some() {
                return cleaned;
            }
            return format!("{cleaned}.{format}");
        }
    }

    let suffix = event_id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>();
    format!("voice-matrix-{}.{}", suffix, format)
}

fn parse_geo_uri(geo_uri: &str) -> Option<(f64, f64)> {
    let coordinates = geo_uri.trim().strip_prefix("geo:")?;
    let mut parts = coordinates.split(';');
    let lat_lon = parts.next()?;
    let mut lat_lon_parts = lat_lon.split(',');
    let latitude = lat_lon_parts.next()?.trim().parse().ok()?;
    let longitude = lat_lon_parts.next()?.trim().parse().ok()?;
    Some((latitude, longitude))
}

fn is_valid_lat_lon(latitude: f64, longitude: f64) -> bool {
    (-90.0..=90.0).contains(&latitude) && (-180.0..=180.0).contains(&longitude)
}

fn parse_coordinate_component(input: &str) -> Option<f64> {
    let trimmed = input
        .trim()
        .trim_matches(|c| matches!(c, '(' | ')' | '[' | ']' | '{' | '}'));
    if trimmed.is_empty() {
        return None;
    }

    let mut end = 0usize;
    for (idx, ch) in trimmed.char_indices() {
        if ch.is_ascii_digit() || matches!(ch, '+' | '-' | '.') {
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 {
        return None;
    }

    let token = &trimmed[..end];
    if !token.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }

    token.parse::<f64>().ok()
}

fn parse_coordinate_pair(input: &str) -> Option<(f64, f64)> {
    let mut parts = input.split(',');
    let latitude = parse_coordinate_component(parts.next()?)?;
    let longitude = parse_coordinate_component(parts.next()?)?;
    if is_valid_lat_lon(latitude, longitude) {
        Some((latitude, longitude))
    } else {
        None
    }
}

fn parse_coordinates_from_url(url_str: &str) -> Option<(f64, f64)> {
    let parsed = reqwest::Url::parse(url_str).ok()?;

    for key in ["ll", "q", "query"] {
        if let Some((_, value)) = parsed
            .query_pairs()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            && let Some(coords) = parse_coordinate_pair(value.as_ref())
        {
            return Some(coords);
        }
    }

    for segment in [
        parsed.path(),
        parsed.fragment().unwrap_or_default(),
        url_str,
    ] {
        if let Some(at_pos) = segment.find('@')
            && let Some(coords) = parse_coordinate_pair(&segment[at_pos + 1..])
        {
            return Some(coords);
        }
    }

    None
}

fn parse_map_link_coordinates(text: &str) -> Option<(f64, f64)> {
    for raw in text.split_whitespace() {
        let token = raw.trim_matches(|c: char| {
            matches!(
                c,
                '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | '.' | '!' | '?'
            )
        });
        if let Some(coords) = parse_geo_uri(token) {
            return Some(coords);
        }
        if !(token.starts_with("http://") || token.starts_with("https://")) {
            continue;
        }
        if let Some(coords) = parse_coordinates_from_url(token) {
            return Some(coords);
        }
    }

    None
}

fn parse_plain_text_coordinates(text: &str) -> Option<(f64, f64)> {
    let trimmed = text.trim();
    if trimmed.is_empty() || !trimmed.contains(',') {
        return None;
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | ',' | ' ' | '\t' | '(' | ')'))
    {
        return None;
    }

    parse_coordinate_pair(trimmed)
}

fn extract_location_coordinates(text: &str) -> Option<(f64, f64)> {
    parse_map_link_coordinates(text).or_else(|| parse_plain_text_coordinates(text))
}

fn location_dispatch_body(
    location: &LocationMessageEventContent,
    latitude: f64,
    longitude: f64,
) -> String {
    let description = location.plain_text_representation().trim();
    if description.is_empty() || description == location.geo_uri() {
        return format!("I'm sharing my location: {latitude}, {longitude}");
    }

    format!("{description}\n\nShared location: {latitude}, {longitude}")
}

fn record_message_received() {
    #[cfg(feature = "metrics")]
    moltis_metrics::counter!(
        moltis_metrics::channels::MESSAGES_RECEIVED_TOTAL,
        moltis_metrics::labels::CHANNEL => "matrix"
    )
    .increment(1);
}

fn infer_chat_type(
    direct_flag: bool,
    active_members_count: u64,
    joined_members_count: u64,
) -> ChatType {
    if direct_flag || active_members_count == 2 || joined_members_count == 2 {
        ChatType::Dm
    } else {
        ChatType::Group
    }
}

async fn send_status_text(room: &Room, text: &str, context: &str) {
    if let Err(error) = send_text(room, text).await {
        warn!(
            room = %room.room_id(),
            context,
            "failed to send Matrix status text: {error}"
        );
    }
}

async fn handle_otp(
    body: &str,
    sender_id: &str,
    account_id: &str,
    accounts: &AccountStateMap,
    event_sink: &Option<Arc<dyn ChannelEventSink>>,
    room: &Room,
) {
    let trimmed = body.trim();

    if trimmed.len() == 6 && trimmed.chars().all(|c| c.is_ascii_digit()) {
        let result = {
            let guard = accounts.read().unwrap_or_else(|e| e.into_inner());
            if let Some(state) = guard.get(account_id) {
                let mut otp = state.otp.lock().unwrap_or_else(|e| e.into_inner());
                otp.verify(sender_id, trimmed)
            } else {
                return;
            }
        };

        match result {
            OtpVerifyResult::Approved => {
                approve_sender_via_otp(
                    event_sink.as_deref(),
                    ChannelType::Matrix,
                    account_id,
                    sender_id,
                    sender_id,
                    Some(sender_id),
                )
                .await;
                send_status_text(room, "Access granted.", "otp approved").await;
            },
            OtpVerifyResult::WrongCode { attempts_left } => {
                let msg = format!("Invalid code. {attempts_left} attempts remaining.");
                send_status_text(room, &msg, "otp wrong code").await;
            },
            OtpVerifyResult::Expired => {
                send_status_text(
                    room,
                    "Code expired. Send any message for a new one.",
                    "otp expired",
                )
                .await;
                emit_otp_resolution(
                    event_sink.as_deref(),
                    ChannelType::Matrix,
                    account_id,
                    sender_id,
                    Some(sender_id),
                    "expired",
                )
                .await;
            },
            OtpVerifyResult::LockedOut => {
                send_status_text(room, "Too many attempts. Please wait.", "otp locked").await;
                emit_otp_resolution(
                    event_sink.as_deref(),
                    ChannelType::Matrix,
                    account_id,
                    sender_id,
                    Some(sender_id),
                    "locked_out",
                )
                .await;
            },
            OtpVerifyResult::NoPending => {
                // Fall through to initiate
            },
        }
        if !matches!(result, OtpVerifyResult::NoPending) {
            return;
        }
    }

    let (result, otp_cooldown_secs) = {
        let guard = accounts.read().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = guard.get(account_id) {
            let mut otp = state.otp.lock().unwrap_or_else(|e| e.into_inner());
            (
                otp.initiate(sender_id, Some(sender_id.into()), None),
                state.config.otp_cooldown_secs,
            )
        } else {
            return;
        }
    };

    match result {
        OtpInitResult::Created(code) => {
            let expires_at =
                unix_now().saturating_add(i64::try_from(otp_cooldown_secs).unwrap_or(i64::MAX));
            let msg = otp_request_message().to_string();
            send_status_text(room, &msg, "otp created").await;
            emit_otp_challenge(
                event_sink.as_deref(),
                ChannelType::Matrix,
                account_id,
                sender_id,
                Some(sender_id),
                Some(sender_id),
                code,
                expires_at,
            )
            .await;
        },
        OtpInitResult::AlreadyPending => {
            send_status_text(
                room,
                "A verification code is already pending.",
                "otp already pending",
            )
            .await;
        },
        OtpInitResult::LockedOut => {
            send_status_text(
                room,
                "Too many failed attempts. Please wait.",
                "otp locked out",
            )
            .await;
        },
    }
}

#[tracing::instrument(skip(ev, room, accounts, bot_user_id), fields(account_id, room = %room.room_id(), inviter = %ev.sender))]
pub async fn handle_invite(
    ev: StrippedRoomMemberEvent,
    room: Room,
    account_id: String,
    accounts: AccountStateMap,
    bot_user_id: OwnedUserId,
) {
    if ev.state_key != bot_user_id {
        return;
    }

    let is_direct = ev.content.is_direct.unwrap_or(false);

    let auto_join = {
        let guard = accounts.read().unwrap_or_else(|e| e.into_inner());
        match guard.get(&account_id) {
            Some(state) => should_auto_join_invite(
                &state.config,
                ev.sender.as_str(),
                room.room_id().as_str(),
                is_direct,
            ),
            None => return,
        }
    };

    if !auto_join {
        debug!(account_id, room = %room.room_id(), is_direct, "ignoring invite (auto_join policy)");
        return;
    }

    info!(account_id, room = %room.room_id(), inviter = %ev.sender, is_direct, "auto-joining room");
    if let Err(e) = room.join().await {
        warn!(account_id, room = %room.room_id(), "failed to auto-join: {e}");
    }
}

pub async fn send_text(room: &Room, text: &str) -> Result<(), matrix_sdk::Error> {
    use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;
    let content = RoomMessageEventContent::text_plain(text);
    room.send(content).await?;
    Ok(())
}

fn unix_now() -> i64 {
    OffsetDateTime::now_utc().unix_timestamp()
}

async fn log_inbound_message(message_log: &Option<Arc<dyn MessageLog>>, entry: MessageLogEntry) {
    if let Some(log) = message_log
        && let Err(error) = log.log(entry).await
    {
        warn!(error = %error, "failed to log Matrix inbound message");
    }
}

async fn emit_inbound_message_event(
    event_sink: &Option<Arc<dyn ChannelEventSink>>,
    account_id: &str,
    sender_id: &str,
    sender_name: Option<String>,
    access_granted: bool,
) {
    if let Some(sink) = event_sink {
        sink.emit(ChannelEvent::InboundMessage {
            channel_type: ChannelType::Matrix,
            account_id: account_id.to_string(),
            peer_id: sender_id.to_string(),
            username: Some(sender_id.to_string()),
            sender_name,
            message_count: None,
            access_granted,
        })
        .await;
    }
}

#[cfg(test)]
mod tests;
