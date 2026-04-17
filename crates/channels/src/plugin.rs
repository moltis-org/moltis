use std::sync::Arc;

use {
    async_trait::async_trait,
    moltis_common::{hooks::ChannelBinding, types::ReplyPayload},
    tokio::sync::mpsc,
};

use crate::{Error, Result, config_view::ChannelConfigView};

// ── Channel type enum ───────────────────────────────────────────────────────

/// Supported channel types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum ChannelType {
    Telegram,
    Whatsapp,
    #[serde(rename = "msteams")]
    MsTeams,
    Discord,
    Slack,
    Matrix,
    Nostr,
}

impl ChannelType {
    /// Returns the channel type identifier as a string slice.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Telegram => "telegram",
            Self::Whatsapp => "whatsapp",
            Self::MsTeams => "msteams",
            Self::Discord => "discord",
            Self::Slack => "slack",
            Self::Matrix => "matrix",
            Self::Nostr => "nostr",
        }
    }

    /// Human-readable display name for UI labels.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Telegram => "Telegram",
            Self::Whatsapp => "WhatsApp",
            Self::MsTeams => "Microsoft Teams",
            Self::Discord => "Discord",
            Self::Slack => "Slack",
            Self::Matrix => "Matrix",
            Self::Nostr => "Nostr",
        }
    }

    /// Best-effort chat classification for hook and prompt context.
    #[must_use]
    pub fn classify_chat(&self, chat_id: &str) -> Option<String> {
        match self {
            Self::Telegram => {
                if chat_id.starts_with("-100") {
                    Some("channel_or_supergroup".to_string())
                } else if chat_id.starts_with('-') {
                    Some("group".to_string())
                } else {
                    Some("private".to_string())
                }
            },
            Self::Nostr => Some("dm".to_string()),
            _ => None,
        }
    }

    /// Top-level config fields that must be treated as persisted secrets.
    pub fn secret_fields(&self) -> &'static [&'static str] {
        match self {
            Self::Telegram => &["token"],
            Self::Whatsapp => &[],
            Self::MsTeams => &["app_password", "webhook_secret"],
            Self::Discord => &["token"],
            Self::Slack => &["bot_token", "app_token", "signing_secret"],
            Self::Matrix => &["access_token", "password"],
            Self::Nostr => &["secret_key"],
        }
    }
}

impl std::fmt::Display for ChannelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ChannelType {
    type Err = Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "telegram" => Ok(Self::Telegram),
            "whatsapp" => Ok(Self::Whatsapp),
            "msteams" | "microsoft_teams" | "microsoft-teams" | "teams" => Ok(Self::MsTeams),
            "discord" => Ok(Self::Discord),
            "slack" => Ok(Self::Slack),
            "matrix" | "element" => Ok(Self::Matrix),
            "nostr" => Ok(Self::Nostr),
            other => Err(Error::invalid_input(format!(
                "unknown channel type: {other}"
            ))),
        }
    }
}

impl ChannelType {
    /// All known channel types.
    pub const ALL: &[ChannelType] = &[
        Self::Telegram,
        Self::Whatsapp,
        Self::MsTeams,
        Self::Discord,
        Self::Slack,
        Self::Matrix,
        Self::Nostr,
    ];

    /// Returns the static descriptor for this channel type.
    #[must_use]
    pub fn descriptor(&self) -> ChannelDescriptor {
        match self {
            Self::Telegram => ChannelDescriptor {
                channel_type: *self,
                display_name: "Telegram",
                capabilities: ChannelCapabilities {
                    inbound_mode: InboundMode::Polling,
                    supports_outbound: true,
                    supports_streaming: true,
                    supports_interactive: false,
                    supports_threads: false,
                    supports_voice_ingest: true,
                    supports_pairing: false,
                    supports_otp: true,
                    supports_reactions: false,
                    supports_location: true,
                },
            },
            Self::Whatsapp => ChannelDescriptor {
                channel_type: *self,
                display_name: "WhatsApp",
                capabilities: ChannelCapabilities {
                    inbound_mode: InboundMode::GatewayLoop,
                    supports_outbound: true,
                    supports_streaming: true,
                    supports_interactive: false,
                    supports_threads: false,
                    supports_voice_ingest: true,
                    supports_pairing: true,
                    supports_otp: true,
                    supports_reactions: false,
                    supports_location: false,
                },
            },
            Self::MsTeams => ChannelDescriptor {
                channel_type: *self,
                display_name: "Microsoft Teams",
                capabilities: ChannelCapabilities {
                    inbound_mode: InboundMode::Webhook,
                    supports_outbound: true,
                    supports_streaming: true,
                    supports_interactive: true,
                    supports_threads: true,
                    supports_voice_ingest: false,
                    supports_pairing: false,
                    supports_otp: false,
                    supports_reactions: true,
                    supports_location: true,
                },
            },
            Self::Discord => ChannelDescriptor {
                channel_type: *self,
                display_name: "Discord",
                capabilities: ChannelCapabilities {
                    inbound_mode: InboundMode::GatewayLoop,
                    supports_outbound: true,
                    supports_streaming: true,
                    supports_interactive: true,
                    supports_threads: true,
                    supports_voice_ingest: true,
                    supports_pairing: false,
                    supports_otp: false,
                    supports_reactions: false,
                    supports_location: true,
                },
            },
            Self::Slack => ChannelDescriptor {
                channel_type: *self,
                display_name: "Slack",
                capabilities: ChannelCapabilities {
                    inbound_mode: InboundMode::SocketMode,
                    supports_outbound: true,
                    supports_streaming: true,
                    supports_interactive: true,
                    supports_threads: true,
                    supports_voice_ingest: false,
                    supports_pairing: false,
                    supports_otp: false,
                    supports_reactions: true,
                    supports_location: false,
                },
            },
            Self::Matrix => ChannelDescriptor {
                channel_type: *self,
                display_name: "Matrix",
                capabilities: ChannelCapabilities {
                    inbound_mode: InboundMode::GatewayLoop,
                    supports_outbound: true,
                    supports_streaming: true,
                    supports_interactive: true,
                    supports_threads: true,
                    supports_voice_ingest: true,
                    supports_pairing: false,
                    supports_otp: true,
                    supports_reactions: true,
                    supports_location: true,
                },
            },
            Self::Nostr => ChannelDescriptor {
                channel_type: *self,
                display_name: "Nostr",
                capabilities: ChannelCapabilities {
                    inbound_mode: InboundMode::GatewayLoop,
                    supports_outbound: true,
                    supports_streaming: false,
                    supports_interactive: false,
                    supports_threads: false,
                    supports_voice_ingest: false,
                    supports_pairing: false,
                    supports_otp: true,
                    supports_reactions: false,
                    supports_location: false,
                },
            },
        }
    }
}

// ── Channel capabilities ──────────────────────────────────────────────────

/// How a channel receives inbound messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InboundMode {
    /// Send-only channel with no inbound capability (e.g. email, SMS).
    None,
    /// Long-polling loop (Telegram).
    Polling,
    /// Persistent gateway/WebSocket connection (Discord, WhatsApp).
    GatewayLoop,
    /// Socket Mode connection (Slack).
    SocketMode,
    /// HTTP webhook endpoint (Microsoft Teams).
    Webhook,
}

/// Static capability flags for a channel type.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ChannelCapabilities {
    pub inbound_mode: InboundMode,
    pub supports_outbound: bool,
    pub supports_streaming: bool,
    pub supports_interactive: bool,
    pub supports_threads: bool,
    pub supports_voice_ingest: bool,
    pub supports_pairing: bool,
    pub supports_otp: bool,
    pub supports_reactions: bool,
    pub supports_location: bool,
}

/// Full descriptor for a channel type, including capabilities.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ChannelDescriptor {
    pub channel_type: ChannelType,
    pub display_name: &'static str,
    pub capabilities: ChannelCapabilities,
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
    /// A reaction was added or removed on a channel message.
    ReactionChange {
        channel_type: ChannelType,
        account_id: String,
        chat_id: String,
        message_id: String,
        user_id: String,
        emoji: String,
        added: bool,
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
    /// A QR code was generated for device pairing (e.g. WhatsApp Linked Devices).
    PairingQrCode {
        channel_type: ChannelType,
        account_id: String,
        /// Raw QR data string to be rendered as a QR code image.
        qr_data: String,
    },
    /// Device pairing completed successfully.
    PairingComplete {
        channel_type: ChannelType,
        account_id: String,
        /// Display name of the paired device/account.
        display_name: Option<String>,
    },
    /// Device pairing failed.
    PairingFailed {
        channel_type: ChannelType,
        account_id: String,
        reason: String,
    },
    /// Channel account status changed (e.g. ownership bootstrap complete).
    StatusChanged {
        channel_type: ChannelType,
        account_id: String,
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
    ///
    /// `sender_id` identifies the message sender. Privileged commands
    /// (`/approve`, `/deny`) are restricted to senders on the channel
    /// account's allowlist — authorization is enforced centrally by the
    /// gateway, so channel implementations do not need to handle it.
    async fn dispatch_command(
        &self,
        command: &str,
        reply_to: ChannelReplyTarget,
        sender_id: Option<&str>,
    ) -> Result<String>;

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

    /// Save voice audio bytes to the session's media directory.
    ///
    /// Returns the saved filename on success, or `None` if saving is not
    /// available or fails. The gateway implementation resolves the session
    /// key from the reply target and delegates to `SessionStore::save_media`.
    async fn save_channel_voice(
        &self,
        _audio_data: &[u8],
        _filename: &str,
        _reply_to: &ChannelReplyTarget,
    ) -> Option<String> {
        None
    }

    /// Save a non-audio inbound file to the session's media directory.
    ///
    /// Returns both the relative media reference and the absolute local path
    /// on success so the agent can inspect the exact saved file.
    async fn save_channel_attachment(
        &self,
        _file_data: &[u8],
        _filename: &str,
        _reply_to: &ChannelReplyTarget,
    ) -> Option<SavedChannelFile> {
        None
    }

    /// Transcribe voice audio to text using the configured STT provider.
    ///
    /// Returns the transcribed text, or an error if transcription fails.
    /// The audio format is specified (e.g., "ogg", "mp3", "webm").
    async fn transcribe_voice(&self, audio_data: &[u8], format: &str) -> Result<String> {
        let _ = (audio_data, format);
        Err(Error::unavailable("voice transcription not available"))
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

    /// Resolve a pending tool-triggered location request from channel text/link input.
    ///
    /// Unlike `update_location`, this should not update cached location state
    /// when there is no pending request. Returns `true` only when a pending
    /// request was found and resolved.
    async fn resolve_pending_location(
        &self,
        _reply_to: &ChannelReplyTarget,
        _latitude: f64,
        _longitude: f64,
    ) -> bool {
        false
    }

    /// Dispatch a button/menu interaction callback.
    ///
    /// Returns a response message to send back to the user.
    async fn dispatch_interaction(
        &self,
        _callback_data: &str,
        _reply_to: ChannelReplyTarget,
    ) -> Result<String> {
        Err(Error::unavailable("interactions not supported"))
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
    /// Forum-topic / thread identifier (e.g. Telegram `message_thread_id`).
    /// When present, outbound messages are routed to this topic instead of the
    /// top-level chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

impl ChannelReplyTarget {
    /// Returns the address string for outbound sends.
    ///
    /// For Telegram forum topics this encodes both chat and thread as
    /// `"chat_id:thread_id"` so the outbound implementation can route to the
    /// correct topic. All other channels return the plain `chat_id`.
    pub fn outbound_to(&self) -> std::borrow::Cow<'_, str> {
        match &self.thread_id {
            Some(tid) => std::borrow::Cow::Owned(format!("{}:{}", self.chat_id, tid)),
            None => std::borrow::Cow::Borrowed(&self.chat_id),
        }
    }
}

impl From<&ChannelReplyTarget> for ChannelBinding {
    fn from(target: &ChannelReplyTarget) -> Self {
        let channel_type = target.channel_type.as_str().to_string();
        Self {
            surface: Some(channel_type.clone()),
            session_kind: Some("channel".to_string()),
            channel_type: Some(channel_type),
            account_id: Some(target.account_id.clone()),
            chat_id: Some(target.chat_id.clone()),
            chat_type: target.channel_type.classify_chat(&target.chat_id),
            sender_id: None,
        }
    }
}

#[must_use]
pub fn web_session_channel_binding() -> ChannelBinding {
    ChannelBinding {
        surface: Some("web".to_string()),
        session_kind: Some("web".to_string()),
        ..Default::default()
    }
}

pub fn resolve_session_channel_binding(
    session_key: &str,
    binding_json: Option<&str>,
) -> std::result::Result<ChannelBinding, serde_json::Error> {
    if session_key == "cron:heartbeat" {
        return Ok(ChannelBinding {
            surface: Some("heartbeat".to_string()),
            session_kind: Some("cron".to_string()),
            ..Default::default()
        });
    }

    if session_key.starts_with("cron:") {
        return Ok(ChannelBinding {
            surface: Some("cron".to_string()),
            session_kind: Some("cron".to_string()),
            ..Default::default()
        });
    }

    if let Some(binding_json) = binding_json {
        let binding = serde_json::from_str::<ChannelReplyTarget>(binding_json)?;
        return Ok((&binding).into());
    }

    Ok(web_session_channel_binding())
}

// ── Interactive messages ─────────────────────────────────────────────────────

/// A clickable button in a channel message.
#[derive(Debug, Clone)]
pub struct InteractiveButton {
    pub label: String,
    pub callback_data: String,
    pub style: ButtonStyle,
}

/// Visual style for interactive buttons.
#[derive(Debug, Clone, Default)]
pub enum ButtonStyle {
    #[default]
    Default,
    Primary,
    Danger,
}

/// A row of buttons.
pub type ButtonRow = Vec<InteractiveButton>;

/// A message with interactive button components.
#[derive(Debug, Clone)]
pub struct InteractiveMessage {
    pub text: String,
    pub button_rows: Vec<ButtonRow>,
    pub replace_message_id: Option<String>,
}

// ── Thread context ──────────────────────────────────────────────────────────

/// A single message from a thread conversation.
#[derive(Debug, Clone)]
pub struct ThreadMessage {
    pub sender_id: String,
    pub is_bot: bool,
    pub text: String,
    pub timestamp: String,
}

/// Fetch prior thread messages for context injection.
#[async_trait]
pub trait ChannelThreadContext: Send + Sync {
    /// Fetch up to `limit` messages from the given thread.
    async fn fetch_thread_messages(
        &self,
        account_id: &str,
        channel_id: &str,
        thread_id: &str,
        limit: usize,
    ) -> Result<Vec<ThreadMessage>>;
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

    /// Retry account-specific setup that is waiting on some external action.
    ///
    /// Most channels do not need this. Matrix uses it to resume a pending
    /// browser-approved cross-signing reset without tearing down the account.
    async fn retry_account_setup(&mut self, _account_id: &str) -> Result<()> {
        Err(Error::unavailable("account setup retry not supported"))
    }

    /// Get outbound adapter for sending messages.
    fn outbound(&self) -> Option<&dyn ChannelOutbound>;

    /// Get status adapter for health checks.
    fn status(&self) -> Option<&dyn ChannelStatus>;

    /// Whether the given account is currently active.
    fn has_account(&self, account_id: &str) -> bool;

    /// List all active account IDs.
    fn account_ids(&self) -> Vec<String>;

    /// Get the typed config view for a specific account.
    fn account_config(&self, account_id: &str) -> Option<Box<dyn ChannelConfigView>>;

    /// Update the in-memory config for an account without restarting.
    ///
    /// Accepts raw JSON because the store persists `Value`. Each plugin
    /// deserializes into its concrete config type internally.
    fn update_account_config(&self, account_id: &str, config: serde_json::Value) -> Result<()>;

    /// Get a shared outbound sender for routing outside the plugin.
    fn shared_outbound(&self) -> Arc<dyn ChannelOutbound>;

    /// Get a shared streaming outbound sender for routing outside the plugin.
    fn shared_stream_outbound(&self) -> Arc<dyn ChannelStreamOutbound>;

    /// Get the raw JSON config for an account (for API status responses).
    ///
    /// Each plugin serializes its concrete config type. Returns `None` if the
    /// account is not found.
    fn account_config_json(&self, _account_id: &str) -> Option<serde_json::Value> {
        None
    }

    /// Downcast to OTP provider if this channel supports OTP self-approval.
    fn as_otp_provider(&self) -> Option<&dyn ChannelOtpProvider> {
        None
    }

    /// Thread context provider for fetching prior thread messages.
    fn thread_context(&self) -> Option<&dyn ChannelThreadContext> {
        None
    }

    /// Return the webhook verifier for this channel account, if this channel
    /// uses HTTP webhooks. Channels that use polling/socket modes return `None`.
    fn channel_webhook_verifier(
        &self,
        _account_id: &str,
    ) -> Option<Box<dyn crate::channel_webhook_middleware::ChannelWebhookVerifier>> {
        None
    }

    /// Start an OAuth/OIDC login flow. Returns auth URL and CSRF state.
    async fn oidc_start(
        &self,
        _account_id: &str,
        _config: serde_json::Value,
        _redirect_uri: &str,
    ) -> Result<serde_json::Value> {
        Err(Error::unavailable(
            "OIDC login not supported for this channel",
        ))
    }

    /// Complete an OAuth/OIDC login after browser redirect.
    async fn oidc_complete(
        &self,
        _csrf_state: &str,
        _callback_url: &str,
    ) -> Result<serde_json::Value> {
        Err(Error::unavailable(
            "OIDC login not supported for this channel",
        ))
    }
}

/// OTP challenge provider for channels that support self-approval.
pub trait ChannelOtpProvider: Send + Sync {
    /// List pending OTP challenges for the given account.
    fn pending_otp_challenges(&self, account_id: &str) -> Vec<crate::otp::OtpChallengeInfo>;
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
    /// Send a text message with a pre-formatted HTML suffix appended after the main
    /// content. Used to attach a collapsible activity logbook to channel replies.
    /// The default implementation ignores the suffix and calls `send_text`.
    async fn send_text_with_suffix(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        suffix_html: &str,
        reply_to: Option<&str>,
    ) -> Result<()> {
        let _ = suffix_html;
        self.send_text(account_id, to, text, reply_to).await
    }
    /// Send pre-formatted HTML without markdown conversion.
    ///
    /// Used for content that is already valid Telegram HTML (e.g. the activity
    /// logbook with `<blockquote>` tags).  Default falls back to `send_text`.
    async fn send_html(
        &self,
        account_id: &str,
        to: &str,
        html: &str,
        reply_to: Option<&str>,
    ) -> Result<()> {
        self.send_text(account_id, to, html, reply_to).await
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
    /// Send an interactive message with buttons. Default: numbered text fallback.
    async fn send_interactive(
        &self,
        account_id: &str,
        to: &str,
        message: &InteractiveMessage,
        reply_to: Option<&str>,
    ) -> Result<()> {
        // Default implementation: render buttons as numbered text lines.
        let mut text = message.text.clone();
        let mut idx = 1;
        for row in &message.button_rows {
            for btn in row {
                text.push_str(&format!("\n{idx}. {}", btn.label));
                idx += 1;
            }
        }
        self.send_text(account_id, to, &text, reply_to).await
    }

    /// Add a reaction (emoji) to a message. No-op by default.
    async fn add_reaction(
        &self,
        _account_id: &str,
        _channel_id: &str,
        _message_id: &str,
        _emoji: &str,
    ) -> Result<()> {
        Ok(())
    }

    /// Remove a reaction (emoji) from a message. No-op by default.
    async fn remove_reaction(
        &self,
        _account_id: &str,
        _channel_id: &str,
        _message_id: &str,
        _emoji: &str,
    ) -> Result<()> {
        Ok(())
    }

    /// Send a native location pin to the channel.
    ///
    /// When `title` is provided, platforms that support it (e.g. Telegram) send
    /// a venue with the place name visible in the chat bubble. Otherwise a raw
    /// location pin is sent.
    ///
    /// Default implementation is a no-op so channels that don't support native
    /// location pins are unaffected.
    async fn send_location(
        &self,
        account_id: &str,
        to: &str,
        latitude: f64,
        longitude: f64,
        title: Option<&str>,
        reply_to: Option<&str>,
    ) -> Result<()> {
        let _ = (account_id, to, latitude, longitude, title, reply_to);
        Ok(())
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
    pub extra: Option<serde_json::Value>,
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
    async fn send_stream(
        &self,
        account_id: &str,
        to: &str,
        reply_to: Option<&str>,
        stream: StreamReceiver,
    ) -> Result<()>;

    /// Whether streaming is enabled for this account.
    async fn is_stream_enabled(&self, _account_id: &str) -> bool {
        true
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
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
            _sender_id: Option<&str>,
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
            thread_id: None,
        };
        assert!(!sink.update_location(&target, 48.8566, 2.3522).await);
    }

    #[test]
    fn outbound_to_without_thread_id() {
        let target = ChannelReplyTarget {
            channel_type: ChannelType::Telegram,
            account_id: "bot1".into(),
            chat_id: "12345".into(),
            message_id: None,
            thread_id: None,
        };
        assert_eq!(target.outbound_to().as_ref(), "12345");
    }

    #[test]
    fn outbound_to_with_thread_id() {
        let target = ChannelReplyTarget {
            channel_type: ChannelType::Telegram,
            account_id: "bot1".into(),
            chat_id: "-100999".into(),
            message_id: None,
            thread_id: Some("42".into()),
        };
        assert_eq!(target.outbound_to().as_ref(), "-100999:42");
    }

    #[test]
    fn reply_target_thread_id_serde_roundtrip() {
        let target = ChannelReplyTarget {
            channel_type: ChannelType::Telegram,
            account_id: "bot1".into(),
            chat_id: "-100999".into(),
            message_id: None,
            thread_id: Some("42".into()),
        };
        let json = serde_json::to_string(&target).unwrap();
        assert!(json.contains("\"thread_id\":\"42\""));
        let restored: ChannelReplyTarget = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.thread_id.as_deref(), Some("42"));
    }

    #[test]
    fn reply_target_without_thread_id_deserializes() {
        // Existing JSON without thread_id should deserialize with None.
        let json = r#"{"channel_type":"telegram","account_id":"bot1","chat_id":"123"}"#;
        let target: ChannelReplyTarget = serde_json::from_str(json).unwrap();
        assert!(target.thread_id.is_none());
    }

    #[test]
    fn channel_type_whatsapp_roundtrip() {
        let ct = ChannelType::Whatsapp;
        assert_eq!(ct.as_str(), "whatsapp");
        assert_eq!(ct.to_string(), "whatsapp");
        assert_eq!("whatsapp".parse::<ChannelType>().unwrap(), ct);
    }

    #[test]
    fn channel_type_serde_roundtrip() {
        for ct in [
            ChannelType::Telegram,
            ChannelType::Whatsapp,
            ChannelType::MsTeams,
            ChannelType::Discord,
            ChannelType::Slack,
        ] {
            let json = serde_json::to_string(&ct).unwrap();
            let parsed: ChannelType = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, ct);
        }
    }

    #[test]
    fn channel_type_discord_roundtrip() {
        let ct = ChannelType::Discord;
        assert_eq!(ct.as_str(), "discord");
        assert_eq!(ct.to_string(), "discord");
        assert_eq!("discord".parse::<ChannelType>().unwrap(), ct);
    }

    #[test]
    fn pairing_qr_code_event_serialization() {
        let event = ChannelEvent::PairingQrCode {
            channel_type: ChannelType::Whatsapp,
            account_id: "wa1".into(),
            qr_data: "2@abc123".into(),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["kind"], "pairing_qr_code");
        assert_eq!(json["channel_type"], "whatsapp");
        assert_eq!(json["account_id"], "wa1");
        assert_eq!(json["qr_data"], "2@abc123");
    }

    #[test]
    fn pairing_complete_event_serialization() {
        let event = ChannelEvent::PairingComplete {
            channel_type: ChannelType::Whatsapp,
            account_id: "wa1".into(),
            display_name: Some("My Phone".into()),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["kind"], "pairing_complete");
        assert_eq!(json["display_name"], "My Phone");
    }

    #[test]
    fn pairing_failed_event_serialization() {
        let event = ChannelEvent::PairingFailed {
            channel_type: ChannelType::Whatsapp,
            account_id: "wa1".into(),
            reason: "timeout".into(),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["kind"], "pairing_failed");
        assert_eq!(json["reason"], "timeout");
    }

    struct DummyOutbound;

    #[async_trait]
    impl ChannelOutbound for DummyOutbound {
        async fn send_text(
            &self,
            _account_id: &str,
            _to: &str,
            _text: &str,
            _reply_to: Option<&str>,
        ) -> Result<()> {
            Ok(())
        }

        async fn send_media(
            &self,
            _account_id: &str,
            _to: &str,
            _payload: &ReplyPayload,
            _reply_to: Option<&str>,
        ) -> Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn default_send_location_is_noop() {
        let out = DummyOutbound;
        let result = out
            .send_location("acct", "42", 48.8566, 2.3522, Some("Eiffel Tower"), None)
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn default_add_reaction_is_noop() {
        let out = DummyOutbound;
        let result = out
            .add_reaction("acct", "C123", "1234.5678", "thumbsup")
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn default_remove_reaction_is_noop() {
        let out = DummyOutbound;
        let result = out
            .remove_reaction("acct", "C123", "1234.5678", "thumbsup")
            .await;
        assert!(result.is_ok());
    }

    #[test]
    fn reaction_change_event_serialization() {
        let event = ChannelEvent::ReactionChange {
            channel_type: ChannelType::Slack,
            account_id: "slack1".into(),
            chat_id: "C123".into(),
            message_id: "1234.5678".into(),
            user_id: "U456".into(),
            emoji: "thumbsup".into(),
            added: true,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["kind"], "reaction_change");
        assert_eq!(json["channel_type"], "slack");
        assert_eq!(json["emoji"], "thumbsup");
        assert_eq!(json["added"], true);
    }

    #[test]
    fn channel_type_round_trip() {
        for (s, expected) in [
            ("telegram", ChannelType::Telegram),
            ("whatsapp", ChannelType::Whatsapp),
            ("msteams", ChannelType::MsTeams),
            ("discord", ChannelType::Discord),
            ("slack", ChannelType::Slack),
            ("matrix", ChannelType::Matrix),
        ] {
            let parsed: ChannelType = s.parse().unwrap_or_else(|e| panic!("parse {s}: {e}"));
            assert_eq!(parsed, expected);
            assert_eq!(parsed.as_str(), s);
            assert_eq!(parsed.to_string(), s);
        }
    }

    #[test]
    fn channel_type_from_str_invalid() {
        assert!("foobar".parse::<ChannelType>().is_err());
        assert!("".parse::<ChannelType>().is_err());
    }

    #[test]
    fn channel_type_serde_round_trip() {
        for ct in [
            ChannelType::Telegram,
            ChannelType::Whatsapp,
            ChannelType::MsTeams,
            ChannelType::Discord,
            ChannelType::Slack,
            ChannelType::Matrix,
        ] {
            let json = serde_json::to_string(&ct).unwrap_or_else(|e| panic!("serialize: {e}"));
            let back: ChannelType =
                serde_json::from_str(&json).unwrap_or_else(|e| panic!("deserialize: {e}"));
            assert_eq!(ct, back);
        }
    }

    #[test]
    fn all_covers_every_variant() {
        // If a new variant is added to ChannelType, this test forces updating ALL.
        assert_eq!(ChannelType::ALL.len(), 7);
        for ct in ChannelType::ALL {
            // descriptor() must not panic
            let desc = ct.descriptor();
            assert_eq!(desc.channel_type, *ct);
        }
    }

    #[test]
    fn descriptor_returns_correct_display_names() {
        assert_eq!(ChannelType::Telegram.descriptor().display_name, "Telegram");
        assert_eq!(ChannelType::Whatsapp.descriptor().display_name, "WhatsApp");
        assert_eq!(
            ChannelType::MsTeams.descriptor().display_name,
            "Microsoft Teams"
        );
        assert_eq!(ChannelType::Discord.descriptor().display_name, "Discord");
        assert_eq!(ChannelType::Slack.descriptor().display_name, "Slack");
        assert_eq!(ChannelType::Matrix.descriptor().display_name, "Matrix");
        assert_eq!(ChannelType::Nostr.descriptor().display_name, "Nostr");
    }

    #[test]
    fn descriptor_channel_type_matches() {
        for ct in ChannelType::ALL {
            let desc = ct.descriptor();
            assert_eq!(
                desc.channel_type, *ct,
                "descriptor channel_type mismatch for {ct}"
            );
            assert_eq!(desc.display_name, ct.display_name());
        }
    }

    #[test]
    fn channel_type_secret_fields_are_declared() {
        assert_eq!(ChannelType::Telegram.secret_fields(), ["token"]);
        assert_eq!(ChannelType::Whatsapp.secret_fields(), &[] as &[&str]);
        assert_eq!(ChannelType::MsTeams.secret_fields(), [
            "app_password",
            "webhook_secret"
        ]);
        assert_eq!(ChannelType::Discord.secret_fields(), ["token"]);
        assert_eq!(ChannelType::Slack.secret_fields(), [
            "bot_token",
            "app_token",
            "signing_secret"
        ]);
        assert_eq!(ChannelType::Matrix.secret_fields(), [
            "access_token",
            "password"
        ]);
    }

    #[test]
    fn descriptor_serialization_does_not_panic() {
        for ct in ChannelType::ALL {
            let desc = ct.descriptor();
            let json = serde_json::to_value(&desc)
                .unwrap_or_else(|e| panic!("serialize descriptor for {ct}: {e}"));
            assert_eq!(json["channel_type"], ct.as_str());
            assert!(json["capabilities"]["inbound_mode"].is_string());
        }
    }

    #[test]
    fn inbound_mode_serialization() {
        let json = serde_json::to_string(&InboundMode::None).unwrap();
        assert_eq!(json, "\"none\"");
        let json = serde_json::to_string(&InboundMode::Polling).unwrap();
        assert_eq!(json, "\"polling\"");
        let json = serde_json::to_string(&InboundMode::GatewayLoop).unwrap();
        assert_eq!(json, "\"gateway_loop\"");
        let json = serde_json::to_string(&InboundMode::SocketMode).unwrap();
        assert_eq!(json, "\"socket_mode\"");
        let json = serde_json::to_string(&InboundMode::Webhook).unwrap();
        assert_eq!(json, "\"webhook\"");
    }

    #[test]
    fn telegram_chat_classification_matches_chat_id_shape() {
        assert_eq!(
            ChannelType::Telegram.classify_chat("-100123").as_deref(),
            Some("channel_or_supergroup")
        );
        assert_eq!(
            ChannelType::Telegram.classify_chat("-42").as_deref(),
            Some("group")
        );
        assert_eq!(
            ChannelType::Telegram.classify_chat("123").as_deref(),
            Some("private")
        );
        assert!(ChannelType::Discord.classify_chat("123").is_none());
    }

    #[test]
    fn channel_reply_target_converts_to_hook_channel_binding() {
        let target = ChannelReplyTarget {
            channel_type: ChannelType::Telegram,
            account_id: "bot1".into(),
            chat_id: "-100999".into(),
            message_id: Some("7".into()),
            thread_id: Some("42".into()),
        };

        let binding: ChannelBinding = (&target).into();
        assert_eq!(binding.surface.as_deref(), Some("telegram"));
        assert_eq!(binding.session_kind.as_deref(), Some("channel"));
        assert_eq!(binding.channel_type.as_deref(), Some("telegram"));
        assert_eq!(binding.account_id.as_deref(), Some("bot1"));
        assert_eq!(binding.chat_id.as_deref(), Some("-100999"));
        assert_eq!(binding.chat_type.as_deref(), Some("channel_or_supergroup"));
        assert!(binding.sender_id.is_none());
    }

    #[test]
    fn resolve_session_channel_binding_classifies_special_sessions() {
        let heartbeat = resolve_session_channel_binding("cron:heartbeat", None)
            .unwrap_or_else(|error| panic!("heartbeat binding should resolve: {error}"));
        assert_eq!(heartbeat.surface.as_deref(), Some("heartbeat"));
        assert_eq!(heartbeat.session_kind.as_deref(), Some("cron"));

        let cron = resolve_session_channel_binding("cron:nightly", None)
            .unwrap_or_else(|error| panic!("cron binding should resolve: {error}"));
        assert_eq!(cron.surface.as_deref(), Some("cron"));
        assert_eq!(cron.session_kind.as_deref(), Some("cron"));

        let web = resolve_session_channel_binding("main", None)
            .unwrap_or_else(|error| panic!("web binding should resolve: {error}"));
        assert_eq!(web.surface.as_deref(), Some("web"));
        assert_eq!(web.session_kind.as_deref(), Some("web"));
    }

    #[test]
    fn resolve_session_channel_binding_extracts_channel_target() {
        let binding_json = serde_json::to_string(&ChannelReplyTarget {
            channel_type: ChannelType::Telegram,
            account_id: "bot-main".into(),
            chat_id: "-100123".into(),
            message_id: Some("11".into()),
            thread_id: None,
        })
        .unwrap_or_else(|error| panic!("serialize binding: {error}"));

        let binding =
            resolve_session_channel_binding("telegram:bot-main:-100123", Some(&binding_json))
                .unwrap_or_else(|error| panic!("channel binding should resolve: {error}"));

        assert_eq!(binding.surface.as_deref(), Some("telegram"));
        assert_eq!(binding.session_kind.as_deref(), Some("channel"));
        assert_eq!(binding.channel_type.as_deref(), Some("telegram"));
        assert_eq!(binding.account_id.as_deref(), Some("bot-main"));
        assert_eq!(binding.chat_id.as_deref(), Some("-100123"));
        assert_eq!(binding.chat_type.as_deref(), Some("channel_or_supergroup"));
    }

    #[test]
    fn resolve_session_channel_binding_returns_error_for_invalid_json() {
        let error = resolve_session_channel_binding("telegram:bot-main:-100123", Some("{not-json"))
            .err()
            .unwrap_or_else(|| panic!("invalid binding json should fail"));
        assert!(error.is_syntax());
    }
}
