use {
    anyhow::Result, async_trait::async_trait, moltis_common::types::ReplyPayload, tokio::sync::mpsc,
};

// ── Channel type enum ───────────────────────────────────────────────────────

/// Supported channel types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelType {
    Telegram,
    // Future: Discord, Slack, WhatsApp, etc.
}

impl ChannelType {
    /// Returns the channel type identifier as a string slice.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Telegram => "telegram",
        }
    }
}

impl std::fmt::Display for ChannelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ChannelType {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "telegram" => Ok(Self::Telegram),
            other => Err(format!("unknown channel type: {other}")),
        }
    }
}

// ── Channel events (pub/sub) ────────────────────────────────────────────────

/// Events emitted by channel plugins for real-time UI updates.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChannelEvent {
    InboundMessage {
        channel_type: ChannelType,
        account_id: String,
        peer_id: String,
        username: Option<String>,
        sender_name: Option<String>,
        message_count: Option<i64>,
        access_granted: bool,
    },
    /// A channel account was automatically disabled due to a runtime error.
    AccountDisabled {
        channel_type: ChannelType,
        account_id: String,
        reason: String,
    },
    /// An OTP challenge was issued to a non-allowlisted DM user.
    OtpChallenge {
        channel_type: ChannelType,
        account_id: String,
        peer_id: String,
        username: Option<String>,
        sender_name: Option<String>,
        code: String,
        expires_at: i64,
    },
    /// An OTP challenge was resolved (approved, locked out, or expired).
    OtpResolved {
        channel_type: ChannelType,
        account_id: String,
        peer_id: String,
        username: Option<String>,
        resolution: String,
    },
}

/// Sink for channel events — the gateway provides the concrete implementation.
#[async_trait]
pub trait ChannelEventSink: Send + Sync {
    /// Broadcast a channel event for real-time UI updates.
    async fn emit(&self, event: ChannelEvent);

    /// Dispatch an inbound message to the main chat session (like sending
    /// from the web UI). The response is broadcast over WebSocket and
    /// routed back to the originating channel.
    async fn dispatch_to_chat(
        &self,
        text: &str,
        reply_to: ChannelReplyTarget,
        meta: ChannelMessageMeta,
    );

    /// Dispatch a slash command (e.g. "new", "clear", "compact", "context")
    /// and return a text result to send back to the channel.
    async fn dispatch_command(
        &self,
        command: &str,
        reply_to: ChannelReplyTarget,
    ) -> anyhow::Result<String>;

    /// Request disabling a channel account due to a runtime error.
    ///
    /// This is used when the polling loop detects an unrecoverable error
    /// (e.g. another bot instance is running with the same token).
    async fn request_disable_account(&self, channel_type: &str, account_id: &str, reason: &str);

    /// Request adding a sender to the allowlist (OTP self-approval).
    ///
    /// The gateway implementation calls `sender_approve` to persist the change
    /// and restart the account.
    async fn request_sender_approval(
        &self,
        _channel_type: &str,
        _account_id: &str,
        _identifier: &str,
    ) {
    }

    /// Transcribe voice audio to text using the configured STT provider.
    ///
    /// Returns the transcribed text, or an error if transcription fails.
    /// The audio format is specified (e.g., "ogg", "mp3", "webm").
    async fn transcribe_voice(&self, audio_data: &[u8], format: &str) -> Result<String> {
        let _ = (audio_data, format);
        Err(anyhow::anyhow!("voice transcription not available"))
    }

    /// Whether voice STT is configured and available for channel audio messages.
    async fn voice_stt_available(&self) -> bool {
        true
    }

    /// Update the user's geolocation from a channel message (e.g. Telegram location share).
    ///
    /// Returns `true` if a pending tool-triggered location request was resolved.
    async fn update_location(
        &self,
        _reply_to: &ChannelReplyTarget,
        _latitude: f64,
        _longitude: f64,
    ) -> bool {
        false
    }

    /// Dispatch an inbound message with attachments (images, files) to the chat session.
    ///
    /// This is used when a channel message contains both text and media (e.g., a
    /// Telegram photo with a caption). The attachments are sent to the LLM as
    /// multimodal content.
    async fn dispatch_to_chat_with_attachments(
        &self,
        text: &str,
        attachments: Vec<ChannelAttachment>,
        reply_to: ChannelReplyTarget,
        meta: ChannelMessageMeta,
    ) {
        // Default implementation ignores attachments and just sends text.
        let _ = attachments;
        self.dispatch_to_chat(text, reply_to, meta).await;
    }
}

/// Metadata about a channel message, used for UI display.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ChannelMessageMeta {
    pub channel_type: ChannelType,
    pub sender_name: Option<String>,
    pub username: Option<String>,
    /// Original inbound message media kind (voice, audio, photo, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_kind: Option<ChannelMessageKind>,
    /// Default model configured for this channel account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
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

/// Where to send the LLM response back.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChannelReplyTarget {
    pub channel_type: ChannelType,
    pub account_id: String,
    /// Chat/peer ID to send the reply to.
    pub chat_id: String,
    /// Platform-specific message ID of the inbound message.
    /// Used to thread replies (e.g. Telegram `reply_to_message_id`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
}

/// Core channel plugin trait. Each messaging platform implements this.
#[async_trait]
pub trait ChannelPlugin: Send + Sync {
    /// Channel identifier (e.g. "telegram", "discord").
    fn id(&self) -> &str;

    /// Human-readable channel name.
    fn name(&self) -> &str;

    /// Start an account connection.
    async fn start_account(&mut self, account_id: &str, config: serde_json::Value) -> Result<()>;

    /// Stop an account connection.
    async fn stop_account(&mut self, account_id: &str) -> Result<()>;

    /// Get outbound adapter for sending messages.
    fn outbound(&self) -> Option<&dyn ChannelOutbound>;

    /// Get status adapter for health checks.
    fn status(&self) -> Option<&dyn ChannelStatus>;
}

/// Send messages to a channel.
///
/// `reply_to` is an optional platform-specific message ID that the outbound
/// message should thread as a reply to (e.g. Telegram `reply_to_message_id`).
#[async_trait]
pub trait ChannelOutbound: Send + Sync {
    async fn send_text(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> Result<()>;
    async fn send_media(
        &self,
        account_id: &str,
        to: &str,
        payload: &ReplyPayload,
        reply_to: Option<&str>,
    ) -> Result<()>;
    /// Send a "typing" indicator. No-op by default.
    async fn send_typing(&self, _account_id: &str, _to: &str) -> Result<()> {
        Ok(())
    }
    /// Send a text message without notification (silent). Falls back to send_text by default.
    async fn send_text_silent(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> Result<()> {
        self.send_text(account_id, to, text, reply_to).await
    }
}

/// Probe channel account health.
#[async_trait]
pub trait ChannelStatus: Send + Sync {
    async fn probe(&self, account_id: &str) -> Result<ChannelHealthSnapshot>;
}

/// Channel health snapshot.
#[derive(Debug, Clone)]
pub struct ChannelHealthSnapshot {
    pub connected: bool,
    pub account_id: String,
    pub details: Option<String>,
}

/// Stream event for edit-in-place streaming.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// A chunk of text to append.
    Delta(String),
    /// Stream is complete.
    Done,
    /// An error occurred.
    Error(String),
}

/// Receiver end of a stream channel.
pub type StreamReceiver = mpsc::Receiver<StreamEvent>;

/// Sender end of a stream channel.
pub type StreamSender = mpsc::Sender<StreamEvent>;

/// Streaming outbound — send responses via edit-in-place updates.
#[async_trait]
pub trait ChannelStreamOutbound: Send + Sync {
    /// Send a streaming response that updates a message in place.
    async fn send_stream(&self, account_id: &str, to: &str, stream: StreamReceiver) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummySink;

    #[async_trait]
    impl ChannelEventSink for DummySink {
        async fn emit(&self, _event: ChannelEvent) {}

        async fn dispatch_to_chat(
            &self,
            _text: &str,
            _reply_to: ChannelReplyTarget,
            _meta: ChannelMessageMeta,
        ) {
        }

        async fn dispatch_command(
            &self,
            _command: &str,
            _reply_to: ChannelReplyTarget,
        ) -> anyhow::Result<String> {
            Ok(String::new())
        }

        async fn request_disable_account(
            &self,
            _channel_type: &str,
            _account_id: &str,
            _reason: &str,
        ) {
        }
    }

    #[tokio::test]
    async fn default_voice_stt_available_is_true() {
        let sink = DummySink;
        assert!(sink.voice_stt_available().await);
    }

    #[tokio::test]
    async fn default_update_location_returns_false() {
        let sink = DummySink;
        let target = ChannelReplyTarget {
            channel_type: ChannelType::Telegram,
            account_id: "bot1".into(),
            chat_id: "42".into(),
            message_id: None,
        };
        assert!(!sink.update_location(&target, 48.8566, 2.3522).await);
    }
}
