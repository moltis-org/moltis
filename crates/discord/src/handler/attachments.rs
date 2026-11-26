//! Discord CDN attachment download and attachment classification.

use {
    moltis_channels::{
        Error as ChannelError, InboundMediaDownloader, InboundMediaSource, Result as ChannelsResult,
    },
    moltis_common::{http_client::build_default_http_client, ssrf::ssrf_check},
    serenity::{all::Attachment, async_trait},
};

/// Maximum byte size for inbound Discord attachments we download. Matches the
/// 25 MiB free-tier upload limit; anything larger is either a Discord Nitro
/// upload we don't want to spend bandwidth on or a misbehaving client.
pub(super) const MAX_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;

#[derive(Debug, Clone)]
pub(super) struct DiscordInboundMediaDownloader {
    client: reqwest::Client,
}

impl DiscordInboundMediaDownloader {
    pub(super) fn new() -> Self {
        Self {
            client: build_default_http_client(),
        }
    }
}

/// Returns true if the attachment looks like an audio/voice file.
///
/// A reported type wins over the extension: `.webm` is also a video
/// container, and a `video/webm` clip must be saved, not transcribed.
pub(super) fn attachment_is_audio(a: &Attachment) -> bool {
    match a.content_type.as_deref() {
        Some(ct) if ct.starts_with("audio/") => return true,
        Some(ct) if ct.starts_with("video/") || ct.starts_with("image/") => return false,
        _ => {},
    }
    let name = a.filename.to_ascii_lowercase();
    [".ogg", ".opus", ".mp3", ".wav", ".m4a", ".webm"]
        .iter()
        .any(|ext| name.ends_with(ext))
}

/// Returns true if the attachment looks like a still image.
pub(super) fn attachment_is_image(a: &Attachment) -> bool {
    if a.content_type
        .as_deref()
        .is_some_and(|ct| ct.starts_with("image/"))
    {
        return true;
    }
    let name = a.filename.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".webp"]
        .iter()
        .any(|ext| name.ends_with(ext))
}

/// Returns true for attachments that are neither audio nor a still image:
/// PDFs, text, Office files, archives, video and anything else the agent can
/// only reach as a saved file.
pub(super) fn attachment_is_document(a: &Attachment) -> bool {
    !attachment_is_audio(a) && !attachment_is_image(a)
}

/// Pick the first handleable attachment. Audio wins over image when both are
/// present, matching Telegram's voice-first ordering.
pub(super) fn select_media_attachment(attachments: &[Attachment]) -> Option<&Attachment> {
    if let Some(a) = attachments.iter().find(|a| attachment_is_audio(a)) {
        return Some(a);
    }
    attachments.iter().find(|a| attachment_is_image(a))
}

/// Normalize an audio attachment to a short format string the STT provider
/// understands (`ogg`, `mp3`, `wav`, `m4a`, `webm`).
pub(super) fn audio_format_from_attachment(content_type: Option<&str>, filename: &str) -> String {
    if let Some(ct) = content_type
        && let Some(rest) = ct.strip_prefix("audio/")
    {
        let first = rest.split(';').next().unwrap_or(rest).trim();
        let first = first.strip_prefix("x-").unwrap_or(first);
        return match first {
            "mpeg" | "mp3" => "mp3".to_string(),
            "mp4" | "m4a" => "m4a".to_string(),
            "wav" | "wave" => "wav".to_string(),
            "webm" => "webm".to_string(),
            "ogg" | "opus" | "vorbis" => "ogg".to_string(),
            other if !other.is_empty() => other.to_string(),
            _ => "ogg".to_string(),
        };
    }
    let lower = filename.to_ascii_lowercase();
    for (ext, fmt) in [
        (".mp3", "mp3"),
        (".m4a", "m4a"),
        (".wav", "wav"),
        (".webm", "webm"),
        (".opus", "ogg"),
        (".ogg", "ogg"),
    ] {
        if lower.ends_with(ext) {
            return fmt.to_string();
        }
    }
    // Discord voice messages are OGG Opus; safe default.
    "ogg".to_string()
}

/// Normalize an image attachment to a MIME media type. Used as a fallback when
/// `optimize_for_llm` fails and we must send the original bytes anyway.
pub(super) fn image_media_type_fallback(content_type: Option<&str>, filename: &str) -> String {
    if let Some(ct) = content_type
        && ct.starts_with("image/")
    {
        return ct.split(';').next().unwrap_or(ct).trim().to_string();
    }
    let lower = filename.to_ascii_lowercase();
    if lower.ends_with(".png") {
        "image/png".to_string()
    } else if lower.ends_with(".gif") {
        "image/gif".to_string()
    } else if lower.ends_with(".webp") {
        "image/webp".to_string()
    } else {
        "image/jpeg".to_string()
    }
}

/// Download a Discord CDN attachment, enforcing a size cap.
async fn download_discord_attachment(
    client: &reqwest::Client,
    url: &str,
    max_bytes: usize,
) -> ChannelsResult<Vec<u8>> {
    let parsed_url = reqwest::Url::parse(url).map_err(|error| {
        ChannelError::invalid_input(format!("invalid discord attachment URL: {error}"))
    })?;
    ssrf_check(&parsed_url, &[])
        .await
        .map_err(|error| ChannelError::external("discord attachment ssrf check", error))?;

    let response = client
        .get(parsed_url)
        .send()
        .await
        .map_err(|e| ChannelError::external("discord attachment request", e))?;
    if !response.status().is_success() {
        return Err(ChannelError::unavailable(format!(
            "discord attachment request returned HTTP {}",
            response.status()
        )));
    }
    if let Some(len) = response.content_length()
        && len as usize > max_bytes
    {
        return Err(ChannelError::unavailable(format!(
            "attachment too large: {len} bytes (cap {max_bytes})"
        )));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| ChannelError::external("discord attachment read body", e))?;
    if bytes.len() > max_bytes {
        return Err(ChannelError::unavailable(format!(
            "attachment too large: {} bytes (cap {max_bytes})",
            bytes.len()
        )));
    }
    Ok(bytes.to_vec())
}

#[async_trait]
impl InboundMediaDownloader for DiscordInboundMediaDownloader {
    async fn download_media(
        &self,
        source: &InboundMediaSource,
        max_bytes: usize,
    ) -> ChannelsResult<Vec<u8>> {
        match source {
            InboundMediaSource::RemoteUrl { url } => {
                download_discord_attachment(&self.client, url, max_bytes).await
            },
            _ => Err(ChannelError::invalid_input(
                "discord downloader received unsupported media source",
            )),
        }
    }
}
