//! Trait abstracting the gateway runtime operations that the chat engine needs.
//!
//! This decouples `moltis-chat` from `GatewayState` so the crate can be compiled
//! independently without a circular dependency on `moltis-gateway`.

use std::sync::Arc;

use serde_json::Value;

use {
    moltis_channels::{ChannelActivity, ChannelReplyTarget},
    moltis_tools::sandbox::SandboxRouter,
};

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

/// Summary of a connected remote node, returned by `ChatRuntime::connected_nodes`.
#[derive(Debug, Clone)]
pub struct ConnectedNodeSummary {
    pub node_id: String,
    pub display_name: Option<String>,
    pub platform: String,
    pub capabilities: Vec<String>,
    pub cpu_count: Option<u32>,
    pub cpu_usage: Option<f32>,
    pub mem_total: Option<u64>,
    pub mem_available: Option<u64>,
    pub telemetry_stale: bool,
    pub disk_total: Option<u64>,
    pub disk_available: Option<u64>,
    pub runtimes: Vec<String>,
    pub providers: Vec<(String, Vec<String>)>,
}

/// Abstraction over the mutable gateway runtime state that the chat engine
/// requires. The gateway implements this for `GatewayState`; tests can provide
/// a lightweight mock.
#[async_trait::async_trait]
pub trait ChatRuntime: Send + Sync {
    // ── Broadcasting ─────────────────────────────────────────────────────

    /// Broadcast a WebSocket event to all connected clients.
    async fn broadcast(&self, topic: &str, payload: Value);

    /// Reaction-feedback service, when one is configured.
    ///
    /// Defaulted to `None` so runtimes that do not collect feedback — tests,
    /// embedders — need no change.
    fn feedback(&self) -> Option<Arc<moltis_channels::FeedbackService>> {
        None
    }

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

    /// Memory runtime for long-term memory search.
    fn memory_manager(&self) -> Option<&moltis_memory::runtime::DynMemoryRuntime>;

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

    /// Report a mid-turn activity for a channel-dispatched session so the
    /// runtime can drive acknowledgment reactions (phase emojis) and status.
    ///
    /// Default no-op: runtimes without channel reactions (tests, headless)
    /// ignore it. Emitted from the agent loop (Thinking/Tool) and from the run
    /// completion (Finished).
    async fn note_channel_activity(&self, _activity_id: &str, _activity: ChannelActivity) {}

    /// Bind the given acknowledgment keys to this session's now-executing run,
    /// so activity routes to exactly the inbound message(s) this run handles.
    /// Called once the queue decision is known and the run actually starts.
    async fn activate_channel_acks(
        &self,
        _activity_id: &str,
        _session_key: &str,
        _ack_keys: Vec<String>,
    ) {
    }

    /// Finalize the turn identified by `activity_id`.
    ///
    /// For abort, where the run future is killed and cannot emit its own
    /// terminal. Turn-addressed, so it cannot resolve a turn that starts later.
    async fn finalize_active_channel_acks(
        &self,
        _activity_id: &str,
        _outcome: moltis_channels::ChannelAckOutcome,
    ) {
    }

    /// Finalize acknowledgment keys directly, for paths where no run executes
    /// (rejected hook, early error) and nothing else will signal a terminal.
    async fn finalize_channel_acks(
        &self,
        _ack_keys: Vec<String>,
        _outcome: moltis_channels::ChannelAckOutcome,
    ) {
    }

    // ── Push notifications ───────────────────────────────────────────────

    /// Human-readable label for a session, used to title push notifications.
    ///
    /// Returns `None` when the session has no label yet (auto-titling may not
    /// have run) or when no metadata store is wired up.
    async fn session_label(&self, _session_key: &str) -> Option<String> {
        None
    }

    /// Send a push notification to all subscribed devices.
    /// Returns the number of devices notified, or an error.
    async fn send_push_notification(
        &self,
        title: &str,
        body: &str,
        url: Option<&str>,
        session_key: Option<&str>,
    ) -> crate::error::Result<usize>;

    /// Send an ordered notification. External runtimes that do not implement
    /// ordering retain their existing delivery behavior by default.
    async fn send_ordered_push_notification(
        &self,
        title: &str,
        body: &str,
        url: Option<&str>,
        session_key: Option<&str>,
        _order: u64,
    ) -> crate::error::Result<usize> {
        self.send_push_notification(title, body, url, session_key)
            .await
    }

    // ── Local LLM ────────────────────────────────────────────────────────

    /// Ensure a local model is cached/downloaded. No-op if local-llm is disabled.
    async fn ensure_local_model_cached(&self, model_id: &str) -> crate::error::Result<bool>;

    /// Ensure a local model is loaded into RAM (broadcasting lifecycle events).
    /// No-op if local-llm is disabled or the model is already loaded.
    async fn ensure_local_model_loaded(&self, model_id: &str) -> crate::error::Result<()> {
        let _ = model_id;
        Ok(())
    }

    // ── Remote nodes ─────────────────────────────────────────────────────

    /// List currently connected remote nodes.
    async fn connected_nodes(&self) -> Vec<ConnectedNodeSummary>;

    // ── Mid-flight steering ──────────────────────────────────────────────

    /// Take (drain) all pending `/steer` texts for a session.
    async fn take_steer_text(&self, _session_key: &str) -> Option<Vec<String>> {
        None
    }

    /// Check whether fast/priority mode is enabled for a session.
    async fn is_fast_mode(&self, _session_key: &str) -> bool {
        false
    }

    /// Trigger background auto-title generation for a session.
    ///
    /// Called after the first assistant response. The gateway implements this
    /// to spawn a background LLM call; the default is a no-op.
    async fn trigger_auto_title(&self, _session_key: &str) {}
}
