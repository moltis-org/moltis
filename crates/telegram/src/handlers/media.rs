use std::sync::Arc;

use {
    teloxide::{
        prelude::*,
        types::{MediaKind, MessageKind},
    },
    tracing::{debug, warn},
};

use {
    moltis_channels::{
        ChannelAttachment, ChannelDocumentFile, ChannelEventSink, ChannelMessageKind,
        ChannelReplyTarget, SavedChannelFile,
    },
    moltis_common::types::ChatType,
};

use super::outbound_to_for_msg;

use crate::Result;

pub(super) fn extract_text(msg: &Message) -> Option<String> {
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

pub(super) fn has_media(msg: &Message) -> bool {
    match &msg.kind {
        MessageKind::Common(common) => !matches!(common.media_kind, MediaKind::Text(_)),
        _ => false,
    }
}

#[allow(dead_code)]
pub(super) fn extract_media_url(msg: &Message) -> Option<String> {
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

pub(super) struct VoiceFileInfo {
    pub(super) file_id: String,
    pub(super) format: String,
}

pub(super) fn extract_voice_file(msg: &Message) -> Option<VoiceFileInfo> {
    match &msg.kind {
        MessageKind::Common(common) => match &common.media_kind {
            MediaKind::Voice(v) => Some(VoiceFileInfo {
                file_id: v.voice.file.id.clone(),
                format: "ogg".to_string(),
            }),
            MediaKind::Audio(a) => {
                let format = a
                    .audio
                    .mime_type
                    .as_ref()
                    .map(|m| match m.as_ref() {
                        "audio/mpeg" | "audio/mp3" => "mp3",
                        "audio/mp4" | "audio/m4a" | "audio/x-m4a" => "m4a",
                        "audio/ogg" | "audio/opus" => "ogg",
                        "audio/wav" | "audio/x-wav" => "wav",
                        "audio/webm" => "webm",
                        _ => "mp3",
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

pub(super) struct PhotoFileInfo {
    pub(super) file_id: String,
    pub(super) media_type: String,
}

pub(super) fn extract_photo_file(msg: &Message) -> Option<PhotoFileInfo> {
    match &msg.kind {
        MessageKind::Common(common) => match &common.media_kind {
            MediaKind::Photo(p) => p.photo.last().map(|ps| PhotoFileInfo {
                file_id: ps.file.id.clone(),
                media_type: "image/jpeg".to_string(),
            }),
            _ => None,
        },
        _ => None,
    }
}

pub(super) struct DocumentFileInfo {
    pub(super) file_id: String,
    pub(super) media_type: String,
    pub(super) file_name: Option<String>,
}

pub(super) fn extract_document_file(msg: &Message) -> Option<DocumentFileInfo> {
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

pub(super) const MAX_INLINE_DOCUMENT_BYTES: usize = 64 * 1024;
pub(super) const MAX_INLINE_DOCUMENT_CHARS: usize = 24_000;

pub(super) fn format_document_label(file_name: Option<&str>, media_type: &str) -> String {
    match file_name {
        Some(name) if !name.trim().is_empty() => format!("[Document: {name} ({media_type})]"),
        _ => format!("[Document: {media_type}]"),
    }
}

pub(super) fn sanitize_document_filename(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
        .collect();
    sanitized.trim_start_matches('.').to_string()
}

pub(super) fn build_saved_document_filename(
    file_name: Option<&str>,
    media_type: &str,
    file_id: &str,
) -> String {
    let ext = moltis_media::mime::extension_for_mime(media_type);
    let file_id_prefix: String = sanitize_document_filename(file_id)
        .chars()
        .take(16)
        .collect();

    let base_name = file_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(sanitize_document_filename)
        .filter(|name| !name.is_empty())
        .map(|name| {
            if name.contains('.') {
                name
            } else {
                format!("{name}.{ext}")
            }
        })
        .unwrap_or_else(|| format!("document.{ext}"));

    if file_id_prefix.is_empty() {
        base_name
    } else {
        format!("{file_id_prefix}_{base_name}")
    }
}

pub(super) fn channel_document_file(
    saved: &SavedChannelFile,
    file_name: Option<&str>,
    media_type: &str,
) -> ChannelDocumentFile {
    ChannelDocumentFile {
        display_name: file_name
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(&saved.filename)
            .to_string(),
        stored_filename: saved.filename.clone(),
        mime_type: media_type.to_string(),
    }
}

pub(super) fn build_document_body(
    caption: &str,
    doc_label: &str,
    extracted_text: Option<&str>,
) -> String {
    let mut sections = Vec::new();
    if !caption.is_empty() {
        sections.push(caption.to_string());
    }
    sections.push(doc_label.to_string());
    if let Some(text) = extracted_text
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        sections.push(text.to_string());
    }
    sections.join("\n\n")
}

pub(super) fn normalize_media_type(media_type: &str) -> String {
    media_type
        .split(';')
        .next()
        .unwrap_or(media_type)
        .trim()
        .to_ascii_lowercase()
}

pub(super) fn should_inline_document_text(media_type: &str) -> bool {
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

pub(super) fn is_pdf_document_type(media_type: &str) -> bool {
    media_type == "application/pdf"
}

pub(super) fn is_supported_document_type(media_type: &str) -> bool {
    media_type.starts_with("image/")
        || should_inline_document_text(media_type)
        || is_pdf_document_type(media_type)
}

pub(super) fn truncate_inline_document_text(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut truncated = false;
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

pub(super) fn extract_text_document_content(data: &[u8], media_type: &str) -> Option<String> {
    if data.is_empty() || !should_inline_document_text(media_type) {
        return None;
    }

    let byte_truncated = data.len() > MAX_INLINE_DOCUMENT_BYTES;
    let bounded = if byte_truncated {
        let slice = &data[..MAX_INLINE_DOCUMENT_BYTES];
        match std::str::from_utf8(slice) {
            Ok(_) => slice,
            Err(e) => &slice[..e.valid_up_to()],
        }
    } else {
        data
    };

    let lossy = String::from_utf8_lossy(bounded);
    let mut text = truncate_inline_document_text(&lossy)?;
    if byte_truncated && !text.contains("[Document content truncated]") {
        text.push_str("\n\n[Document content truncated]");
    }
    Some(text)
}

pub(super) async fn extract_pdf_document_content(data: Vec<u8>) -> Option<String> {
    let extracted = tokio::task::spawn_blocking(move || {
        use std::io::Write as _;

        let mut file = tempfile::Builder::new().suffix(".pdf").tempfile().ok()?;
        file.write_all(&data).ok()?;
        pdf_extract::extract_text(file.path()).ok()
    })
    .await
    .ok()??;
    truncate_inline_document_text(&extracted)
}

pub(super) async fn save_inbound_document(
    event_sink: Option<&Arc<dyn ChannelEventSink>>,
    reply_to: &ChannelReplyTarget,
    file_name: Option<&str>,
    media_type: &str,
    file_id: &str,
    data: &[u8],
) -> Option<SavedChannelFile> {
    let sink = event_sink?;
    let filename = build_saved_document_filename(file_name, media_type, file_id);
    sink.save_channel_attachment(data, &filename, reply_to)
        .await
}

pub(super) struct LocationInfo {
    pub(super) latitude: f64,
    pub(super) longitude: f64,
    pub(super) is_live: bool,
}

pub(super) fn extract_location(msg: &Message) -> Option<LocationInfo> {
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

pub(super) fn describe_media_kind(msg: &Message) -> Option<&'static str> {
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

pub(super) fn message_kind(msg: &Message) -> Option<ChannelMessageKind> {
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

pub(super) async fn download_telegram_file(bot: &Bot, file_id: &str) -> Result<Vec<u8>> {
    let file = bot.get_file(file_id).await?;
    let token = bot.token();
    let base = bot.api_url();
    let url = format!("{base}file/bot{token}/{}", file.path);

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

pub(super) fn classify_chat(msg: &Message) -> (ChatType, Option<String>) {
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

pub(super) fn check_bot_mentioned(msg: &Message, bot_username: Option<&str>) -> bool {
    let text = extract_text(msg).unwrap_or_default();
    if let Some(username) = bot_username {
        text.contains(&format!("@{username}"))
    } else {
        false
    }
}

#[allow(dead_code)]
pub(super) fn build_session_key(
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

pub(super) const VOICE_REPLY_EMPTY_TRANSCRIPTION: &str =
    "I couldn't hear anything in that voice message. Could you try again or type it out?";
pub(super) const VOICE_REPLY_TRANSCRIPTION_FAILED: &str =
    "I couldn't transcribe your voice message. Could you try again or type it out?";
pub(super) const VOICE_REPLY_DOWNLOAD_FAILED: &str =
    "I couldn't download your voice message. Please try again.";
pub(super) const VOICE_REPLY_UNAVAILABLE: &str =
    "I received your voice message but voice processing is not available right now.";
pub(super) const VOICE_REPLY_STT_SETUP_HINT: &str =
    "I can't understand voice, you did not configure it, please visit Settings -> Voice";

#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_voice_message(
    bot: &Bot,
    msg: &Message,
    account_id: &str,
    caption: Option<&str>,
    event_sink: Option<&Arc<dyn ChannelEventSink>>,
    outbound: &dyn moltis_channels::ChannelOutbound,
    voice_file: &VoiceFileInfo,
) -> Option<(String, Vec<ChannelAttachment>, Option<(Vec<u8>, String)>)> {
    let caption_text = caption
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let reply_target = outbound_to_for_msg(msg);

    async fn send_direct_reply(
        outbound: &dyn moltis_channels::ChannelOutbound,
        account_id: &str,
        to: &str,
        text: &str,
    ) {
        if let Err(e) = outbound.send_text(account_id, to, text, None).await {
            warn!(account_id, "failed to send voice fallback reply: {e}");
        }
    }

    let Some(sink) = event_sink else {
        warn!(
            account_id,
            "no event sink available for voice message; sending direct reply"
        );
        send_direct_reply(outbound, account_id, &reply_target, VOICE_REPLY_UNAVAILABLE).await;
        return None;
    };

    if !sink.voice_stt_available().await {
        if let Some(caption) = caption_text {
            return Some((caption, Vec::new(), None));
        }
        if let Err(e) = outbound
            .send_text(account_id, &reply_target, VOICE_REPLY_STT_SETUP_HINT, None)
            .await
        {
            warn!(account_id, "failed to send STT setup hint: {e}");
        }
        return None;
    }

    let audio_data = match download_telegram_file(bot, &voice_file.file_id).await {
        Ok(data) => data,
        Err(e) => {
            warn!(account_id, error = %e, "failed to download voice file");
            if let Some(caption) = caption_text {
                return Some((caption, Vec::new(), None));
            }
            send_direct_reply(
                outbound,
                account_id,
                &reply_target,
                VOICE_REPLY_DOWNLOAD_FAILED,
            )
            .await;
            return None;
        },
    };

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
            if let Some(caption) = caption_text {
                return Some((caption, Vec::new(), saved_audio));
            }
            send_direct_reply(
                outbound,
                account_id,
                &reply_target,
                VOICE_REPLY_EMPTY_TRANSCRIPTION,
            )
            .await;
            None
        },
        Ok(transcribed) => {
            debug!(
                account_id,
                text_len = transcribed.len(),
                "voice transcription successful"
            );
            let body = match caption_text {
                Some(caption) => format!("{caption}\n\n[Voice message]: {transcribed}"),
                None => transcribed,
            };
            Some((body, Vec::new(), saved_audio))
        },
        Err(e) => {
            warn!(account_id, error = %e, "voice transcription failed");
            if let Some(caption) = caption_text {
                return Some((caption, Vec::new(), saved_audio));
            }
            send_direct_reply(
                outbound,
                account_id,
                &reply_target,
                VOICE_REPLY_TRANSCRIPTION_FAILED,
            )
            .await;
            None
        },
    }
}
