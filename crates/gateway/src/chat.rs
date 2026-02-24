//! Chat execution engine — re-exported from [`moltis_chat`] with the gateway
//! runtime adapter.

pub use moltis_chat::*;

use std::sync::Arc;

use {async_trait::async_trait, serde_json::Value};

use {moltis_channels::ChannelReplyTarget, moltis_tools::sandbox::SandboxRouter};

use crate::state::GatewayState;

// ── GatewayChatRuntime ──────────────────────────────────────────────────────

/// Adapts [`GatewayState`] to the [`ChatRuntime`] trait expected by
/// `moltis-chat`.
pub struct GatewayChatRuntime {
    state: Arc<GatewayState>,
}

impl GatewayChatRuntime {
    pub fn from_state(state: Arc<GatewayState>) -> Arc<dyn ChatRuntime> {
        Arc::new(Self { state })
    }
}

#[async_trait]
impl ChatRuntime for GatewayChatRuntime {
    // ── Broadcasting ────────────────────────────────────────────────────────

    async fn broadcast(&self, topic: &str, payload: Value) {
        crate::broadcast::broadcast(
            &self.state,
            topic,
            payload,
            crate::broadcast::BroadcastOpts::default(),
        )
        .await;
    }

    // ── Channel reply queue ─────────────────────────────────────────────────

    async fn push_channel_reply(&self, session_key: &str, target: ChannelReplyTarget) {
        self.state.push_channel_reply(session_key, target).await;
    }

    async fn drain_channel_replies(&self, session_key: &str) -> Vec<ChannelReplyTarget> {
        self.state.drain_channel_replies(session_key).await
    }

    async fn peek_channel_replies(&self, session_key: &str) -> Vec<ChannelReplyTarget> {
        self.state.peek_channel_replies(session_key).await
    }

    // ── Channel status log ──────────────────────────────────────────────────

    async fn push_channel_status_log(&self, session_key: &str, message: String) {
        self.state
            .push_channel_status_log(session_key, message)
            .await;
    }

    async fn drain_channel_status_log(&self, session_key: &str) -> Vec<String> {
        self.state.drain_channel_status_log(session_key).await
    }

    // ── Run error tracking ──────────────────────────────────────────────────

    async fn set_run_error(&self, run_id: &str, error: String) {
        self.state.set_run_error(run_id, error).await;
    }

    // ── Connection → session/project mapping ────────────────────────────────

    async fn active_session_key(&self, conn_id: &str) -> Option<String> {
        self.state
            .inner
            .read()
            .await
            .active_sessions
            .get(conn_id)
            .cloned()
    }

    async fn active_project_id(&self, conn_id: &str) -> Option<String> {
        self.state
            .inner
            .read()
            .await
            .active_projects
            .get(conn_id)
            .cloned()
    }

    // ── Immutable accessors ─────────────────────────────────────────────────

    fn hostname(&self) -> &str {
        &self.state.hostname
    }

    fn sandbox_router(&self) -> Option<&Arc<SandboxRouter>> {
        self.state.sandbox_router.as_ref()
    }

    fn memory_manager(&self) -> Option<&Arc<moltis_memory::manager::MemoryManager>> {
        self.state.memory_manager.as_ref()
    }

    // ── Cached location ─────────────────────────────────────────────────────

    async fn cached_location(&self) -> Option<moltis_config::GeoLocation> {
        self.state.inner.read().await.cached_location.clone()
    }

    // ── TTS overrides ───────────────────────────────────────────────────────

    async fn tts_overrides(
        &self,
        session_key: &str,
        channel_key: &str,
    ) -> (Option<TtsOverride>, Option<TtsOverride>) {
        let inner = self.state.inner.read().await;
        let channel = inner
            .tts_channel_overrides
            .get(channel_key)
            .map(|o| TtsOverride {
                provider: o.provider.clone(),
                voice_id: o.voice_id.clone(),
                model: o.model.clone(),
            });
        let session = inner
            .tts_session_overrides
            .get(session_key)
            .map(|o| TtsOverride {
                provider: o.provider.clone(),
                voice_id: o.voice_id.clone(),
                model: o.model.clone(),
            });
        (channel, session)
    }

    // ── Services ────────────────────────────────────────────────────────────

    fn channel_outbound(&self) -> Option<Arc<dyn moltis_channels::ChannelOutbound>> {
        self.state.services.channel_outbound_arc()
    }

    fn channel_stream_outbound(&self) -> Option<Arc<dyn moltis_channels::ChannelStreamOutbound>> {
        self.state.services.channel_stream_outbound_arc()
    }

    fn tts_service(&self) -> &dyn moltis_service_traits::TtsService {
        &*self.state.services.tts
    }

    fn project_service(&self) -> &dyn moltis_service_traits::ProjectService {
        &*self.state.services.project
    }

    fn mcp_service(&self) -> &dyn moltis_service_traits::McpService {
        &*self.state.services.mcp
    }

    async fn chat_service(&self) -> Arc<dyn moltis_service_traits::ChatService> {
        self.state.chat().await
    }

    async fn last_run_error(&self, run_id: &str) -> Option<String> {
        self.state.last_run_error(run_id).await
    }

    // ── Push notifications ──────────────────────────────────────────────────

    async fn send_push_notification(
        &self,
        title: &str,
        body: &str,
        url: Option<&str>,
        session_key: Option<&str>,
    ) -> anyhow::Result<usize> {
        #[cfg(feature = "push-notifications")]
        {
            if let Some(push_service) = self.state.get_push_service().await {
                return crate::push_routes::send_push_notification(
                    &push_service,
                    title,
                    body,
                    url,
                    session_key,
                )
                .await;
            }
        }
        let _ = (title, body, url, session_key);
        Ok(0)
    }

    // ── Local LLM ───────────────────────────────────────────────────────────

    async fn ensure_local_model_cached(&self, model_id: &str) -> Result<bool, String> {
        #[cfg(feature = "local-llm")]
        {
            return crate::local_llm_setup::ensure_local_model_cached(model_id, &self.state).await;
        }
        #[cfg(not(feature = "local-llm"))]
        {
            let _ = model_id;
            Ok(false)
        }
    }
}
