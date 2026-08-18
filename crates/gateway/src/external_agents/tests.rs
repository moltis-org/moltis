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
    models: std::sync::Mutex<Vec<Option<String>>>,
    efforts: std::sync::Mutex<Vec<Option<String>>>,
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
        spec: &ExternalAgentSpec,
    ) -> anyhow::Result<Box<dyn ExternalAgentSession>> {
        let start_index = self.state.starts.fetch_add(1, Ordering::SeqCst) + 1;
        self.state
            .models
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(spec.model.clone());
        self.state
            .efforts
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(spec.effort.clone());
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
                usage: (prompt == "usage").then_some(moltis_external_agents::types::TokenUsage {
                    input_tokens: 7,
                    output_tokens: 11,
                }),
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

#[test]
fn context_from_history_includes_project_context() {
    let history = vec![
        PersistedMessage::User {
            content: MessageContent::Text("hello".to_string()),
            created_at: None,
            audio: None,
            documents: None,
            channel: None,
            seq: None,
            run_id: None,
        }
        .to_value(),
    ];

    let context =
        context_from_history_with_project_context(&history, Some("dynamic context".into()));

    assert_eq!(context.project_context.as_deref(), Some("dynamic context"));
    assert_eq!(context.recent_turns.len(), 1);
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
    fake_external_agents_with_config(
        ExternalAgentsConfig {
            enabled: true,
            ..ExternalAgentsConfig::default()
        },
        metadata,
        state,
    )
}

fn fake_external_agents_with_config(
    config: ExternalAgentsConfig,
    metadata: Arc<SqliteSessionMetadata>,
    state: Arc<FakeAgentState>,
) -> Arc<GatewayExternalAgentService> {
    let mut registry = ExternalAgentRegistry::new();
    registry.register(Box::new(FakeTransport { state }));
    Arc::new(GatewayExternalAgentService::with_registry(
        config, metadata, registry,
    ))
}

fn test_gateway_state() -> Arc<GatewayState> {
    test_gateway_state_with_services(GatewayServices::noop())
}

fn test_gateway_state_with_services(services: GatewayServices) -> Arc<GatewayState> {
    GatewayState::new(
        ResolvedAuth {
            mode: AuthMode::Token,
            token: None,
            password: None,
        },
        services,
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

#[derive(Default)]
struct RecordingOutbound {
    messages: Mutex<Vec<(String, String, String, Option<String>)>>,
}

#[async_trait]
impl moltis_channels::ChannelOutbound for RecordingOutbound {
    async fn send_text(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> moltis_channels::Result<()> {
        self.messages.lock().await.push((
            account_id.to_string(),
            to.to_string(),
            text.to_string(),
            reply_to.map(ToOwned::to_owned),
        ));
        Ok(())
    }

    async fn send_media(
        &self,
        _account_id: &str,
        _to: &str,
        _payload: &moltis_common::types::ReplyPayload,
        _reply_to: Option<&str>,
    ) -> moltis_channels::Result<()> {
        Ok(())
    }
}

async fn test_chat_service_with_state(
    external_agents: Arc<GatewayExternalAgentService>,
    metadata: Arc<SqliteSessionMetadata>,
    session_store: Arc<SessionStore>,
    state: Arc<GatewayState>,
) -> ExternalAgentChatService {
    ExternalAgentChatService::new(
        Arc::new(NoopChatService),
        external_agents,
        state,
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
        .bind(serde_json::json!({
            "sessionKey": "main",
            "kind": "codex",
            "model": "gpt-5.2-codex",
            "effort": "xhigh",
        }))
        .await
        .expect("bind external agent");
    assert_eq!(bound["kind"], "codex");
    assert_eq!(bound["model"], "gpt-5.2-codex");
    assert_eq!(bound["effort"], "xhigh");
    assert_eq!(
        bound["modelId"],
        "external-agent::codex::gpt-5.2-codex::xhigh"
    );

    let status = service
        .status(serde_json::json!({ "sessionKey": "main" }))
        .await
        .expect("status");
    assert_eq!(status["bound"], true);
    assert_eq!(status["kind"], "codex");
    assert_eq!(status["model"], "gpt-5.2-codex");
    assert_eq!(status["effort"], "xhigh");
    let entry = metadata.get("main").await.expect("session entry");
    assert_eq!(
        entry.model.as_deref(),
        Some("external-agent::codex::gpt-5.2-codex::xhigh")
    );

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

#[tokio::test]
async fn selected_external_agent_model_is_passed_to_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let session_store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
    let metadata = Arc::new(SqliteSessionMetadata::new(sqlite_pool().await));
    let agent_state = Arc::new(FakeAgentState::default());
    let external_agents = fake_external_agents(Arc::clone(&metadata), Arc::clone(&agent_state));
    external_agents
        .bind(serde_json::json!({
            "sessionKey": "main",
            "kind": "codex",
            "model": "gpt-5.2-codex",
            "effort": "xhigh",
        }))
        .await
        .expect("bind external agent");
    let chat = test_chat_service(
        Arc::clone(&external_agents),
        Arc::clone(&metadata),
        Arc::clone(&session_store),
    )
    .await;

    chat.send(serde_json::json!({ "sessionKey": "main", "text": "hello" }))
        .await
        .expect("send external prompt");

    let models = agent_state
        .models
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let efforts = agent_state
        .efforts
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    assert_eq!(models, vec![Some("gpt-5.2-codex".to_string())]);
    assert_eq!(efforts, vec![Some("xhigh".to_string())]);
}

#[tokio::test]
async fn list_returns_empty_when_external_agents_disabled() {
    let metadata = Arc::new(SqliteSessionMetadata::new(sqlite_pool().await));
    let agent_state = Arc::new(FakeAgentState::default());
    let service = fake_external_agents_with_config(
        ExternalAgentsConfig {
            enabled: false,
            ..ExternalAgentsConfig::default()
        },
        Arc::clone(&metadata),
        Arc::clone(&agent_state),
    );

    let agents = service.list().await.expect("list external agents");

    assert_eq!(agents, serde_json::json!([]));
    assert_eq!(agent_state.starts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn list_uses_default_enabled_external_agent_detection() {
    let metadata = Arc::new(SqliteSessionMetadata::new(sqlite_pool().await));
    let agent_state = Arc::new(FakeAgentState::default());
    let service = fake_external_agents_with_config(
        ExternalAgentsConfig::default(),
        Arc::clone(&metadata),
        Arc::clone(&agent_state),
    );

    let agents = service.list().await.expect("list external agents");

    let codex = agents
        .as_array()
        .expect("list is an array")
        .iter()
        .find(|agent| agent["kind"] == "codex")
        .expect("codex should be in list");
    assert_eq!(codex["name"], "Codex");
    assert_eq!(agent_state.starts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn disabled_external_agents_do_not_route_stale_bindings() {
    let dir = tempfile::tempdir().unwrap();
    let session_store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
    let metadata = Arc::new(SqliteSessionMetadata::new(sqlite_pool().await));
    metadata.upsert("main", None).await.unwrap();
    metadata
        .set_external_agent("main", Some(AgentTransportKind::Codex), None)
        .await;
    let agent_state = Arc::new(FakeAgentState::default());
    let external_agents = fake_external_agents_with_config(
        ExternalAgentsConfig {
            enabled: false,
            ..ExternalAgentsConfig::default()
        },
        Arc::clone(&metadata),
        Arc::clone(&agent_state),
    );
    let chat = test_chat_service(
        Arc::clone(&external_agents),
        Arc::clone(&metadata),
        Arc::clone(&session_store),
    )
    .await;

    let error = chat
        .send(serde_json::json!({ "sessionKey": "main", "text": "hello" }))
        .await
        .expect_err("disabled external agents should fall back to inner chat");

    assert_eq!(error.to_string(), "chat not configured");
    assert_eq!(agent_state.starts.load(Ordering::SeqCst), 0);
    assert!(
        agent_state
            .prompts
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_empty()
    );
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
async fn bound_chat_send_delivers_external_reply_to_channel_target() {
    let dir = tempfile::tempdir().unwrap();
    let session_store = Arc::new(SessionStore::new(dir.path().to_path_buf()));
    let metadata = Arc::new(SqliteSessionMetadata::new(sqlite_pool().await));
    let agent_state = Arc::new(FakeAgentState::default());
    let external_agents = fake_external_agents(Arc::clone(&metadata), Arc::clone(&agent_state));
    external_agents
        .bind(serde_json::json!({ "sessionKey": "telegram:bot:123", "kind": "codex" }))
        .await
        .expect("bind external agent");
    let outbound = Arc::new(RecordingOutbound::default());
    let state = test_gateway_state_with_services(
        GatewayServices::noop().with_channel_outbound(outbound.clone()),
    );
    let chat = test_chat_service_with_state(
        Arc::clone(&external_agents),
        Arc::clone(&metadata),
        Arc::clone(&session_store),
        state,
    )
    .await;

    chat.send(serde_json::json!({
        "sessionKey": "telegram:bot:123",
        "text": "hello",
        "_channel_reply_target": {
            "channel_type": "telegram",
            "account_id": "bot",
            "chat_id": "123",
            "message_id": "456",
            "thread_id": null,
        },
    }))
    .await
    .expect("send external agent turn");

    assert_eq!(*outbound.messages.lock().await, vec![(
        "bot".to_string(),
        "123".to_string(),
        "reply to hello".to_string(),
        Some("456".to_string()),
    )]);
}

#[tokio::test]
async fn full_context_for_bound_external_agent_uses_persisted_history() {
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
        .expect("send external agent prompt");

    let full_context = chat
        .full_context(serde_json::json!({ "_conn_id": "conn-1", "sessionKey": "other" }))
        .await
        .expect("full context");

    assert_eq!(full_context["messageCount"], 2);
    assert_eq!(full_context["messages"][0]["role"], "user");
    assert_eq!(full_context["messages"][0]["content"], "one");
    assert_eq!(full_context["messages"][1]["role"], "assistant");
    assert_eq!(full_context["messages"][1]["content"], "reply to one");
    assert_eq!(full_context["externalAgent"]["kind"], "codex");
    assert_eq!(full_context["externalAgent"]["sessionId"], "fake-session-1");
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

#[test]
fn model_id_round_trips_through_parse() {
    let id = external_agent_model_id("claude-code", Some("opus"), Some("high"));
    let parsed = parse_external_agent_model_id(&id).unwrap();
    assert_eq!(parsed.kind, "claude-code");
    assert_eq!(parsed.model, Some("opus"));
    assert_eq!(parsed.effort, Some("high"));
}

#[test]
fn model_id_kind_only() {
    let id = external_agent_model_id("codex", None, None);
    assert_eq!(id, "external-agent::codex");
    let parsed = parse_external_agent_model_id(&id).unwrap();
    assert_eq!(parsed.kind, "codex");
    assert!(parsed.model.is_none());
    assert!(parsed.effort.is_none());
}

#[test]
fn model_id_with_model_only() {
    let id = external_agent_model_id("claude-code", Some("sonnet"), None);
    let parsed = parse_external_agent_model_id(&id).unwrap();
    assert_eq!(parsed.kind, "claude-code");
    assert_eq!(parsed.model, Some("sonnet"));
    assert!(parsed.effort.is_none());
}

#[test]
fn model_id_effort_only_uses_default_placeholder() {
    let id = external_agent_model_id("codex", None, Some("low"));
    assert!(id.contains("::default::"));
    let parsed = parse_external_agent_model_id(&id).unwrap();
    assert_eq!(parsed.kind, "codex");
    assert!(parsed.model.is_none());
    assert_eq!(parsed.effort, Some("low"));
}

#[test]
fn parse_rejects_non_external_agent_prefix() {
    assert!(parse_external_agent_model_id("openai::gpt-4").is_none());
}

#[test]
fn selected_external_agent_filters_by_kind() {
    let id = external_agent_model_id("claude-code", Some("opus"), None);
    let sel = selected_external_agent(Some(&id), AgentTransportKind::ClaudeCode).unwrap();
    assert_eq!(sel.model.as_deref(), Some("opus"));

    assert!(selected_external_agent(Some(&id), AgentTransportKind::Codex).is_none());
}
