use {
    anyhow::Result, async_trait::async_trait, moltis_common::types::ReplyPayload, tokio::sync::mpsc,
};

// ── Channel events (pub/sub) ────────────────────────────────────────────────

/// Events emitted by channel plugins for real-time UI updates.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChannelEvent {
    InboundMessage {
        channel_type: String,
        account_id: String,
        peer_id: String,
        username: Option<String>,
        sender_name: Option<String>,
        message_count: Option<i64>,
        access_granted: bool,
    },
    /// A channel account was automatically disabled due to a runtime error.
    AccountDisabled {
        channel_type: String,
        account_id: String,
        reason: String,
    },
    /// An OTP challenge was issued to a non-allowlisted DM user.
    OtpChallenge {
        channel_type: String,
        account_id: String,
        peer_id: String,
        username: Option<String>,
        sender_name: Option<String>,
        code: String,
        expires_at: i64,
    },
    /// An OTP challenge was resolved (approved, locked out, or expired).
    OtpResolved {
        channel_type: String,
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
}

/// Metadata about a channel message, used for UI display.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ChannelMessageMeta {
    pub channel_type: String,
    pub sender_name: Option<String>,
    pub username: Option<String>,
    /// Default model configured for this channel account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Where to send the LLM response back.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChannelReplyTarget {
    pub channel_type: String,
    pub account_id: String,
    /// Chat/peer ID to send the reply to.
    pub chat_id: String,
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
#[async_trait]
pub trait ChannelOutbound: Send + Sync {
    async fn send_text(&self, account_id: &str, to: &str, text: &str) -> Result<()>;
    async fn send_media(&self, account_id: &str, to: &str, payload: &ReplyPayload) -> Result<()>;
    /// Send a "typing" indicator. No-op by default.
    async fn send_typing(&self, _account_id: &str, _to: &str) -> Result<()> {
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
