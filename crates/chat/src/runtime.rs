//! Trait abstracting the gateway runtime operations that the chat engine needs.
//!
//! This decouples `moltis-chat` from `GatewayState` so the crate can be compiled
//! independently without a circular dependency on `moltis-gateway`.

use std::sync::Arc;

use serde_json::Value;

use {moltis_channels::ChannelReplyTarget, moltis_tools::sandbox::SandboxRouter};

/// TTS runtime override configuration (provider/voice/model).
///
/// Mirrors `TtsRuntimeOverride` from the gateway state, but owned by this crate
/// to avoid a gateway dependency.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TtsOverride {
    pub provider: Option<String>,
    pub voice_id: Option<String>,
    pub model: Option<String>,
}

/// Abstraction over the mutable gateway runtime state that the chat engine
/// requires. The gateway implements this for `GatewayState`; tests can provide
/// a lightweight mock.
#[async_trait::async_trait]
pub trait ChatRuntime: Send + Sync {
    // ── Broadcasting ─────────────────────────────────────────────────────

    /// Broadcast a WebSocket event to all connected clients.
    async fn broadcast(&self, topic: &str, payload: Value);

    // ── Channel reply queue ──────────────────────────────────────────────

    /// Push a reply target for a session (channel message triggered a chat run).
    async fn push_channel_reply(&self, session_key: &str, target: ChannelReplyTarget);

    /// Drain all pending reply targets for a session (removing them).
    async fn drain_channel_replies(&self, session_key: &str) -> Vec<ChannelReplyTarget>;

    /// Peek at pending reply targets without removing them.
    async fn peek_channel_replies(&self, session_key: &str) -> Vec<ChannelReplyTarget>;

    // ── Channel status log ───────────────────────────────────────────────

    /// Append a status line (tool use, model selection) to a session's log.
    async fn push_channel_status_log(&self, session_key: &str, message: String);

    /// Drain all buffered status log entries for a session.
    async fn drain_channel_status_log(&self, session_key: &str) -> Vec<String>;

    // ── Run error tracking ───────────────────────────────────────────────

    /// Record a run error (for `send_sync` to retrieve).
    async fn set_run_error(&self, run_id: &str, error: String);

    // ── Connection → session/project mapping ─────────────────────────────

    /// Resolve the active session key for a connection.
    async fn active_session_key(&self, conn_id: &str) -> Option<String>;

    /// Resolve the active project id for a connection.
    async fn active_project_id(&self, conn_id: &str) -> Option<String>;

    // ── Immutable accessors ──────────────────────────────────────────────

    /// Server hostname.
    fn hostname(&self) -> &str;

    /// Per-session sandbox router, if configured.
    fn sandbox_router(&self) -> Option<&Arc<SandboxRouter>>;

    /// Memory manager for long-term memory search.
    fn memory_manager(&self) -> Option<&Arc<moltis_memory::manager::MemoryManager>>;

    // ── Cached location ──────────────────────────────────────────────────

    /// Cached user geolocation from browser.
    async fn cached_location(&self) -> Option<moltis_config::GeoLocation>;

    // ── TTS overrides ────────────────────────────────────────────────────

    /// Resolve TTS overrides for a session+channel combination.
    /// Returns `(channel_override, session_override)`.
    async fn tts_overrides(
        &self,
        session_key: &str,
        channel_key: &str,
    ) -> (Option<TtsOverride>, Option<TtsOverride>);

    // ── Services ─────────────────────────────────────────────────────────

    /// Channel outbound service for delivering replies.
    fn channel_outbound(&self) -> Option<Arc<dyn moltis_channels::ChannelOutbound>>;

    /// Channel stream outbound for edit-in-place streaming.
    fn channel_stream_outbound(&self) -> Option<Arc<dyn moltis_channels::ChannelStreamOutbound>>;

    /// TTS service for voice synthesis.
    fn tts_service(&self) -> &dyn moltis_service_traits::TtsService;

    /// Project service for loading project context.
    fn project_service(&self) -> &dyn moltis_service_traits::ProjectService;

    /// MCP service for listing MCP servers.
    fn mcp_service(&self) -> &dyn moltis_service_traits::McpService;

    /// Get the active chat service (for draining queued messages recursively).
    async fn chat_service(&self) -> Arc<dyn moltis_service_traits::ChatService>;

    /// Take (and remove) the last error for a run_id.
    async fn last_run_error(&self, run_id: &str) -> Option<String>;

    // ── Push notifications ───────────────────────────────────────────────

    /// Send a push notification to all subscribed devices.
    /// Returns the number of devices notified, or an error.
    async fn send_push_notification(
        &self,
        title: &str,
        body: &str,
        url: Option<&str>,
        session_key: Option<&str>,
    ) -> crate::error::Result<usize>;

    // ── Local LLM ────────────────────────────────────────────────────────

    /// Ensure a local model is cached/downloaded. No-op if local-llm is disabled.
    async fn ensure_local_model_cached(&self, model_id: &str) -> crate::error::Result<bool>;
}
