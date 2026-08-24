use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

mod helpers;
mod permissions;
mod security;

use {
    async_trait::async_trait,
    futures::StreamExt,
    moltis_config::schema::ExternalAgentsConfig,
    moltis_external_agents::{
        AcpPermissionHandler, AgentTransportKind, ContextSnapshot, ExternalAgentEvent,
        ExternalAgentRegistry, ExternalAgentSession, ExternalAgentSpec,
        runtimes::{acp::AcpTransport, claude_code::ClaudeCodeTransport, codex::CodexTransport},
        types::ContextTurn,
    },
    moltis_service_traits::{ChatService, ExternalAgentService, ServiceResult, SessionService},
    moltis_sessions::{MessageContent, PersistedMessage},
    serde_json::Value,
    tokio::sync::Mutex,
    tracing::{info, warn},
};

#[cfg(test)]
use {
    moltis_external_agents::{AcpPermissionOptionKind, AcpPermissionRequest},
    permissions::{select_allowed_acp_option, select_rejected_acp_option},
};

use moltis_tools::approval::ApprovalManager;

use crate::{broadcast::BroadcastOpts, state::GatewayState};

use self::{
    helpers::{message_content_text, now_ms},
    permissions::GatewayAcpPermissionHandler,
};

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

impl GatewayExternalAgentService {
    pub fn new(
        config: ExternalAgentsConfig,
        session_metadata: Arc<moltis_sessions::metadata::SqliteSessionMetadata>,
        approval_manager: Arc<ApprovalManager>,
    ) -> Self {
        let mut registry = ExternalAgentRegistry::new();
        registry.register(Box::new(ClaudeCodeTransport::new()));
        registry.register(Box::new(CodexTransport::new()));
        let acp_permission_handler: Arc<dyn AcpPermissionHandler> =
            Arc::new(GatewayAcpPermissionHandler::new(approval_manager));
        for (kind, binary, default_args) in [
            (AgentTransportKind::Acp, "acp", Vec::new()),
            (AgentTransportKind::AcpCopilot, "copilot", vec![
                "--acp".to_string(),
            ]),
            (AgentTransportKind::AcpCodex, "codex-acp", Vec::new()),
            (
                AgentTransportKind::AcpClaude,
                "claude-agent-acp",
                Vec::new(),
            ),
            (AgentTransportKind::AcpPi, "pi-acp", Vec::new()),
            (AgentTransportKind::AcpOpencode, "opencode", vec![
                "acp".to_string(),
            ]),
            (AgentTransportKind::AcpGemini, "gemini", vec![
                "--experimental-acp".to_string(),
            ]),
            (AgentTransportKind::AcpAugment, "auggie", vec![
                "--acp".to_string(),
            ]),
            (AgentTransportKind::AcpKiro, "kiro-cli", vec![
                "acp".to_string(),
            ]),
            (AgentTransportKind::AcpOpenclaw, "openclaw", vec![
                "acp".to_string(),
            ]),
            (AgentTransportKind::AcpOpenhands, "openhands", vec![
                "acp".to_string(),
            ]),
            (AgentTransportKind::AcpKimi, "kimi", vec!["acp".to_string()]),
            (AgentTransportKind::AcpMinimaxCode, "mcode", vec![
                "acp".to_string(),
            ]),
            (AgentTransportKind::AcpStakpak, "stakpak", vec![
                "acp".to_string(),
            ]),
            (
                AgentTransportKind::AcpFastAgent,
                "fast-agent-acp",
                Vec::new(),
            ),
        ] {
            registry.register(Box::new(
                AcpTransport::for_kind(kind, kind.display_name(), binary.to_string())
                    .with_default_args(default_args)
                    .with_permission_handler(acp_permission_handler.clone()),
            ));
        }
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
        let entry = self.session_metadata.get(session_key).await;
        let selected = entry
            .as_ref()
            .and_then(|entry| selected_external_agent(entry.model.as_deref(), kind));
        let spec = self.spec_for_kind(kind, selected)?;
        let mut spec = spec;
        spec.session_key = Some(session_key.to_string());
        spec.external_session_id = entry.and_then(|entry| entry.external_session_id);
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

    fn spec_for_kind(
        &self,
        kind: AgentTransportKind,
        selected: Option<SelectedExternalAgent>,
    ) -> anyhow::Result<ExternalAgentSpec> {
        if !self.config.enabled {
            anyhow::bail!("external agents are disabled")
        }
        let mut spec = ExternalAgentSpec::new(kind);
        if let Some(selected) = selected {
            spec.model = selected.model;
            spec.effort = selected.effort;
        }
        if let Some(agent_config) = self.config.agents.get(kind.as_str()) {
            spec.binary = agent_config.binary.clone();
            if let Some(args) = &agent_config.args {
                spec.args.clone_from(args);
                spec.args_configured = true;
            }
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
        if !self.config.enabled {
            return Ok(serde_json::json!([]));
        }
        let mut agents = serde_json::to_value(self.registry.list_agents().await)
            .unwrap_or_else(|_| serde_json::json!([]));
        if let Some(agents) = agents.as_array_mut() {
            for agent in agents {
                let Some(kind) = agent.get("kind").and_then(Value::as_str) else {
                    continue;
                };
                let models = self
                    .config
                    .agents
                    .get(kind)
                    .map(|config| config.models.clone())
                    .unwrap_or_default();
                let efforts = self
                    .config
                    .agents
                    .get(kind)
                    .map(|config| config.efforts.clone())
                    .unwrap_or_default();
                agent["models"] = serde_json::json!(models);
                agent["efforts"] = serde_json::json!(efforts);
            }
        }
        Ok(agents)
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
        let model = params
            .get("model")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .map(ToOwned::to_owned);
        let effort = params
            .get("effort")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .map(ToOwned::to_owned);
        if !self.registry.has_kind(kind) {
            return Err(format!("external agent kind is not registered: {kind}").into());
        }
        self.shutdown_binding(session_key).await;
        let _ = self.session_metadata.upsert(session_key, None).await;
        self.session_metadata
            .set_external_agent(session_key, Some(kind), None)
            .await;
        let selected_model_id =
            external_agent_model_id(kind.as_str(), model.as_deref(), effort.as_deref());
        self.session_metadata
            .set_model(session_key, Some(selected_model_id.clone()))
            .await;
        Ok(serde_json::json!({
            "ok": true,
            "sessionKey": session_key,
            "kind": kind.as_str(),
            "model": model,
            "effort": effort,
            "modelId": selected_model_id,
        }))
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
            "model": entry.as_ref().and_then(|entry| {
                let kind = entry.external_agent_kind?;
                selected_external_agent(entry.model.as_deref(), kind).and_then(|selected| selected.model)
            }),
            "effort": entry.as_ref().and_then(|entry| {
                let kind = entry.external_agent_kind?;
                selected_external_agent(entry.model.as_deref(), kind).and_then(|selected| selected.effort)
            }),
            "externalSessionId": entry.and_then(|entry| entry.external_session_id),
        }))
    }
}

pub(crate) const EXTERNAL_AGENT_MODEL_PREFIX: &str = "external-agent::";

#[derive(Debug, Clone, PartialEq, Eq)]
struct SelectedExternalAgent {
    model: Option<String>,
    effort: Option<String>,
}

pub(crate) fn external_agent_model_id(
    kind: &str,
    model: Option<&str>,
    effort: Option<&str>,
) -> String {
    match (model, effort) {
        (Some(model), Some(effort)) => {
            format!("{EXTERNAL_AGENT_MODEL_PREFIX}{kind}::{model}::{effort}")
        },
        (Some(model), None) => format!("{EXTERNAL_AGENT_MODEL_PREFIX}{kind}::{model}"),
        (None, Some(effort)) => format!("{EXTERNAL_AGENT_MODEL_PREFIX}{kind}::default::{effort}"),
        (None, None) => format!("{EXTERNAL_AGENT_MODEL_PREFIX}{kind}"),
    }
}

pub(crate) struct ExternalAgentModelSelection<'a> {
    pub kind: &'a str,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
}

pub(crate) fn parse_external_agent_model_id(
    model_id: &str,
) -> Option<ExternalAgentModelSelection<'_>> {
    let suffix = model_id.strip_prefix(EXTERNAL_AGENT_MODEL_PREFIX)?;
    let mut parts = suffix.split("::");
    let kind = parts.next()?;
    let model = parts.next();
    let effort = parts.next();
    Some(ExternalAgentModelSelection {
        kind,
        model: model.filter(|m| *m != "default"),
        effort,
    })
}

fn selected_external_agent(
    selection_id: Option<&str>,
    kind: AgentTransportKind,
) -> Option<SelectedExternalAgent> {
    let sel = parse_external_agent_model_id(selection_id?)?;
    if sel.kind != kind.as_str() {
        return None;
    }
    Some(SelectedExternalAgent {
        model: sel.model.map(ToOwned::to_owned),
        effort: sel.effort.map(ToOwned::to_owned),
    })
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
        if !self.external_agents.config.enabled {
            return None;
        }
        if security::is_explicit_shell_request(params) {
            return None;
        }
        let session_key = resolve_session_key(params, &self.state).await;
        let entry = self.session_metadata.get(&session_key).await?;
        let kind = entry.external_agent_kind?;
        if !security::allows_external_agent_request(params, entry.channel_binding.is_some()) {
            return Some(Err(
                "external agents are unavailable for public or tool-restricted turns".into(),
            ));
        }
        Some(self.send_external(params.clone(), session_key, kind).await)
    }

    /// Resolve the directory the configured `context_command` should run in for
    /// this session: the session worktree when present, else the bound project
    /// directory. Returns `None` when no project is bound, in which case the
    /// command inherits the server process's current directory.
    async fn resolve_context_working_dir(&self, session_key: &str) -> Option<PathBuf> {
        let entry = self.session_metadata.get(session_key).await?;
        let pid = entry.project_id?;
        let val = self
            .state
            .services
            .project
            .get(serde_json::json!({ "id": pid }))
            .await
            .ok()?;
        let dir = val.get("directory").and_then(|v| v.as_str())?;
        let worktree = entry.worktree_branch.as_ref().and_then(|_| {
            let wt = Path::new(dir).join(".moltis-worktrees").join(session_key);
            wt.exists().then_some(wt)
        });
        Some(worktree.unwrap_or_else(|| PathBuf::from(dir)))
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
        let channel_reply_target = params
            .get("_channel_reply_target")
            .cloned()
            .and_then(|value| {
                match serde_json::from_value::<moltis_channels::ChannelReplyTarget>(value) {
                    Ok(target) => Some(target),
                    Err(error) => {
                        warn!(
                            session = %session_key,
                            %error,
                            "ignoring invalid external-agent channel reply target"
                        );
                        None
                    },
                }
            });
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
        let selected = self
            .session_metadata
            .get(&session_key)
            .await
            .and_then(|entry| selected_external_agent(entry.model.as_deref(), kind));
        let selected_model = selected
            .as_ref()
            .and_then(|selected| selected.model.as_deref());
        let selected_effort = selected
            .as_ref()
            .and_then(|selected| selected.effort.as_deref());
        let message_model = selected_model.unwrap_or_else(|| kind.as_str());

        crate::broadcast::broadcast(
            &self.state,
            "chat",
            serde_json::json!({
                "runId": run_id,
                "sessionKey": session_key,
                "state": "running",
                "model": message_model,
                "provider": "external-agent",
                "seq": seq,
            }),
            BroadcastOpts::default(),
        )
        .await;

        let context_working_dir = self.resolve_context_working_dir(&session_key).await;
        let context_command_output = moltis_common::context_command::run_context_command(
            self.state.config.chat.context_command.as_deref(),
            context_working_dir.as_deref(),
        )
        .await;
        let context = context_from_history_with_project_context(&history, context_command_output);
        let start = std::time::Instant::now();
        info!(
            session = %session_key,
            kind = kind.as_str(),
            model = message_model,
            effort = selected_effort.unwrap_or("default"),
            run_id,
            text_len = text.len(),
            channel_reply = channel_reply_target.is_some(),
            "external-agent turn starting"
        );
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
            self.broadcast_external_agent_session_update(&session_key)
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
        let mut thinking_text = String::new();
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
                    thinking_text.push_str(&delta);
                    crate::broadcast::broadcast(
                        &self.state,
                        "chat",
                        serde_json::json!({
                            "runId": run_id,
                            "sessionKey": session_key,
                            "state": "thinking_text",
                            "text": thinking_text,
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
            self.broadcast_external_agent_session_update(&session_key)
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
            model: Some(message_model.to_string()),
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
                "model": message_model,
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
        deliver_external_agent_channel_reply(
            &self.state,
            channel_reply_target,
            &assistant_text,
            &session_key,
        )
        .await;
        info!(
            session = %session_key,
            kind = kind.as_str(),
            model = message_model,
            effort = selected_effort.unwrap_or("default"),
            run_id,
            text_len = assistant_text.len(),
            duration_ms,
            "external-agent turn completed"
        );
        Ok(serde_json::json!({
            "ok": true,
            "runId": run_id,
            "text": assistant_text,
            "inputTokens": token_usage.as_ref().map(|usage| usage.input_tokens).unwrap_or(0),
            "outputTokens": token_usage.as_ref().map(|usage| usage.output_tokens).unwrap_or(0),
            "durationMs": duration_ms,
        }))
    }
}

async fn deliver_external_agent_channel_reply(
    state: &GatewayState,
    target: Option<moltis_channels::ChannelReplyTarget>,
    text: &str,
    session_key: &str,
) {
    let Some(target) = target else {
        return;
    };
    if text.trim().is_empty() {
        info!(
            session_key,
            account_id = target.account_id,
            chat_id = target.chat_id,
            "external-agent channel reply skipped: empty response text"
        );
        return;
    }
    let Some(outbound) = state.services.channel_outbound_arc() else {
        warn!(
            session_key,
            account_id = target.account_id,
            chat_id = target.chat_id,
            "external-agent channel reply skipped: outbound unavailable"
        );
        return;
    };
    let to = target.outbound_to().into_owned();
    if let Err(error) = outbound
        .send_text(&target.account_id, &to, text, target.message_id.as_deref())
        .await
    {
        warn!(
            session_key,
            account_id = target.account_id,
            chat_id = target.chat_id,
            thread_id = target.thread_id.as_deref().unwrap_or("-"),
            %error,
            "external-agent channel reply failed"
        );
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
        if let Some(result) = self.maybe_send_external(&params).await {
            return result;
        }
        self.inner.send_sync(params).await
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
        if let Some(context) = self.external_full_context(&params).await {
            return context;
        }
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

impl ExternalAgentChatService {
    async fn broadcast_external_agent_session_update(&self, session_key: &str) {
        let Some(entry) = self.session_metadata.get(session_key).await else {
            return;
        };
        crate::broadcast::broadcast(
            &self.state,
            "session",
            serde_json::json!({
                "kind": "patched",
                "sessionKey": session_key,
                "entry": {
                    "key": session_key,
                    "externalAgentKind": entry.external_agent_kind.map(|kind| kind.as_str()),
                    "externalSessionId": entry.external_session_id,
                    "version": entry.version,
                },
            }),
            BroadcastOpts::default(),
        )
        .await;
    }

    async fn external_full_context(&self, params: &Value) -> Option<ServiceResult> {
        if !self.external_agents.config.enabled {
            return None;
        }
        let session_key = resolve_connection_session_key(params, &self.state).await;
        let entry = self.session_metadata.get(&session_key).await?;
        let kind = entry.external_agent_kind?;
        let history = self
            .session_store
            .read(&session_key)
            .await
            .unwrap_or_default();
        let context = context_from_history_with_project_context(&history, None);
        let messages: Vec<Value> = context
            .recent_turns
            .iter()
            .map(|turn| {
                serde_json::json!({
                    "role": turn.role,
                    "content": turn.content,
                })
            })
            .collect();
        let llm_outputs: Vec<Value> = history
            .iter()
            .filter(|entry| entry.get("role").and_then(|role| role.as_str()) == Some("assistant"))
            .cloned()
            .collect();
        let total_chars = messages
            .iter()
            .map(|message| serde_json::to_string(message).unwrap_or_default().len())
            .sum::<usize>();

        Some(Ok(serde_json::json!({
            "messages": messages,
            "llmOutputs": llm_outputs,
            "messageCount": context.recent_turns.len(),
            "systemPromptChars": 0,
            "totalChars": total_chars,
            "truncated": history.len() > context.recent_turns.len(),
            "workspaceFiles": [],
            "promptMemory": null,
            "externalAgent": {
                "kind": kind.as_str(),
                "sessionId": entry.external_session_id,
            },
        })))
    }
}

async fn resolve_connection_session_key(params: &Value, state: &GatewayState) -> String {
    let Some(conn_id) = params.get("_conn_id").and_then(|value| value.as_str()) else {
        return "main".to_string();
    };
    let registry = state.client_registry.read().await;
    registry
        .active_sessions
        .get(conn_id)
        .cloned()
        .unwrap_or_else(|| "main".to_string())
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

fn context_from_history_with_project_context(
    history: &[Value],
    project_context: Option<String>,
) -> ContextSnapshot {
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
        project_context,
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

#[cfg(test)]
#[path = "external_agents/registry_tests.rs"]
mod registry_tests;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "external_agents/tests.rs"]
mod tests;
