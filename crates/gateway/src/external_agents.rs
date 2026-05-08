use std::{collections::HashMap, sync::Arc, time::SystemTime};

use {
    async_trait::async_trait,
    futures::StreamExt,
    moltis_config::schema::ExternalAgentsConfig,
    moltis_external_agents::{
        AcpPermissionHandler, AcpPermissionOptionKind, AcpPermissionRequest, AgentTransportKind,
        ContextSnapshot, ExternalAgentEvent, ExternalAgentRegistry, ExternalAgentSession,
        ExternalAgentSpec,
        runtimes::{acp::AcpTransport, claude_code::ClaudeCodeTransport, codex::CodexTransport},
        types::ContextTurn,
    },
    moltis_service_traits::{ChatService, ExternalAgentService, ServiceResult, SessionService},
    moltis_sessions::{MessageContent, PersistedMessage},
    serde_json::Value,
    tokio::sync::Mutex,
    tracing::warn,
};

use moltis_tools::approval::{ApprovalDecision, ApprovalManager};

use crate::{broadcast::BroadcastOpts, state::GatewayState};

pub struct GatewayExternalAgentService {
    registry: ExternalAgentRegistry,
    config: ExternalAgentsConfig,
    session_metadata: Arc<moltis_sessions::metadata::SqliteSessionMetadata>,
    live_sessions: Mutex<HashMap<LiveSessionKey, LiveSessionEntry>>,
}

type LiveExternalAgentSession = Arc<Mutex<Box<dyn ExternalAgentSession>>>;

const LIVE_SESSION_IDLE_TTL: std::time::Duration = std::time::Duration::from_secs(60 * 60);

struct LiveSessionEntry {
    session: LiveExternalAgentSession,
    last_used: std::time::Instant,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LiveSessionKey {
    session_key: String,
    kind: AgentTransportKind,
}

struct GatewayAcpPermissionHandler {
    approval_manager: Arc<ApprovalManager>,
}

impl GatewayAcpPermissionHandler {
    fn new(approval_manager: Arc<ApprovalManager>) -> Self {
        Self { approval_manager }
    }
}

#[async_trait]
impl AcpPermissionHandler for GatewayAcpPermissionHandler {
    async fn select_option(&self, request: AcpPermissionRequest) -> anyhow::Result<Option<String>> {
        let command = format!(
            "ACP permission requested for {} [{}]",
            request.tool_call,
            request
                .options
                .iter()
                .map(|option| option.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let session_key = request.moltis_session_key.as_deref();
        let (request_id, decision_rx) = self
            .approval_manager
            .create_request(&command, session_key)
            .await;
        if let Some(session_key) = request.moltis_session_key.as_deref() {
            tracing::info!(request_id, session_key, "ACP permission request is pending");
        }
        match self.approval_manager.wait_for_decision(decision_rx).await {
            ApprovalDecision::Approved => Ok(select_allowed_acp_option(&request)),
            ApprovalDecision::Denied | ApprovalDecision::Timeout => {
                Ok(select_rejected_acp_option(&request))
            },
        }
    }
}

fn select_allowed_acp_option(request: &AcpPermissionRequest) -> Option<String> {
    request
        .options
        .iter()
        .find(|option| option.kind == AcpPermissionOptionKind::AllowOnce)
        .or_else(|| {
            request
                .options
                .iter()
                .find(|option| option.kind == AcpPermissionOptionKind::AllowAlways)
        })
        .map(|option| option.id.clone())
}

fn select_rejected_acp_option(request: &AcpPermissionRequest) -> Option<String> {
    request
        .options
        .iter()
        .find(|option| option.kind == AcpPermissionOptionKind::RejectOnce)
        .or_else(|| {
            request
                .options
                .iter()
                .find(|option| option.kind == AcpPermissionOptionKind::RejectAlways)
        })
        .map(|option| option.id.clone())
}

impl GatewayExternalAgentService {
    pub fn new(
        config: ExternalAgentsConfig,
        session_metadata: Arc<moltis_sessions::metadata::SqliteSessionMetadata>,
        approval_manager: Arc<ApprovalManager>,
    ) -> Self {
        let mut registry = ExternalAgentRegistry::new();
        registry.register(Box::new(ClaudeCodeTransport::new()));
        registry.register(Box::new(CodexTransport::new()));
        registry.register(Box::new(
            AcpTransport::new("acp".to_string()).with_permission_handler(Arc::new(
                GatewayAcpPermissionHandler::new(approval_manager),
            )),
        ));
        Self {
            registry,
            config,
            session_metadata,
            live_sessions: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    fn with_registry(
        config: ExternalAgentsConfig,
        session_metadata: Arc<moltis_sessions::metadata::SqliteSessionMetadata>,
        registry: ExternalAgentRegistry,
    ) -> Self {
        Self {
            registry,
            config,
            session_metadata,
            live_sessions: Mutex::new(HashMap::new()),
        }
    }

    async fn session_for_binding(
        &self,
        session_key: &str,
        kind: AgentTransportKind,
    ) -> anyhow::Result<LiveExternalAgentSession> {
        self.shutdown_idle_sessions().await;
        let key = LiveSessionKey {
            session_key: session_key.to_string(),
            kind,
        };
        let mut live_sessions = self.live_sessions.lock().await;
        if let Some(entry) = live_sessions.get_mut(&key) {
            let is_alive = entry.session.lock().await.is_alive().await;
            if is_alive {
                entry.last_used = std::time::Instant::now();
                return Ok(Arc::clone(&entry.session));
            }
        }
        let spec = self.spec_for_kind(kind)?;
        let mut spec = spec;
        spec.session_key = Some(session_key.to_string());
        spec.external_session_id = self
            .session_metadata
            .get(session_key)
            .await
            .and_then(|entry| entry.external_session_id);
        let session = Arc::new(Mutex::new(self.registry.start_session(&spec).await?));
        live_sessions.insert(key, LiveSessionEntry {
            session: Arc::clone(&session),
            last_used: std::time::Instant::now(),
        });
        Ok(session)
    }

    async fn shutdown_idle_sessions(&self) {
        let sessions = {
            let mut live_sessions = self.live_sessions.lock().await;
            let now = std::time::Instant::now();
            let keys = live_sessions
                .iter()
                .filter(|(_, entry)| now.duration_since(entry.last_used) >= LIVE_SESSION_IDLE_TTL)
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>();
            keys.into_iter()
                .filter_map(|key| live_sessions.remove(&key).map(|entry| entry.session))
                .collect::<Vec<_>>()
        };
        for session in sessions {
            let mut session = session.lock().await;
            if let Err(error) = session.shutdown().await {
                warn!(%error, "failed to shut down idle external agent session");
            }
        }
    }

    pub(crate) async fn shutdown_binding(&self, session_key: &str) {
        let sessions = {
            let mut live_sessions = self.live_sessions.lock().await;
            let keys = live_sessions
                .keys()
                .filter(|key| key.session_key == session_key)
                .cloned()
                .collect::<Vec<_>>();
            keys.into_iter()
                .filter_map(|key| live_sessions.remove(&key).map(|entry| entry.session))
                .collect::<Vec<_>>()
        };
        for session in sessions {
            let mut session = session.lock().await;
            if let Err(error) = session.shutdown().await {
                warn!(%error, session_key, "failed to shut down external agent session");
            }
        }
    }

    fn spec_for_kind(&self, kind: AgentTransportKind) -> anyhow::Result<ExternalAgentSpec> {
        if !self.config.enabled {
            anyhow::bail!("external agents are disabled")
        }
        let mut spec = ExternalAgentSpec::new(kind);
        if let Some(agent_config) = self.config.agents.get(kind.as_str()) {
            spec.binary = agent_config.binary.clone();
            spec.args = agent_config.args.clone();
            spec.env = agent_config.env.clone();
            spec.working_dir = agent_config.working_dir.as_ref().map(Into::into);
            spec.timeout_secs = agent_config.timeout_secs;
            spec.use_tmux = agent_config.use_tmux.unwrap_or(false);
        }
        Ok(spec)
    }
}

pub struct ExternalAgentSessionService {
    inner: Arc<dyn SessionService>,
    external_agents: Arc<GatewayExternalAgentService>,
}

impl ExternalAgentSessionService {
    pub fn new(
        inner: Arc<dyn SessionService>,
        external_agents: Arc<GatewayExternalAgentService>,
    ) -> Self {
        Self {
            inner,
            external_agents,
        }
    }
}

#[async_trait]
impl SessionService for ExternalAgentSessionService {
    async fn list(&self) -> ServiceResult {
        self.inner.list().await
    }

    async fn preview(&self, params: Value) -> ServiceResult {
        self.inner.preview(params).await
    }

    async fn resolve(&self, params: Value) -> ServiceResult {
        self.inner.resolve(params).await
    }

    async fn patch(&self, params: Value) -> ServiceResult {
        self.inner.patch(params).await
    }

    async fn voice_generate(&self, params: Value) -> ServiceResult {
        self.inner.voice_generate(params).await
    }

    async fn share_create(&self, params: Value) -> ServiceResult {
        self.inner.share_create(params).await
    }

    async fn share_list(&self, params: Value) -> ServiceResult {
        self.inner.share_list(params).await
    }

    async fn share_revoke(&self, params: Value) -> ServiceResult {
        self.inner.share_revoke(params).await
    }

    async fn reset(&self, params: Value) -> ServiceResult {
        if let Some(session_key) = session_key_param(&params) {
            self.external_agents.shutdown_binding(&session_key).await;
        }
        self.inner.reset(params).await
    }

    async fn delete(&self, params: Value) -> ServiceResult {
        if let Some(session_key) = session_key_param(&params) {
            self.external_agents.shutdown_binding(&session_key).await;
        }
        self.inner.delete(params).await
    }

    async fn compact(&self, params: Value) -> ServiceResult {
        self.inner.compact(params).await
    }

    async fn search(&self, params: Value) -> ServiceResult {
        self.inner.search(params).await
    }

    async fn fork(&self, params: Value) -> ServiceResult {
        self.inner.fork(params).await
    }

    async fn branches(&self, params: Value) -> ServiceResult {
        self.inner.branches(params).await
    }

    async fn run_detail(&self, params: Value) -> ServiceResult {
        self.inner.run_detail(params).await
    }

    async fn clear_all(&self) -> ServiceResult {
        for entry in self.external_agents.session_metadata.list().await {
            self.external_agents.shutdown_binding(&entry.key).await;
        }
        self.inner.clear_all().await
    }

    async fn mark_seen(&self, key: &str) {
        self.inner.mark_seen(key).await;
    }
}

#[async_trait]
impl ExternalAgentService for GatewayExternalAgentService {
    async fn list(&self) -> ServiceResult {
        Ok(serde_json::to_value(self.registry.list_agents().await)
            .unwrap_or_else(|_| serde_json::json!([])))
    }

    async fn bind(&self, params: Value) -> ServiceResult {
        if !self.config.enabled {
            return Err("external agents are disabled".into());
        }
        let session_key = params
            .get("sessionKey")
            .or_else(|| params.get("session_key"))
            .and_then(|value| value.as_str())
            .ok_or_else(|| "missing sessionKey".to_string())?;
        let kind = params
            .get("kind")
            .and_then(|value| value.as_str())
            .ok_or_else(|| "missing kind".to_string())?
            .parse::<AgentTransportKind>()
            .map_err(|error| error.to_string())?;
        if !self.registry.has_kind(kind) {
            return Err(format!("external agent kind is not registered: {kind}").into());
        }
        self.shutdown_binding(session_key).await;
        let _ = self.session_metadata.upsert(session_key, None).await;
        self.session_metadata
            .set_external_agent(session_key, Some(kind), None)
            .await;
        Ok(serde_json::json!({ "ok": true, "sessionKey": session_key, "kind": kind.as_str() }))
    }

    async fn unbind(&self, params: Value) -> ServiceResult {
        let session_key = params
            .get("sessionKey")
            .or_else(|| params.get("session_key"))
            .and_then(|value| value.as_str())
            .ok_or_else(|| "missing sessionKey".to_string())?;
        self.shutdown_binding(session_key).await;
        self.session_metadata
            .set_external_agent(session_key, None, None)
            .await;
        Ok(serde_json::json!({ "ok": true, "sessionKey": session_key }))
    }

    async fn status(&self, params: Value) -> ServiceResult {
        let session_key = params
            .get("sessionKey")
            .or_else(|| params.get("session_key"))
            .and_then(|value| value.as_str())
            .ok_or_else(|| "missing sessionKey".to_string())?;
        let entry = self.session_metadata.get(session_key).await;
        let kind = entry.as_ref().and_then(|entry| entry.external_agent_kind);
        Ok(serde_json::json!({
            "bound": kind.is_some(),
            "sessionKey": session_key,
            "kind": kind.map(|kind| kind.as_str()),
            "externalSessionId": entry.and_then(|entry| entry.external_session_id),
        }))
    }
}

pub struct ExternalAgentChatService {
    inner: Arc<dyn ChatService>,
    external_agents: Arc<GatewayExternalAgentService>,
    state: Arc<GatewayState>,
    session_store: Arc<moltis_sessions::store::SessionStore>,
    session_metadata: Arc<moltis_sessions::metadata::SqliteSessionMetadata>,
}

impl ExternalAgentChatService {
    pub fn new(
        inner: Arc<dyn ChatService>,
        external_agents: Arc<GatewayExternalAgentService>,
        state: Arc<GatewayState>,
        session_store: Arc<moltis_sessions::store::SessionStore>,
        session_metadata: Arc<moltis_sessions::metadata::SqliteSessionMetadata>,
    ) -> Self {
        Self {
            inner,
            external_agents,
            state,
            session_store,
            session_metadata,
        }
    }

    async fn maybe_send_external(&self, params: &Value) -> Option<ServiceResult> {
        let session_key = resolve_session_key(params, &self.state).await;
        let entry = self.session_metadata.get(&session_key).await?;
        let kind = entry.external_agent_kind?;
        Some(self.send_external(params.clone(), session_key, kind).await)
    }

    async fn send_external(
        &self,
        params: Value,
        session_key: String,
        kind: AgentTransportKind,
    ) -> ServiceResult {
        let text = params
            .get("text")
            .or_else(|| params.get("message"))
            .and_then(|value| value.as_str())
            .ok_or_else(|| "external agents currently require text input".to_string())?
            .to_string();
        let seq = params.get("_seq").and_then(|value| value.as_u64());
        let run_id = uuid::Uuid::new_v4().to_string();
        let created_at = now_ms();
        let mut history = self
            .session_store
            .read(&session_key)
            .await
            .unwrap_or_default();
        let user_msg = PersistedMessage::User {
            content: MessageContent::Text(text.clone()),
            created_at: Some(created_at),
            audio: None,
            documents: None,
            channel: params.get("channel").cloned(),
            seq,
            run_id: Some(run_id.clone()),
        };
        self.session_store
            .append(&session_key, &user_msg.to_value())
            .await
            .map_err(|error| error.to_string())?;
        history.push(user_msg.to_value());
        self.session_metadata
            .touch(&session_key, history.len() as u32)
            .await;

        crate::broadcast::broadcast(
            &self.state,
            "chat",
            serde_json::json!({
                "runId": run_id,
                "sessionKey": session_key,
                "state": "running",
                "model": kind.as_str(),
                "provider": "external-agent",
                "seq": seq,
            }),
            BroadcastOpts::default(),
        )
        .await;

        let context = context_from_history(&history);
        let start = std::time::Instant::now();
        let live_session = self
            .external_agents
            .session_for_binding(&session_key, kind)
            .await
            .map_err(|error| error.to_string())?;
        let mut session = live_session.lock().await;
        let external_session_id = session.external_session_id().map(str::to_string);
        if external_session_id.is_some() {
            self.session_metadata
                .set_external_agent(&session_key, Some(kind), external_session_id.clone())
                .await;
        }
        let mut events = match session.send_prompt(&text, Some(&context)).await {
            Ok(events) => events,
            Err(error) => {
                let error = error.to_string();
                drop(session);
                self.external_agents.shutdown_binding(&session_key).await;
                return Err(error.into());
            },
        };
        let mut assistant_text = String::new();
        let mut token_usage = None;
        let mut external_error = None;
        while let Some(event) = events.next().await {
            match event {
                ExternalAgentEvent::TextDelta(delta) => {
                    assistant_text.push_str(&delta);
                    crate::broadcast::broadcast(
                        &self.state,
                        "chat",
                        serde_json::json!({
                            "runId": run_id,
                            "sessionKey": session_key,
                            "state": "delta",
                            "text": delta,
                            "seq": seq,
                        }),
                        BroadcastOpts::default(),
                    )
                    .await;
                },
                ExternalAgentEvent::ThinkingDelta(delta) => {
                    crate::broadcast::broadcast(
                        &self.state,
                        "chat",
                        serde_json::json!({
                            "runId": run_id,
                            "sessionKey": session_key,
                            "state": "thinking_text",
                            "text": delta,
                            "seq": seq,
                        }),
                        BroadcastOpts::default(),
                    )
                    .await;
                },
                ExternalAgentEvent::Error(error) => {
                    external_error = Some(error);
                    break;
                },
                ExternalAgentEvent::Done { usage } => {
                    token_usage = usage;
                },
                ExternalAgentEvent::ToolCallStart { .. }
                | ExternalAgentEvent::ToolCallEnd { .. } => {},
            }
        }
        if let Some(external_session_id) = session.external_session_id().map(str::to_string) {
            self.session_metadata
                .set_external_agent(&session_key, Some(kind), Some(external_session_id))
                .await;
        }
        drop(session);
        if let Some(error) = external_error {
            self.external_agents.shutdown_binding(&session_key).await;
            return Err(error.into());
        }
        let duration_ms = start.elapsed().as_millis() as u64;
        let assistant_msg = PersistedMessage::Assistant {
            content: assistant_text.clone(),
            created_at: Some(now_ms()),
            model: Some(kind.as_str().to_string()),
            provider: Some("external-agent".to_string()),
            input_tokens: token_usage.as_ref().map(|usage| usage.input_tokens),
            output_tokens: token_usage.as_ref().map(|usage| usage.output_tokens),
            cache_read_tokens: None,
            cache_write_tokens: None,
            duration_ms: Some(duration_ms),
            request_input_tokens: None,
            request_output_tokens: None,
            request_cache_read_tokens: None,
            request_cache_write_tokens: None,
            tool_calls: None,
            reasoning: None,
            llm_api_response: None,
            audio: None,
            seq,
            run_id: Some(run_id.clone()),
        };
        self.session_store
            .append(&session_key, &assistant_msg.to_value())
            .await
            .map_err(|error| error.to_string())?;
        let message_count = history.len() + 1;
        self.session_metadata
            .touch(&session_key, message_count as u32)
            .await;
        crate::broadcast::broadcast(
            &self.state,
            "chat",
            serde_json::json!({
                "runId": run_id,
                "sessionKey": session_key,
                "state": "final",
                "text": assistant_text,
                "model": kind.as_str(),
                "provider": "external-agent",
                "inputTokens": token_usage.as_ref().map(|usage| usage.input_tokens).unwrap_or(0),
                "outputTokens": token_usage.as_ref().map(|usage| usage.output_tokens).unwrap_or(0),
                "durationMs": duration_ms,
                "messageIndex": message_count - 1,
                "replyMedium": "text",
                "seq": seq,
            }),
            BroadcastOpts::default(),
        )
        .await;
        Ok(serde_json::json!({ "ok": true, "runId": run_id }))
    }
}

#[async_trait]
impl ChatService for ExternalAgentChatService {
    async fn send(&self, params: Value) -> ServiceResult {
        if let Some(result) = self.maybe_send_external(&params).await {
            return result;
        }
        self.inner.send(params).await
    }

    async fn send_sync(&self, params: Value) -> ServiceResult {
        self.send(params).await
    }

    async fn abort(&self, params: Value) -> ServiceResult {
        let session_key = resolve_session_key(&params, &self.state).await;
        self.external_agents.shutdown_binding(&session_key).await;
        self.inner.abort(params).await
    }

    async fn cancel_queued(&self, params: Value) -> ServiceResult {
        self.inner.cancel_queued(params).await
    }

    async fn history(&self, params: Value) -> ServiceResult {
        self.inner.history(params).await
    }

    async fn inject(&self, params: Value) -> ServiceResult {
        self.inner.inject(params).await
    }

    async fn clear(&self, params: Value) -> ServiceResult {
        let session_key = resolve_session_key(&params, &self.state).await;
        self.external_agents.shutdown_binding(&session_key).await;
        self.inner.clear(params).await
    }

    async fn compact(&self, params: Value) -> ServiceResult {
        self.inner.compact(params).await
    }

    async fn context(&self, params: Value) -> ServiceResult {
        self.inner.context(params).await
    }

    async fn raw_prompt(&self, params: Value) -> ServiceResult {
        self.inner.raw_prompt(params).await
    }

    async fn full_context(&self, params: Value) -> ServiceResult {
        self.inner.full_context(params).await
    }

    async fn refresh_prompt_memory(&self, params: Value) -> ServiceResult {
        self.inner.refresh_prompt_memory(params).await
    }

    async fn active(&self, params: Value) -> ServiceResult {
        self.inner.active(params).await
    }

    async fn active_session_keys(&self) -> Vec<String> {
        self.inner.active_session_keys().await
    }

    async fn active_thinking_text(&self, session_key: &str) -> Option<String> {
        self.inner.active_thinking_text(session_key).await
    }

    async fn active_voice_pending(&self, session_key: &str) -> bool {
        self.inner.active_voice_pending(session_key).await
    }

    async fn peek(&self, params: Value) -> ServiceResult {
        self.inner.peek(params).await
    }
}

async fn resolve_session_key(params: &Value, state: &GatewayState) -> String {
    if let Some(key) = params
        .get("_session_key")
        .or_else(|| params.get("sessionKey"))
        .or_else(|| params.get("session_key"))
        .and_then(|value| value.as_str())
    {
        return key.to_string();
    }
    let conn_id = params.get("_conn_id").and_then(|value| value.as_str());
    if let Some(conn_id) = conn_id
        && let Some(key) = state
            .client_registry
            .read()
            .await
            .active_sessions
            .get(conn_id)
            .cloned()
    {
        return key;
    }
    "main".to_string()
}

fn context_from_history(history: &[Value]) -> ContextSnapshot {
    let recent_turns = history
        .iter()
        .rev()
        .take(20)
        .filter_map(|value| serde_json::from_value::<PersistedMessage>(value.clone()).ok())
        .filter_map(|message| match message {
            PersistedMessage::User { content, .. } => Some(ContextTurn {
                role: "user".to_string(),
                content: message_content_text(&content),
            }),
            PersistedMessage::Assistant { content, .. } => Some(ContextTurn {
                role: "assistant".to_string(),
                content,
            }),
            PersistedMessage::System { content, .. } => Some(ContextTurn {
                role: "system".to_string(),
                content,
            }),
            _ => None,
        })
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    ContextSnapshot {
        recent_turns,
        ..ContextSnapshot::default()
    }
}

fn session_key_param(params: &Value) -> Option<String> {
    params
        .get("key")
        .or_else(|| params.get("sessionKey"))
        .or_else(|| params.get("session_key"))
        .or_else(|| params.get("_session_key"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn message_content_text(content: &MessageContent) -> String {
    match content {
        MessageContent::Text(text) => text.clone(),
        MessageContent::Multimodal(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                moltis_sessions::ContentBlock::Text { text } => Some(text.as_str()),
                moltis_sessions::ContentBlock::ImageUrl { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn now_ms() -> u64 {
    match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => duration.as_millis() as u64,
        Err(error) => {
            warn!(%error, "system clock is before UNIX_EPOCH");
            0
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::{pin::Pin, sync::atomic::Ordering};

    use {
        super::*,
        crate::{
            auth::{AuthMode, ResolvedAuth},
            services::GatewayServices,
        },
        futures::{Stream, stream},
        moltis_external_agents::{
            ExternalAgentTransport,
            types::{AcpPermissionOption, ExternalAgentStatus},
        },
        moltis_service_traits::{ExternalAgentService, NoopChatService},
        moltis_sessions::{metadata::SqliteSessionMetadata, store::SessionStore},
    };

    #[derive(Default)]
    struct FakeAgentState {
        starts: std::sync::atomic::AtomicUsize,
        prompts: std::sync::Mutex<Vec<String>>,
        shutdowns: std::sync::atomic::AtomicUsize,
    }

    struct FakeTransport {
        state: Arc<FakeAgentState>,
    }

    #[async_trait]
    impl ExternalAgentTransport for FakeTransport {
        fn name(&self) -> &str {
            "fake"
        }

        async fn is_available(&self) -> bool {
            true
        }

        fn supported_kinds(&self) -> &[AgentTransportKind] {
            &[AgentTransportKind::Codex]
        }

        async fn start_session(
            &self,
            _spec: &ExternalAgentSpec,
        ) -> anyhow::Result<Box<dyn ExternalAgentSession>> {
            let start_index = self.state.starts.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(Box::new(FakeSession {
                state: Arc::clone(&self.state),
                external_session_id: format!("fake-session-{start_index}"),
                alive: true,
            }))
        }
    }

    struct FakeSession {
        state: Arc<FakeAgentState>,
        external_session_id: String,
        alive: bool,
    }

    #[async_trait]
    impl ExternalAgentSession for FakeSession {
        fn external_session_id(&self) -> Option<&str> {
            Some(&self.external_session_id)
        }

        async fn send_prompt(
            &mut self,
            prompt: &str,
            _context: Option<&ContextSnapshot>,
        ) -> anyhow::Result<Pin<Box<dyn Stream<Item = ExternalAgentEvent> + Send>>> {
            if prompt == "fail" {
                anyhow::bail!("fake send failure");
            }
            if prompt == "event-error" {
                return Ok(Box::pin(stream::iter([ExternalAgentEvent::Error(
                    "fake event failure".to_string(),
                )])));
            }
            self.state
                .prompts
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(prompt.to_string());
            Ok(Box::pin(stream::iter([
                ExternalAgentEvent::TextDelta(format!("reply to {prompt}")),
                ExternalAgentEvent::Done {
                    usage: (prompt == "usage").then_some(
                        moltis_external_agents::types::TokenUsage {
                            input_tokens: 7,
                            output_tokens: 11,
                        },
                    ),
                },
            ])))
        }

        async fn is_alive(&self) -> bool {
            self.alive
        }

        async fn shutdown(&mut self) -> anyhow::Result<()> {
            self.alive = false;
            self.state.shutdowns.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn status(&self) -> ExternalAgentStatus {
            if self.alive {
                ExternalAgentStatus::Idle
            } else {
                ExternalAgentStatus::Stopped
            }
        }
    }

    async fn sqlite_pool() -> sqlx::SqlitePool {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        moltis_projects::run_migrations(&pool).await.unwrap();
        SqliteSessionMetadata::init(&pool).await.unwrap();
        pool
    }

    fn fake_external_agents(
        metadata: Arc<SqliteSessionMetadata>,
        state: Arc<FakeAgentState>,
    ) -> Arc<GatewayExternalAgentService> {
        let mut registry = ExternalAgentRegistry::new();
        registry.register(Box::new(FakeTransport { state }));
        Arc::new(GatewayExternalAgentService::with_registry(
            ExternalAgentsConfig {
                enabled: true,
                ..ExternalAgentsConfig::default()
            },
            metadata,
            registry,
        ))
    }

    fn test_gateway_state() -> Arc<GatewayState> {
        GatewayState::new(
            ResolvedAuth {
                mode: AuthMode::Token,
                token: None,
                password: None,
            },
            GatewayServices::noop(),
        )
    }

    async fn test_chat_service(
        external_agents: Arc<GatewayExternalAgentService>,
        metadata: Arc<SqliteSessionMetadata>,
        session_store: Arc<SessionStore>,
    ) -> ExternalAgentChatService {
        ExternalAgentChatService::new(
            Arc::new(NoopChatService),
            external_agents,
            test_gateway_state(),
            session_store,
            metadata,
        )
    }

    #[tokio::test]
    async fn bind_unbind_and_status_update_metadata() {
        let metadata = Arc::new(SqliteSessionMetadata::new(sqlite_pool().await));
        let agent_state = Arc::new(FakeAgentState::default());
        let service = fake_external_agents(Arc::clone(&metadata), agent_state);

        let bound = service
            .bind(serde_json::json!({ "sessionKey": "main", "kind": "codex" }))
            .await
            .expect("bind external agent");
        assert_eq!(bound["kind"], "codex");

        let status = service
            .status(serde_json::json!({ "sessionKey": "main" }))
            .await
            .expect("status");
        assert_eq!(status["bound"], true);
        assert_eq!(status["kind"], "codex");

        service
            .unbind(serde_json::json!({ "sessionKey": "main" }))
            .await
            .expect("unbind external agent");
        let status = service
            .status(serde_json::json!({ "sessionKey": "main" }))
            .await
            .expect("status after unbind");
        assert_eq!(status["bound"], false);
        assert!(status["kind"].is_null());
    }

    #[test]
    fn acp_permission_selection_prefers_matching_decision_kind() {
        let request = AcpPermissionRequest {
            moltis_session_key: Some("main".to_string()),
            acp_session_id: "acp-1".to_string(),
            tool_call: "run tool".to_string(),
            options: vec![
                AcpPermissionOption {
                    id: "reject".to_string(),
                    name: "Reject".to_string(),
                    kind: AcpPermissionOptionKind::RejectOnce,
                },
                AcpPermissionOption {
                    id: "allow".to_string(),
                    name: "Allow".to_string(),
                    kind: AcpPermissionOptionKind::AllowOnce,
                },
            ],
        };

        assert_eq!(
            select_allowed_acp_option(&request),
            Some("allow".to_string())
        );
        assert_eq!(
            select_rejected_acp_option(&request),
            Some("reject".to_string())
        );
    }

    #[tokio::test]
    async fn bound_chat_send_reuses_live_external_session() {
        let dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let metadata = Arc::new(SqliteSessionMetadata::new(sqlite_pool().await));
        let agent_state = Arc::new(FakeAgentState::default());
        let external_agents = fake_external_agents(Arc::clone(&metadata), Arc::clone(&agent_state));
        external_agents
            .bind(serde_json::json!({ "sessionKey": "main", "kind": "codex" }))
            .await
            .expect("bind external agent");
        let chat = test_chat_service(
            Arc::clone(&external_agents),
            Arc::clone(&metadata),
            Arc::clone(&session_store),
        )
        .await;

        chat.send(serde_json::json!({ "sessionKey": "main", "text": "one" }))
            .await
            .expect("first send");
        chat.send(serde_json::json!({ "sessionKey": "main", "text": "two" }))
            .await
            .expect("second send");

        assert_eq!(agent_state.starts.load(Ordering::SeqCst), 1);
        assert_eq!(
            *agent_state
                .prompts
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
            vec!["one".to_string(), "two".to_string()]
        );
        let history = session_store.read("main").await.expect("read history");
        assert_eq!(history.len(), 4);
        assert_eq!(history[1]["content"], "reply to one");
        assert_eq!(history[3]["provider"], "external-agent");
        assert_eq!(
            metadata
                .get("main")
                .await
                .and_then(|entry| entry.external_session_id),
            Some("fake-session-1".to_string())
        );
    }

    #[tokio::test]
    async fn idle_live_external_sessions_are_evicted_before_reuse() {
        let metadata = Arc::new(SqliteSessionMetadata::new(sqlite_pool().await));
        let agent_state = Arc::new(FakeAgentState::default());
        let external_agents = fake_external_agents(Arc::clone(&metadata), Arc::clone(&agent_state));
        external_agents
            .bind(serde_json::json!({ "sessionKey": "main", "kind": "codex" }))
            .await
            .expect("bind external agent");

        let first = external_agents
            .session_for_binding("main", AgentTransportKind::Codex)
            .await
            .expect("first live session");
        drop(first);
        {
            let mut live_sessions = external_agents.live_sessions.lock().await;
            for entry in live_sessions.values_mut() {
                entry.last_used = std::time::Instant::now() - LIVE_SESSION_IDLE_TTL;
            }
        }

        let second = external_agents
            .session_for_binding("main", AgentTransportKind::Codex)
            .await
            .expect("second live session");
        drop(second);

        assert_eq!(agent_state.shutdowns.load(Ordering::SeqCst), 1);
        assert_eq!(agent_state.starts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn send_failure_evicts_live_external_session() {
        let dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let metadata = Arc::new(SqliteSessionMetadata::new(sqlite_pool().await));
        let agent_state = Arc::new(FakeAgentState::default());
        let external_agents = fake_external_agents(Arc::clone(&metadata), Arc::clone(&agent_state));
        external_agents
            .bind(serde_json::json!({ "sessionKey": "main", "kind": "codex" }))
            .await
            .expect("bind external agent");
        let chat = test_chat_service(
            Arc::clone(&external_agents),
            Arc::clone(&metadata),
            Arc::clone(&session_store),
        )
        .await;

        chat.send(serde_json::json!({ "sessionKey": "main", "text": "one" }))
            .await
            .expect("first send");
        let error = chat
            .send(serde_json::json!({ "sessionKey": "main", "text": "fail" }))
            .await
            .expect_err("failing send should error");
        assert_eq!(error.to_string(), "fake send failure");
        assert_eq!(agent_state.shutdowns.load(Ordering::SeqCst), 1);

        chat.send(serde_json::json!({ "sessionKey": "main", "text": "two" }))
            .await
            .expect("send after eviction");
        assert_eq!(agent_state.starts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn error_event_evicts_live_external_session() {
        let dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let metadata = Arc::new(SqliteSessionMetadata::new(sqlite_pool().await));
        let agent_state = Arc::new(FakeAgentState::default());
        let external_agents = fake_external_agents(Arc::clone(&metadata), Arc::clone(&agent_state));
        external_agents
            .bind(serde_json::json!({ "sessionKey": "main", "kind": "codex" }))
            .await
            .expect("bind external agent");
        let chat = test_chat_service(
            Arc::clone(&external_agents),
            Arc::clone(&metadata),
            Arc::clone(&session_store),
        )
        .await;

        let error = chat
            .send(serde_json::json!({ "sessionKey": "main", "text": "event-error" }))
            .await
            .expect_err("error event should fail chat send");
        assert_eq!(error.to_string(), "fake event failure");
        assert_eq!(agent_state.shutdowns.load(Ordering::SeqCst), 1);

        chat.send(serde_json::json!({ "sessionKey": "main", "text": "two" }))
            .await
            .expect("send after event-error eviction");
        assert_eq!(agent_state.starts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn bound_chat_send_persists_external_token_usage() {
        let dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let metadata = Arc::new(SqliteSessionMetadata::new(sqlite_pool().await));
        let agent_state = Arc::new(FakeAgentState::default());
        let external_agents = fake_external_agents(Arc::clone(&metadata), agent_state);
        external_agents
            .bind(serde_json::json!({ "sessionKey": "main", "kind": "codex" }))
            .await
            .expect("bind external agent");
        let chat = test_chat_service(
            Arc::clone(&external_agents),
            Arc::clone(&metadata),
            Arc::clone(&session_store),
        )
        .await;

        chat.send(serde_json::json!({ "sessionKey": "main", "text": "usage" }))
            .await
            .expect("send with usage");

        let history = session_store.read("main").await.expect("read history");
        assert_eq!(history[1]["inputTokens"], 7);
        assert_eq!(history[1]["outputTokens"], 11);
    }

    #[tokio::test]
    async fn unbind_shuts_down_live_external_session() {
        let dir = tempfile::tempdir().unwrap();
        let session_store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
        let metadata = Arc::new(SqliteSessionMetadata::new(sqlite_pool().await));
        let agent_state = Arc::new(FakeAgentState::default());
        let external_agents = fake_external_agents(Arc::clone(&metadata), Arc::clone(&agent_state));
        external_agents
            .bind(serde_json::json!({ "sessionKey": "main", "kind": "codex" }))
            .await
            .expect("bind external agent");
        let chat = test_chat_service(
            Arc::clone(&external_agents),
            Arc::clone(&metadata),
            Arc::clone(&session_store),
        )
        .await;
        chat.send(serde_json::json!({ "sessionKey": "main", "text": "one" }))
            .await
            .expect("send starts live session");

        external_agents
            .unbind(serde_json::json!({ "sessionKey": "main" }))
            .await
            .expect("unbind external agent");

        assert_eq!(agent_state.shutdowns.load(Ordering::SeqCst), 1);
        assert!(external_agents.live_sessions.lock().await.is_empty());
    }
}
