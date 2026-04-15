//! Media sending logic for Telegram (images, audio, documents, URLs).

use {
    base64::Engine,
    teloxide::{
        payloads::{SendAudioSetters, SendDocumentSetters, SendPhotoSetters, SendVoiceSetters},
        prelude::*,
        types::{ChatId, InputFile, ReplyParameters, ThreadId},
    },
    tracing::{debug, info},
};

use {
    moltis_channels::{Error as ChannelError, Result},
    moltis_common::types::ReplyPayload,
};

use crate::topic::parse_chat_target;

use super::{TelegramOutbound, retry::RequestResultExt};

/// Inner implementation of `send_media` for the `ChannelOutbound` trait.
/// Kept in a dedicated module to reduce the size of `send.rs`.
pub(super) async fn send_media_impl(
    outbound: &TelegramOutbound,
    account_id: &str,
    to: &str,
    payload: &ReplyPayload,
    reply_to: Option<&str>,
) -> Result<()> {
    let bot = outbound.get_bot(account_id)?;
    let (chat_id, thread_id) = parse_chat_target(to)?;
    let rp = outbound.reply_params(account_id, reply_to);
    let media_mime = payload
        .media
        .as_ref()
        .map(|m| m.mime_type.as_str())
        .unwrap_or("none");
    info!(
        account_id,
        chat_id = to,
        reply_to = ?reply_to,
        has_media = payload.media.is_some(),
        media_mime,
        caption_len = payload.text.len(),
        "telegram outbound media send start"
    );

    if let Some(ref media) = payload.media {
        if media.url.starts_with("data:") {
            send_base64_media(
                outbound, &bot, account_id, to, chat_id, thread_id, &rp, payload,
            )
            .await?;
        } else {
            send_url_media(&bot, account_id, to, chat_id, thread_id, payload).await?;
        }
    } else if !payload.text.is_empty() {
        // No media attachment -- fall back to plain text.
        use moltis_channels::plugin::ChannelOutbound;
        outbound
            .send_text(account_id, to, &payload.text, reply_to)
            .await?;
    }

    Ok(())
}

async fn send_base64_media(
    outbound: &TelegramOutbound,
    bot: &Bot,
    account_id: &str,
    to: &str,
    chat_id: ChatId,
    thread_id: Option<ThreadId>,
    rp: &Option<ReplyParameters>,
    payload: &ReplyPayload,
) -> Result<()> {
    let media = payload
        .media
        .as_ref()
        .ok_or_else(|| ChannelError::invalid_input("send_base64_media called without media"))?;

    // Parse data URI: data:<mime>;base64,<data>
    let Some(comma_pos) = media.url.find(',') else {
        return Err(ChannelError::invalid_input(
            "invalid data URI: no comma separator",
        ));
    };
    let base64_data = &media.url[comma_pos + 1..];
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_data)
        .map_err(|e| ChannelError::invalid_input(format!("failed to decode base64: {e}")))?;

    debug!(
        bytes = bytes.len(),
        mime_type = %media.mime_type,
        "sending base64 media to telegram"
    );

    // Use the original filename when provided, otherwise derive from MIME type.
    let filename = media.filename.clone().unwrap_or_else(|| {
        let ext = moltis_media::mime::extension_for_mime(&media.mime_type);
        format!("file.{ext}")
    });

    if media.mime_type.starts_with("image/") {
        send_base64_image(
            outbound, bot, account_id, to, chat_id, thread_id, rp, payload, &bytes, &filename,
        )
        .await?;
    } else if media.mime_type == "audio/ogg" {
        let input = InputFile::memory(bytes).file_name("voice.ogg");
        let mut req = bot.send_voice(chat_id, input);
        if let Some(tid) = thread_id {
            req = req.message_thread_id(tid);
        }
        if !payload.text.is_empty() {
            req = req.caption(&payload.text);
        }
        req.await.channel_context("send voice media")?;
        info!(
            account_id,
            chat_id = to,
            media_mime = %media.mime_type,
            caption_len = payload.text.len(),
            "telegram outbound media sent as voice"
        );
    } else if media.mime_type.starts_with("audio/") {
        let input = InputFile::memory(bytes).file_name("audio.mp3");
        let mut req = bot.send_audio(chat_id, input);
        if let Some(tid) = thread_id {
            req = req.message_thread_id(tid);
        }
        if !payload.text.is_empty() {
            req = req.caption(&payload.text);
        }
        req.await.channel_context("send audio media")?;
        info!(
            account_id,
            chat_id = to,
            media_mime = %media.mime_type,
            caption_len = payload.text.len(),
            "telegram outbound media sent as audio"
        );
    } else {
        let input = InputFile::memory(bytes).file_name(filename);
        let mut req = bot.send_document(chat_id, input);
        if let Some(tid) = thread_id {
            req = req.message_thread_id(tid);
        }
        if !payload.text.is_empty() {
            req = req.caption(&payload.text);
        }
        req.await.channel_context("send document media")?;
        info!(
            account_id,
            chat_id = to,
            media_mime = %media.mime_type,
            caption_len = payload.text.len(),
            "telegram outbound media sent as document"
        );
    }

    Ok(())
}

/// Send a base64-decoded image, falling back to document if photo dimensions
/// are rejected by Telegram.
async fn send_base64_image(
    outbound: &TelegramOutbound,
    bot: &Bot,
    account_id: &str,
    to: &str,
    chat_id: ChatId,
    thread_id: Option<ThreadId>,
    rp: &Option<ReplyParameters>,
    payload: &ReplyPayload,
    bytes: &[u8],
    filename: &str,
) -> Result<()> {
    let media = payload
        .media
        .as_ref()
        .ok_or_else(|| ChannelError::invalid_input("send_base64_image called without media"))?;

    let input = InputFile::memory(bytes.to_vec()).file_name(filename.to_string());
    let mut req = bot.send_photo(chat_id, input);
    if let Some(tid) = thread_id {
        req = req.message_thread_id(tid);
    }
    if !payload.text.is_empty() {
        req = req.caption(&payload.text);
    }
    if let Some(rp) = rp {
        req = req.reply_parameters(rp.clone());
    }

    // Suppress the unused-variable warning on `outbound` -- it is used only to
    // satisfy the function signature for symmetry with other senders.
    let _ = outbound;

    match req.await {
        Ok(_) => {
            info!(
                account_id,
                chat_id = to,
                media_mime = %media.mime_type,
                caption_len = payload.text.len(),
                "telegram outbound media sent as photo"
            );
        },
        Err(e) => {
            let err_str = e.to_string();
            // Retry as document if photo dimensions are invalid
            if err_str.contains("PHOTO_INVALID_DIMENSIONS")
                || err_str.contains("PHOTO_SAVE_FILE_INVALID")
            {
                debug!(
                    error = %err_str,
                    "photo rejected, retrying as document"
                );
                let input = InputFile::memory(bytes.to_vec()).file_name(filename.to_string());
                let mut req = bot.send_document(chat_id, input);
                if let Some(tid) = thread_id {
                    req = req.message_thread_id(tid);
                }
                if !payload.text.is_empty() {
                    req = req.caption(&payload.text);
                }
                req.await.channel_context("send document fallback")?;
                info!(
                    account_id,
                    chat_id = to,
                    media_mime = %media.mime_type,
                    caption_len = payload.text.len(),
                    "telegram outbound media sent as document fallback"
                );
            } else {
                return Err(ChannelError::external("send media photo", e));
            }
        },
    }

    Ok(())
}

async fn send_url_media(
    bot: &Bot,
    account_id: &str,
    to: &str,
    chat_id: ChatId,
    thread_id: Option<ThreadId>,
    payload: &ReplyPayload,
) -> Result<()> {
    let media = payload
        .media
        .as_ref()
        .ok_or_else(|| ChannelError::invalid_input("send_url_media called without media"))?;

    let url = media.url.parse().map_err(|e| {
        ChannelError::invalid_input(format!("invalid media URL '{}': {e}", media.url))
    })?;
    let input = InputFile::url(url);

    match media.mime_type.as_str() {
        t if t.starts_with("image/") => {
            let mut req = bot.send_photo(chat_id, input);
            if !payload.text.is_empty() {
                req = req.caption(&payload.text);
            }
            req.await.channel_context("send URL photo media")?;
            info!(
                account_id,
                chat_id = to,
                media_mime = %media.mime_type,
                caption_len = payload.text.len(),
                "telegram outbound URL media sent as photo"
            );
        },
        "audio/ogg" => {
            let mut req = bot.send_voice(chat_id, input);
            if let Some(tid) = thread_id {
                req = req.message_thread_id(tid);
            }
            if !payload.text.is_empty() {
                req = req.caption(&payload.text);
            }
            req.await.channel_context("send URL voice media")?;
            info!(
                account_id,
                chat_id = to,
                media_mime = %media.mime_type,
                caption_len = payload.text.len(),
                "telegram outbound URL media sent as voice"
            );
        },
        t if t.starts_with("audio/") => {
            let mut req = bot.send_audio(chat_id, input);
            if let Some(tid) = thread_id {
                req = req.message_thread_id(tid);
            }
            if !payload.text.is_empty() {
                req = req.caption(&payload.text);
            }
            req.await.channel_context("send URL audio media")?;
            info!(
                account_id,
                chat_id = to,
                media_mime = %media.mime_type,
                caption_len = payload.text.len(),
                "telegram outbound URL media sent as audio"
            );
        },
        _ => {
            let mut req = bot.send_document(chat_id, input);
            if let Some(tid) = thread_id {
                req = req.message_thread_id(tid);
            }
            if !payload.text.is_empty() {
                req = req.caption(&payload.text);
            }
            req.await.channel_context("send URL document media")?;
            info!(
                account_id,
                chat_id = to,
                media_mime = %media.mime_type,
                caption_len = payload.text.len(),
                "telegram outbound URL media sent as document"
            );
        },
    }

    Ok(())
}
