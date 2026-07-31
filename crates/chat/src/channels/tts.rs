//! Text-to-speech synthesis for channel and web UI replies.
//!
//! Both entry points go through the `tts.status` / `tts.convert` service pair:
//! [`generate_tts_audio`] returns raw bytes for the web UI, while
//! [`build_tts_payload`] wraps them in a channel [`ReplyPayload`] with the
//! original (unsanitized) text kept as the caption.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{error, runtime::ChatRuntime};

#[derive(Debug, Deserialize)]
struct TtsStatusResponse {
    enabled: bool,
}

#[derive(Debug, Serialize)]
struct TtsConvertRequest<'a> {
    text: &'a str,
    format: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "voiceId")]
    voice_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TtsConvertResponse {
    audio: String,
    #[serde(default)]
    mime_type: Option<String>,
}

/// Generate TTS audio bytes for a web UI response.
///
/// Uses the session-level TTS override if configured, otherwise the global TTS
/// config. Returns raw audio bytes (OGG format) on success, and an error if TTS
/// is disabled, the text has nothing speakable, or generation fails.
pub(crate) async fn generate_tts_audio(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    text: &str,
) -> error::Result<Vec<u8>> {
    use base64::Engine;

    let tts_status = state
        .tts_service()
        .status()
        .await
        .map_err(error::Error::message)?;
    let status: TtsStatusResponse = serde_json::from_value(tts_status)
        .map_err(|_| error::Error::message("invalid tts.status response"))?;
    if !status.enabled {
        return Err(error::Error::message("TTS is disabled or not configured"));
    }

    // Layer 2: strip markdown/URLs the LLM may have included despite the prompt.
    let text = moltis_voice::tts::sanitize_text_for_tts(text);
    let text = text.trim();
    if text.is_empty() {
        return Err(error::Error::message("response has no speakable text"));
    }

    let (_, session_override) = state.tts_overrides(session_key, "").await;

    let request = TtsConvertRequest {
        text,
        format: "ogg",
        provider: session_override.as_ref().and_then(|o| o.provider.clone()),
        voice_id: session_override.as_ref().and_then(|o| o.voice_id.clone()),
        model: session_override.as_ref().and_then(|o| o.model.clone()),
    };

    let request_value = serde_json::to_value(request)
        .map_err(|_| error::Error::message("failed to build tts.convert request"))?;
    let tts_result = state
        .tts_service()
        .convert(request_value)
        .await
        .map_err(error::Error::message)?;

    let response: TtsConvertResponse = serde_json::from_value(tts_result)
        .map_err(|_| error::Error::message("invalid tts.convert response"))?;
    base64::engine::general_purpose::STANDARD
        .decode(&response.audio)
        .map_err(|_| error::Error::message("invalid base64 audio returned by TTS provider"))
}

/// Build a voice [`ReplyPayload`] for one channel target, or `None` when TTS is
/// disabled or any step of the conversion fails (the caller then falls back to
/// delivering plain text).
///
/// The channel-level override wins over the session-level one, since a user who
/// configured a voice for, say, WhatsApp expects it regardless of which session
/// the reply belongs to.
pub(crate) async fn build_tts_payload(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    target: &moltis_channels::ChannelReplyTarget,
    text: &str,
) -> Option<moltis_common::types::ReplyPayload> {
    use moltis_common::types::{MediaAttachment, ReplyPayload};

    let tts_status = state.tts_service().status().await.ok()?;
    let status: TtsStatusResponse = serde_json::from_value(tts_status).ok()?;
    if !status.enabled {
        return None;
    }

    // Strip markdown/URLs the LLM may have included — use sanitized text
    // only for TTS conversion, but keep the original for the caption.
    let sanitized = moltis_voice::tts::sanitize_text_for_tts(text);

    let channel_key = format!("{}:{}", target.channel_type.as_str(), target.account_id);
    let (channel_override, session_override) = state.tts_overrides(session_key, &channel_key).await;
    let resolved = channel_override.or(session_override);

    let request = TtsConvertRequest {
        text: &sanitized,
        format: "ogg",
        provider: resolved.as_ref().and_then(|o| o.provider.clone()),
        voice_id: resolved.as_ref().and_then(|o| o.voice_id.clone()),
        model: resolved.as_ref().and_then(|o| o.model.clone()),
    };

    let tts_result = state
        .tts_service()
        .convert(serde_json::to_value(request).ok()?)
        .await
        .ok()?;

    let response: TtsConvertResponse = serde_json::from_value(tts_result).ok()?;

    let mime_type = response
        .mime_type
        .unwrap_or_else(|| "audio/ogg".to_string());

    Some(ReplyPayload {
        text: text.to_string(),
        media: Some(MediaAttachment {
            url: format!("data:{mime_type};base64,{}", response.audio),
            mime_type,
            filename: None,
        }),
        reply_to_id: None,
        silent: false,
    })
}
