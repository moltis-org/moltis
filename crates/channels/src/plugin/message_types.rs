use super::ChannelType;

/// Metadata about a channel message, used for UI display.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ChannelMessageMeta {
    pub channel_type: ChannelType,
    pub sender_name: Option<String>,
    pub username: Option<String>,
    /// Platform-specific sender/peer ID (e.g. Telegram user ID, Discord user ID).
    /// Used for per-sender tool policy resolution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_id: Option<String>,
    /// Original inbound message media kind (voice, audio, photo, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_kind: Option<ChannelMessageKind>,
    /// Default model configured for this channel account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Default agent configured for this channel account or chat override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Filename of saved voice audio (set by `save_channel_voice`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_filename: Option<String>,
    /// Saved inbound documents/files attached to this user message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documents: Option<Vec<ChannelDocumentFile>>,
}

/// Inbound channel message media kind.
#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelMessageKind {
    Text,
    Voice,
    Audio,
    Photo,
    Document,
    Video,
    Location,
    Other,
}

/// An attachment (image, file) from a channel message.
#[derive(Debug, Clone)]
pub struct ChannelAttachment {
    /// MIME type of the attachment (e.g., "image/jpeg", "image/png").
    pub media_type: String,
    /// Raw binary data of the attachment.
    pub data: Vec<u8>,
}

/// Metadata for a saved inbound channel document.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ChannelDocumentFile {
    /// User-facing original filename when available.
    pub display_name: String,
    /// Sanitized stored filename inside session media.
    pub stored_filename: String,
    /// MIME type reported by the channel.
    pub mime_type: String,
    /// Attachment size when the channel exposes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

/// Metadata for an inbound channel file saved to session media.
#[derive(Debug, Clone)]
pub struct SavedChannelFile {
    /// Original or generated filename used in session media storage.
    pub filename: String,
    /// Relative media reference (e.g. `media/main/report.pdf`).
    pub media_ref: String,
    /// Absolute filesystem path for local tooling access.
    pub absolute_path: String,
}
