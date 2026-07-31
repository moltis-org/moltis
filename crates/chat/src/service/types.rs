//! `LiveChatService` struct, constructors, and helper methods.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    task::Poll,
    time::Duration,
};

use {
    futures::{
        FutureExt,
        future::{BoxFuture, Shared},
    },
    serde::Serialize,
    serde_json::Value,
    tokio::{
        sync::{AcquireError, OwnedSemaphorePermit, RwLock, Semaphore},
        task::{AbortHandle, JoinHandle},
    },
    tracing::warn,
};

use {
    moltis_agents::tool_registry::ToolRegistry,
    moltis_providers::ProviderRegistry,
    moltis_service_traits::ServiceError,
    moltis_sessions::{
        PersistedMessage,
        message::{PersistedFunction, PersistedToolCall},
        metadata::SqliteSessionMetadata,
        state_store::SessionStateStore,
        store::SessionStore,
    },
};

use crate::{error, models::DisabledModelsStore, runtime::ChatRuntime, types::*};

/// A message that arrived while an agent run was already active on the session.
#[derive(Debug, Clone)]
pub(in crate::service) struct QueuedMessage {
    pub(in crate::service) params: Value,
}

/// Per-session FIFO plus an in-progress replay reservation.
#[derive(Debug, Default)]
pub(in crate::service) struct SessionMessageQueue {
    pub(in crate::service) messages: VecDeque<QueuedMessage>,
    pub(in crate::service) draining: bool,
}

pub(in crate::service) enum TurnAdmission {
    Acquired(OwnedSemaphorePermit),
    Queued(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::service) enum RunHandleDisposition {
    Aborted,
    Stale,
    Unavailable,
}

type CleanupPermitFuture =
    Pin<Box<dyn Future<Output = Result<OwnedSemaphorePermit, AcquireError>> + Send>>;

pub(in crate::service) enum CleanupPermitReservation {
    Acquired(Result<OwnedSemaphorePermit, AcquireError>),
    Waiting(CleanupPermitFuture),
}

impl CleanupPermitReservation {
    pub(in crate::service) async fn acquire(self) -> Result<OwnedSemaphorePermit, AcquireError> {
        match self {
            Self::Acquired(result) => result,
            Self::Waiting(future) => future.await,
        }
    }
}

pub(in crate::service) struct AbortRunClaim {
    pub(in crate::service) disposition: RunHandleDisposition,
    pub(in crate::service) cleanup: Option<CleanupPermitReservation>,
}

pub(crate) async fn commit_terminal_run(
    terminal_runs: &Arc<RwLock<HashSet<String>>>,
    run_id: &str,
) {
    terminal_runs.write().await.insert(run_id.to_string());
}

pub(in crate::service) async fn commit_successful_turn<P, B>(
    terminal_runs: &Arc<RwLock<HashSet<String>>>,
    run_id: &str,
    persistence: P,
    final_broadcast: B,
) where
    P: Future<Output = ()>,
    B: Future<Output = ()>,
{
    commit_terminal_run(terminal_runs, run_id).await;
    persistence.await;
    final_broadcast.await;
}

#[derive(Clone)]
pub(crate) struct EventForwarder {
    completion: Shared<BoxFuture<'static, String>>,
    abort_handle: AbortHandle,
}

impl EventForwarder {
    pub(crate) fn new(task: JoinHandle<String>, session_key: String) -> Self {
        let abort_handle = task.abort_handle();
        let completion = async move {
            match task.await {
                Ok(reasoning) => reasoning,
                Err(error) => {
                    warn!(
                        session = %session_key,
                        error = %error,
                        "runner event forwarder task failed"
                    );
                    String::new()
                },
            }
        }
        .boxed()
        .shared();
        Self {
            completion,
            abort_handle,
        }
    }

    async fn wait(&self) -> String {
        self.completion.clone().await
    }

    fn abort(&self) {
        self.abort_handle.abort();
    }
}

/// A tool call currently executing within an active agent run.
#[derive(Debug, Clone, Serialize)]
pub struct ActiveToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
    #[serde(rename = "startedAt")]
    pub started_at: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct ActiveAssistantDraft {
    content: String,
    reasoning: String,
    model: String,
    provider: String,
    seq: Option<u64>,
    run_id: String,
}

impl ActiveAssistantDraft {
    pub(crate) fn new(run_id: &str, model: &str, provider: &str, seq: Option<u64>) -> Self {
        Self {
            content: String::new(),
            reasoning: String::new(),
            model: model.to_string(),
            provider: provider.to_string(),
            seq,
            run_id: run_id.to_string(),
        }
    }

    pub(crate) fn append_text(&mut self, delta: &str) {
        if !delta.is_empty() {
            self.content.push_str(delta);
        }
    }

    pub(crate) fn set_reasoning(&mut self, reasoning: &str) {
        self.reasoning.clear();
        self.reasoning.push_str(reasoning);
    }

    pub(crate) fn has_visible_content(&self) -> bool {
        !self.content.trim().is_empty() || !self.reasoning.trim().is_empty()
    }

    pub(crate) fn to_persisted_message(&self) -> PersistedMessage {
        let reasoning = self.reasoning.trim();
        PersistedMessage::Assistant {
            content: self.content.clone(),
            created_at: Some(now_ms()),
            model: Some(self.model.clone()),
            provider: Some(self.provider.clone()),
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            duration_ms: None,
            request_input_tokens: None,
            request_output_tokens: None,
            request_cache_read_tokens: None,
            request_cache_write_tokens: None,
            tool_calls: None,
            reasoning: (!reasoning.is_empty()).then(|| reasoning.to_string()),
            llm_api_response: None,
            audio: None,
            seq: self.seq,
            run_id: Some(self.run_id.clone()),
        }
    }
}

fn build_persisted_tool_call(
    tool_call_id: impl Into<String>,
    tool_name: impl Into<String>,
    arguments: Option<Value>,
    metadata: Option<serde_json::Map<String, Value>>,
) -> PersistedToolCall {
    PersistedToolCall {
        id: tool_call_id.into(),
        call_type: "function".to_string(),
        function: PersistedFunction {
            name: tool_name.into(),
            arguments: arguments
                .unwrap_or_else(|| serde_json::json!({}))
                .to_string(),
        },
        metadata,
    }
}

pub(crate) fn build_tool_call_assistant_message(
    tool_call_id: impl Into<String>,
    tool_name: impl Into<String>,
    arguments: Option<Value>,
    metadata: Option<serde_json::Map<String, Value>>,
    seq: Option<u64>,
    run_id: Option<&str>,
) -> PersistedMessage {
    PersistedMessage::Assistant {
        content: String::new(),
        created_at: Some(now_ms()),
        model: None,
        provider: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
        duration_ms: None,
        request_input_tokens: None,
        request_output_tokens: None,
        request_cache_read_tokens: None,
        request_cache_write_tokens: None,
        tool_calls: Some(vec![build_persisted_tool_call(
            tool_call_id,
            tool_name,
            arguments,
            metadata,
        )]),
        reasoning: None,
        llm_api_response: None,
        audio: None,
        seq,
        run_id: run_id.map(str::to_string),
    }
}

pub(crate) fn build_persisted_assistant_message(
    assistant_output: AssistantTurnOutput,
    model: Option<String>,
    provider: Option<String>,
    seq: Option<u64>,
    run_id: Option<String>,
) -> PersistedMessage {
    PersistedMessage::Assistant {
        content: assistant_output.text,
        created_at: Some(now_ms()),
        model,
        provider,
        input_tokens: Some(assistant_output.input_tokens),
        output_tokens: Some(assistant_output.output_tokens),
        cache_read_tokens: Some(assistant_output.cache_read_tokens),
        cache_write_tokens: Some(assistant_output.cache_write_tokens),
        duration_ms: Some(assistant_output.duration_ms),
        request_input_tokens: Some(assistant_output.request_input_tokens),
        request_output_tokens: Some(assistant_output.request_output_tokens),
        request_cache_read_tokens: Some(assistant_output.request_cache_read_tokens),
        request_cache_write_tokens: Some(assistant_output.request_cache_write_tokens),
        tool_calls: None,
        reasoning: assistant_output.reasoning,
        llm_api_response: assistant_output.llm_api_response,
        audio: assistant_output.audio_path,
        seq,
        run_id,
    }
}

pub(crate) async fn persist_tool_history_pair(
    session_store: &Arc<SessionStore>,
    session_key: &str,
    assistant_tool_call_msg: PersistedMessage,
    tool_result_msg: PersistedMessage,
    assistant_warn_context: &str,
    tool_result_warn_context: &str,
) {
    if let Err(e) = session_store
        .append(session_key, &assistant_tool_call_msg.to_value())
        .await
    {
        warn!("{assistant_warn_context}: {e}");
        warn!(
            session = %session_key,
            "skipping tool result persistence to avoid orphaned tool history"
        );
        return;
    }

    if let Err(e) = session_store
        .append(session_key, &tool_result_msg.to_value())
        .await
    {
        warn!("{tool_result_warn_context}: {e}");
    }
}

pub struct LiveChatService {
    pub(in crate::service) providers: Arc<RwLock<ProviderRegistry>>,
    pub(in crate::service) model_store: Arc<RwLock<DisabledModelsStore>>,
    pub(in crate::service) state: Arc<dyn ChatRuntime>,
    pub(in crate::service) active_runs: Arc<RwLock<HashMap<String, AbortHandle>>>,
    pub(in crate::service) active_runs_by_session: Arc<RwLock<HashMap<String, String>>>,
    pub(in crate::service) active_event_forwarders: Arc<RwLock<HashMap<String, EventForwarder>>>,
    pub(in crate::service) terminal_runs: Arc<RwLock<HashSet<String>>>,
    pub(in crate::service) tool_registry: Arc<RwLock<ToolRegistry>>,
    pub(in crate::service) session_tool_overlays:
        Arc<RwLock<HashMap<String, Arc<RwLock<ToolRegistry>>>>>,
    pub(in crate::service) session_store: Arc<SessionStore>,
    pub(in crate::service) session_metadata: Arc<SqliteSessionMetadata>,
    pub(in crate::service) session_state_store: Option<Arc<SessionStateStore>>,
    pub(in crate::service) hook_registry: Option<Arc<moltis_common::hooks::HookRegistry>>,
    /// Per-session semaphore ensuring only one agent run executes per session at a time.
    pub(in crate::service) session_locks: Arc<RwLock<HashMap<String, Arc<Semaphore>>>>,
    /// Per-session message queue for messages arriving during an active run.
    pub(in crate::service) message_queue: Arc<RwLock<HashMap<String, SessionMessageQueue>>>,
    /// Per-session last-seen client sequence number for ordering diagnostics.
    pub(in crate::service) last_client_seq: Arc<RwLock<HashMap<String, u64>>>,
    /// Per-session accumulated thinking text for active runs, so it can be
    /// returned in `sessions.switch` after a page reload.
    pub(in crate::service) active_thinking_text: Arc<RwLock<HashMap<String, String>>>,
    /// Per-session active tool calls for `chat.peek` snapshot.
    pub(in crate::service) active_tool_calls: Arc<RwLock<HashMap<String, Vec<ActiveToolCall>>>>,
    /// Per-session streamed assistant content buffered so an abort can persist
    /// what the user already saw instead of dropping it on the floor.
    pub(in crate::service) active_partial_assistant:
        Arc<RwLock<HashMap<String, ActiveAssistantDraft>>>,
    /// Per-session reply medium for active runs, so the frontend can restore
    /// `voicePending` state after a page reload.
    pub(in crate::service) active_reply_medium: Arc<RwLock<HashMap<String, ReplyMedium>>>,
    /// Startup configuration snapshot for chat hot-path decisions.
    pub(in crate::service) config: moltis_config::MoltisConfig,
    /// Failover configuration for automatic model/provider failover.
    pub(in crate::service) failover_config: moltis_config::schema::FailoverConfig,
}

impl LiveChatService {
    pub(in crate::service) async fn load_turn_history(
        &self,
        session_key: &str,
        limits: Option<(usize, usize)>,
    ) -> Result<Vec<Value>, ServiceError> {
        let Some((max_messages, max_bytes)) = limits else {
            return Ok(self
                .session_store
                .read(session_key)
                .await
                .unwrap_or_default());
        };
        self.session_store
            .read_bounded(session_key, max_messages, max_bytes)
            .await
            .map_err(|error| {
                ServiceError::message(format!("failed to read session history: {error}"))
            })
    }

    /// Reads persisted history with strict pre-allocation bounds for non-UI
    /// protocol surfaces such as ACP session replay.
    pub async fn read_session_history_bounded(
        &self,
        session_key: &str,
        max_messages: usize,
        max_bytes: usize,
    ) -> anyhow::Result<Vec<Value>> {
        self.session_store
            .read_bounded(session_key, max_messages, max_bytes)
            .await
            .map_err(Into::into)
    }

    pub fn new(
        providers: Arc<RwLock<ProviderRegistry>>,
        model_store: Arc<RwLock<DisabledModelsStore>>,
        state: Arc<dyn ChatRuntime>,
        session_store: Arc<SessionStore>,
        session_metadata: Arc<SqliteSessionMetadata>,
    ) -> Self {
        Self {
            providers,
            model_store,
            state,
            active_runs: Arc::new(RwLock::new(HashMap::new())),
            active_runs_by_session: Arc::new(RwLock::new(HashMap::new())),
            active_event_forwarders: Arc::new(RwLock::new(HashMap::new())),
            terminal_runs: Arc::new(RwLock::new(HashSet::new())),
            tool_registry: Arc::new(RwLock::new(ToolRegistry::new())),
            session_tool_overlays: Arc::new(RwLock::new(HashMap::new())),
            session_store,
            session_metadata,
            session_state_store: None,
            hook_registry: None,
            session_locks: Arc::new(RwLock::new(HashMap::new())),
            message_queue: Arc::new(RwLock::new(HashMap::new())),
            last_client_seq: Arc::new(RwLock::new(HashMap::new())),
            active_thinking_text: Arc::new(RwLock::new(HashMap::new())),
            active_tool_calls: Arc::new(RwLock::new(HashMap::new())),
            active_partial_assistant: Arc::new(RwLock::new(HashMap::new())),
            active_reply_medium: Arc::new(RwLock::new(HashMap::new())),
            config: moltis_config::discover_and_load(),
            failover_config: moltis_config::schema::FailoverConfig::default(),
        }
    }

    pub fn with_config(mut self, config: moltis_config::MoltisConfig) -> Self {
        self.config = config;
        self
    }

    pub fn with_failover(mut self, config: moltis_config::schema::FailoverConfig) -> Self {
        self.failover_config = config;
        self
    }

    pub fn with_tools(mut self, registry: Arc<RwLock<ToolRegistry>>) -> Self {
        self.tool_registry = registry;
        self
    }

    pub async fn set_session_tool_overlay(
        &self,
        session_key: &str,
        registry: Arc<RwLock<ToolRegistry>>,
    ) {
        self.session_tool_overlays
            .write()
            .await
            .insert(session_key.to_string(), registry);
    }

    pub async fn remove_session_tool_overlay(&self, session_key: &str) {
        self.session_tool_overlays.write().await.remove(session_key);
    }

    pub async fn bind_session_project(&self, session_key: &str, project_id: &str) {
        let _ = self.session_metadata.upsert(session_key, None).await;
        self.session_metadata
            .set_project_id(session_key, Some(project_id.to_string()))
            .await;
    }

    pub async fn session_exists(&self, session_key: &str) -> bool {
        self.session_metadata.get(session_key).await.is_some()
    }

    pub fn with_session_state_store(mut self, store: Arc<SessionStateStore>) -> Self {
        self.session_state_store = Some(store);
        self
    }

    pub fn with_hooks(mut self, registry: moltis_common::hooks::HookRegistry) -> Self {
        self.hook_registry = Some(Arc::new(registry));
        self
    }

    pub fn with_hooks_arc(mut self, registry: Arc<moltis_common::hooks::HookRegistry>) -> Self {
        self.hook_registry = Some(registry);
        self
    }

    pub(in crate::service) fn has_tools_sync(&self) -> bool {
        // Best-effort check: try_read avoids blocking. If the lock is held,
        // assume tools are present (conservative — enables tool mode).
        self.tool_registry
            .try_read()
            .map(|r| {
                let schemas = r.list_schemas();
                let has = !schemas.is_empty();
                tracing::debug!(
                    tool_count = schemas.len(),
                    has_tools = has,
                    "has_tools_sync check"
                );
                has
            })
            .unwrap_or(true)
    }

    /// Return the per-session semaphore, creating one if absent.
    pub(in crate::service) async fn session_semaphore(&self, key: &str) -> Arc<Semaphore> {
        // Fast path: read lock.
        {
            let locks = self.session_locks.read().await;
            if let Some(sem) = locks.get(key) {
                return Arc::clone(sem);
            }
        }
        // Slow path: write lock, insert.
        let mut locks = self.session_locks.write().await;
        Arc::clone(
            locks
                .entry(key.to_string())
                .or_insert_with(|| Arc::new(Semaphore::new(1))),
        )
    }

    /// Arbitrate the permit and FIFO under one queue lock. Ordinary arrivals
    /// cannot pass queued work, while the drainer's replay can consume its
    /// reservation. If that replay cannot acquire, it returns to the front.
    pub(in crate::service) async fn admit_turn(
        &self,
        session_key: &str,
        params: Value,
        queued_replay: bool,
    ) -> TurnAdmission {
        let session_sem = self.session_semaphore(session_key).await;
        admit_queued_turn(
            &self.message_queue,
            session_sem,
            session_key,
            params,
            queued_replay,
        )
        .await
    }

    pub(in crate::service) async fn claim_run_for_abort(
        active_runs: &Arc<RwLock<HashMap<String, AbortHandle>>>,
        active_runs_by_session: &Arc<RwLock<HashMap<String, String>>>,
        terminal_runs: &Arc<RwLock<HashSet<String>>>,
        session_sem: Arc<Semaphore>,
        run_id: Option<&str>,
        session_key: Option<&str>,
    ) -> (Option<String>, AbortRunClaim) {
        let terminal = terminal_runs.read().await;
        let mut by_session = active_runs_by_session.write().await;
        let target = match (run_id, session_key) {
            (Some(id), Some(key)) if by_session.get(key).is_some_and(|active| active == id) => {
                Some((id.to_string(), key.to_string()))
            },
            (Some(_), Some(_)) => None,
            (Some(id), None) => by_session
                .iter()
                .find_map(|(key, active)| (active == id).then(|| (id.to_string(), key.clone()))),
            (None, Some(key)) => by_session.get(key).cloned().map(|id| (id, key.to_string())),
            (None, None) => None,
        };
        let Some((target_run_id, target_session_key)) = target else {
            return (None, AbortRunClaim {
                disposition: RunHandleDisposition::Unavailable,
                cleanup: None,
            });
        };

        let mut runs = active_runs.write().await;
        if terminal.contains(&target_run_id) {
            return (Some(target_run_id), AbortRunClaim {
                disposition: RunHandleDisposition::Unavailable,
                cleanup: None,
            });
        }

        let disposition = match runs.get(&target_run_id) {
            Some(handle) if handle.is_finished() => RunHandleDisposition::Stale,
            None => RunHandleDisposition::Stale,
            Some(_) => RunHandleDisposition::Aborted,
        };

        // Establish ownership before registering the fair waiter. Holding the
        // terminal and run-map locks makes abort versus successful terminal
        // transition a single-winner race; an unavailable request never
        // reserves the session permit.
        let mut cleanup_future: CleanupPermitFuture = Box::pin(session_sem.acquire_owned());
        let mut immediate = None;
        std::future::poll_fn(|cx| {
            if let Poll::Ready(result) = cleanup_future.as_mut().poll(cx) {
                immediate = Some(result);
            }
            Poll::Ready(())
        })
        .await;
        let cleanup = match immediate {
            Some(result) => CleanupPermitReservation::Acquired(result),
            None => CleanupPermitReservation::Waiting(cleanup_future),
        };

        if disposition == RunHandleDisposition::Aborted
            && let Some(handle) = runs.get(&target_run_id)
        {
            handle.abort();
        }
        runs.remove(&target_run_id);
        if by_session.get(&target_session_key) == Some(&target_run_id) {
            by_session.remove(&target_session_key);
        }

        (Some(target_run_id), AbortRunClaim {
            disposition,
            cleanup: Some(cleanup),
        })
    }

    pub(in crate::service) async fn resolve_session_key_for_run(
        active_runs_by_session: &Arc<RwLock<HashMap<String, String>>>,
        run_id: Option<&str>,
        session_key: Option<&str>,
    ) -> Option<String> {
        let by_session = active_runs_by_session.read().await;
        match (run_id, session_key) {
            (Some(id), Some(key)) => by_session
                .get(key)
                .is_some_and(|active| active == id)
                .then(|| key.to_string()),
            (Some(id), None) => by_session
                .iter()
                .find_map(|(key, active)| (active == id).then(|| key.clone())),
            (None, Some(key)) => by_session.contains_key(key).then(|| key.to_string()),
            (None, None) => None,
        }
    }

    pub(crate) async fn wait_for_event_forwarder(
        active_event_forwarders: &Arc<RwLock<HashMap<String, EventForwarder>>>,
        run_id: &str,
    ) -> String {
        let completion = active_event_forwarders.read().await.get(run_id).cloned();
        let Some(completion) = completion else {
            return String::new();
        };

        let reasoning = completion.wait().await;
        active_event_forwarders.write().await.remove(run_id);
        reasoning
    }

    pub(in crate::service) async fn cancel_event_forwarder(
        active_event_forwarders: &Arc<RwLock<HashMap<String, EventForwarder>>>,
        run_id: &str,
    ) {
        let forwarder = active_event_forwarders.write().await.remove(run_id);
        let Some(forwarder) = forwarder else {
            return;
        };
        forwarder.abort();
        if tokio::time::timeout(Duration::from_secs(1), forwarder.wait())
            .await
            .is_err()
        {
            warn!(run_id, "timed out waiting for cancelled event forwarder");
        }
    }

    pub(in crate::service) async fn register_run_handle(
        active_runs: &Arc<RwLock<HashMap<String, AbortHandle>>>,
        active_runs_by_session: &Arc<RwLock<HashMap<String, String>>>,
        run_id: &str,
        session_key: &str,
        abort_handle: AbortHandle,
    ) {
        // Keep the same lock order as claim_run_for_abort so registration is atomic
        // from an abort caller's perspective.
        let mut by_session = active_runs_by_session.write().await;
        let mut runs = active_runs.write().await;
        by_session.insert(session_key.to_string(), run_id.to_string());
        runs.insert(run_id.to_string(), abort_handle);
    }

    pub(in crate::service) async fn persist_partial_assistant_on_abort(
        &self,
        session_key: &str,
        run_id: &str,
    ) -> Option<(Value, Option<u32>)> {
        let partial = {
            let mut partials = self.active_partial_assistant.write().await;
            if !partials
                .get(session_key)
                .is_some_and(|partial| partial.run_id == run_id)
            {
                return None;
            }
            partials.remove(session_key)?
        };
        if !partial.has_visible_content() {
            return None;
        }

        let partial_message = partial.to_persisted_message();
        let partial_value = partial_message.to_value();
        let mut message_index = None;

        if let Err(e) = self.session_store.append(session_key, &partial_value).await {
            warn!(session = %session_key, error = %e, "failed to persist aborted partial assistant message");
            return Some((partial_value, None));
        }

        match self.session_store.count(session_key).await {
            Ok(count) => {
                self.session_metadata.touch(session_key, count).await;
                message_index = Some(count.saturating_sub(1));
            },
            Err(e) => {
                warn!(session = %session_key, error = %e, "failed to count session after persisting aborted partial assistant message");
            },
        }

        Some((partial_value, message_index))
    }

    /// Resolve a provider from session metadata, history, or first registered.
    pub(in crate::service) async fn resolve_provider(
        &self,
        session_key: &str,
        history: &[Value],
    ) -> error::Result<Arc<dyn moltis_agents::model::LlmProvider>> {
        let reg = self.providers.read().await;
        let session_model = self
            .session_metadata
            .get(session_key)
            .await
            .and_then(|e| e.model.clone());
        let history_model = history
            .iter()
            .rev()
            .find_map(|m| m.get("model").and_then(|v| v.as_str()).map(String::from));
        let model_id = session_model.or(history_model);

        model_id
            .and_then(|id| reg.get(&id))
            .or_else(|| reg.first())
            .ok_or_else(|| error::Error::message("no LLM providers configured"))
    }

    /// Resolve the active session key for a connection.
    pub(in crate::service) async fn session_key_for(&self, conn_id: Option<&str>) -> String {
        if let Some(cid) = conn_id
            && let Some(key) = self.state.active_session_key(cid).await
        {
            return key;
        }
        "main".to_string()
    }

    /// Resolve the effective session key for chat operations.
    ///
    /// Precedence is:
    /// 1. Internal `_session_key` overrides used by runtime-owned callers.
    /// 2. Public `sessionKey` / `session_key` request parameters.
    /// 3. Connection-scoped active session derived from `_conn_id`.
    /// 4. The default `"main"` session.
    pub(in crate::service) async fn resolve_session_key_from_params(
        &self,
        params: &Value,
    ) -> String {
        if let Some(session_key) = params
            .get("_session_key")
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
        {
            return session_key.to_string();
        }
        if let Some(session_key) = params
            .get("sessionKey")
            .or_else(|| params.get("session_key"))
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
        {
            return session_key.to_string();
        }
        let conn_id = params.get("_conn_id").and_then(|v| v.as_str());
        self.session_key_for(conn_id).await
    }

    /// Resolve the project context prompt section and effective working
    /// directory for a session.
    ///
    /// The working directory is the session worktree when present, otherwise
    /// the bound project directory; it is `None` when no project is bound. It
    /// is used to run the configured `context_command` in the expected place.
    pub(in crate::service) async fn resolve_project_context(
        &self,
        session_key: &str,
        conn_id: Option<&str>,
    ) -> (Option<String>, Option<PathBuf>) {
        let project_id = if let Some(cid) = conn_id {
            self.state.active_project_id(cid).await
        } else {
            None
        };
        // Also check session metadata for project binding (async path).
        let project_id = match project_id {
            Some(pid) => Some(pid),
            None => self
                .session_metadata
                .get(session_key)
                .await
                .and_then(|e| e.project_id),
        };

        let Some(pid) = project_id else {
            return (None, None);
        };
        let Ok(val) = self
            .state
            .project_service()
            .get(serde_json::json!({"id": pid}))
            .await
        else {
            return (None, None);
        };
        let Some(dir) = val.get("directory").and_then(|v| v.as_str()) else {
            return (None, None);
        };
        let files = match moltis_projects::context::load_context_files(Path::new(dir)) {
            Ok(f) => f,
            Err(e) => {
                warn!("failed to load project context: {e}");
                return (None, None);
            },
        };
        let Ok(project) = serde_json::from_value::<moltis_projects::Project>(val.clone()) else {
            return (None, None);
        };
        let worktree_dir = self
            .session_metadata
            .get(session_key)
            .await
            .and_then(|e| e.worktree_branch)
            .and_then(|_| {
                let wt_path = Path::new(dir).join(".moltis-worktrees").join(session_key);
                if wt_path.exists() {
                    Some(wt_path)
                } else {
                    None
                }
            });
        // The command runs in the session worktree when present, else the
        // project root — matching where the operator's scripts expect to be.
        let working_dir = worktree_dir.clone().unwrap_or_else(|| PathBuf::from(dir));
        let ctx = moltis_projects::ProjectContext {
            project,
            context_files: files,
            worktree_dir,
        };
        (Some(ctx.to_prompt_section()), Some(working_dir))
    }

    /// Resolve all dynamic prompt context for a turn.
    pub(in crate::service) async fn resolve_turn_context(
        &self,
        session_key: &str,
        conn_id: Option<&str>,
    ) -> (Option<String>, Option<PathBuf>) {
        let (project_context, working_dir) =
            self.resolve_project_context(session_key, conn_id).await;
        let command_context = moltis_common::context_command::run_context_command(
            self.config.chat.context_command.as_deref(),
            working_dir.as_deref(),
        )
        .await;
        (
            merge_context_sections(project_context, command_context),
            working_dir,
        )
    }
}

async fn admit_queued_turn(
    queues: &Arc<RwLock<HashMap<String, SessionMessageQueue>>>,
    session_sem: Arc<Semaphore>,
    session_key: &str,
    params: Value,
    queued_replay: bool,
) -> TurnAdmission {
    let mut queues = queues.write().await;
    let queue = queues.entry(session_key.to_string()).or_default();
    let reserved_replay = queued_replay && queue.draining;

    if !reserved_replay && (queue.draining || !queue.messages.is_empty()) {
        queue.messages.push_back(QueuedMessage { params });
        return TurnAdmission::Queued(queue.messages.len());
    }

    match session_sem.try_acquire_owned() {
        Ok(permit) => TurnAdmission::Acquired(permit),
        Err(_) => {
            let message = QueuedMessage { params };
            if reserved_replay {
                queue.messages.push_front(message);
            } else {
                queue.messages.push_back(message);
            }
            TurnAdmission::Queued(queue.messages.len())
        },
    }
}

pub(in crate::service) fn cancel_queued_messages(
    queues: &mut HashMap<String, SessionMessageQueue>,
    session_key: &str,
) -> Vec<QueuedMessage> {
    match queues.get_mut(session_key) {
        Some(queue) if queue.draining => queue.messages.drain(..).collect(),
        Some(_) => queues
            .remove(session_key)
            .map(|queue| queue.messages.into_iter().collect())
            .unwrap_or_default(),
        None => Vec::new(),
    }
}

pub(in crate::service) fn merge_context_sections(
    project_context: Option<String>,
    command_context: Option<String>,
) -> Option<String> {
    match (project_context, command_context) {
        (Some(project), Some(command)) => Some(format!("{project}\n\n{command}")),
        (Some(project), None) => Some(project),
        (None, Some(command)) => Some(command),
        (None, None) => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use {
        super::{
            ActiveAssistantDraft, EventForwarder, LiveChatService, QueuedMessage,
            RunHandleDisposition, SessionMessageQueue, TurnAdmission, admit_queued_turn,
            build_persisted_assistant_message, build_tool_call_assistant_message,
            cancel_queued_messages, commit_successful_turn, merge_context_sections,
        },
        crate::types::AssistantTurnOutput,
        moltis_sessions::PersistedMessage,
        serde_json::json,
        std::{
            collections::{HashMap, HashSet},
            future::Future,
            sync::Arc,
            task::Poll,
        },
        tokio::sync::{RwLock, Semaphore},
    };

    #[test]
    fn active_assistant_draft_omits_cache_usage_fields() {
        let mut draft = ActiveAssistantDraft::new("run-1", "gpt-4.1", "openai", Some(7));
        draft.append_text("hello");
        draft.set_reasoning("thinking");

        let message = draft.to_persisted_message();

        match message {
            PersistedMessage::Assistant {
                cache_read_tokens,
                cache_write_tokens,
                request_cache_read_tokens,
                request_cache_write_tokens,
                seq,
                run_id,
                ..
            } => {
                assert_eq!(cache_read_tokens, None);
                assert_eq!(cache_write_tokens, None);
                assert_eq!(request_cache_read_tokens, None);
                assert_eq!(request_cache_write_tokens, None);
                assert_eq!(seq, Some(7));
                assert_eq!(run_id.as_deref(), Some("run-1"));
            },
            _ => panic!("expected assistant message"),
        }
    }

    #[test]
    fn merge_context_sections_combines_project_and_command_context() {
        let merged = merge_context_sections(Some("project".into()), Some("dynamic".into()))
            .expect("merged context");
        assert_eq!(merged, "project\n\ndynamic");
    }

    #[test]
    fn merge_context_sections_keeps_single_context() {
        assert_eq!(
            merge_context_sections(Some("project".into()), None).as_deref(),
            Some("project")
        );
        assert_eq!(
            merge_context_sections(None, Some("dynamic".into())).as_deref(),
            Some("dynamic")
        );
        assert_eq!(merge_context_sections(None, None), None);
    }

    #[tokio::test]
    async fn replay_reservation_prevents_new_arrival_overtaking_fifo() {
        let queues = Arc::new(RwLock::new(HashMap::from([(
            "s".to_string(),
            SessionMessageQueue {
                messages: [QueuedMessage {
                    params: json!({"text": "second"}),
                }]
                .into_iter()
                .collect(),
                draining: true,
            },
        )])));
        let semaphore = Arc::new(Semaphore::new(1));
        let replay = admit_queued_turn(
            &queues,
            Arc::clone(&semaphore),
            "s",
            json!({"text": "first"}),
            true,
        )
        .await;
        let TurnAdmission::Acquired(permit) = replay else {
            panic!("reserved replay should acquire");
        };

        let arrival =
            admit_queued_turn(&queues, semaphore, "s", json!({"text": "third"}), false).await;
        assert!(matches!(arrival, TurnAdmission::Queued(2)));
        let queue = queues.read().await;
        let texts: Vec<_> = queue["s"]
            .messages
            .iter()
            .filter_map(|message| message.params["text"].as_str())
            .collect();
        assert_eq!(texts, ["second", "third"]);
        drop(permit);
    }

    #[tokio::test]
    async fn abort_mismatch_preserves_active_run_state() {
        let task = tokio::spawn(std::future::pending::<()>());
        let active_runs = Arc::new(RwLock::new(HashMap::from([(
            "run-1".to_string(),
            task.abort_handle(),
        )])));
        let by_session = Arc::new(RwLock::new(HashMap::from([(
            "session-1".to_string(),
            "run-1".to_string(),
        )])));
        let terminal = Arc::new(RwLock::new(HashSet::new()));

        let semaphore = Arc::new(Semaphore::new(1));
        let (resolved, claim) = LiveChatService::claim_run_for_abort(
            &active_runs,
            &by_session,
            &terminal,
            Arc::clone(&semaphore),
            Some("run-2"),
            Some("session-1"),
        )
        .await;
        assert_eq!(resolved, None);
        assert_eq!(claim.disposition, RunHandleDisposition::Unavailable);
        assert!(claim.cleanup.is_none());
        assert!(active_runs.read().await.contains_key("run-1"));
        assert_eq!(
            by_session.read().await.get("session-1").map(String::as_str),
            Some("run-1")
        );

        let (_, claim) = LiveChatService::claim_run_for_abort(
            &active_runs,
            &by_session,
            &terminal,
            semaphore,
            Some("run-1"),
            Some("session-1"),
        )
        .await;
        assert_eq!(claim.disposition, RunHandleDisposition::Aborted);
        drop(claim.cleanup.unwrap().acquire().await.unwrap());
        assert!(active_runs.read().await.is_empty());
        assert!(by_session.read().await.is_empty());
        assert!(task.await.is_err());
    }

    #[tokio::test]
    async fn panicked_terminal_run_handle_is_removed_as_stale() {
        let task = tokio::spawn(async { panic!("simulated run panic") });
        let handle = task.abort_handle();
        assert!(task.await.is_err());
        let active_runs = Arc::new(RwLock::new(HashMap::from([("run-1".to_string(), handle)])));
        let by_session = Arc::new(RwLock::new(HashMap::from([(
            "session-1".to_string(),
            "run-1".to_string(),
        )])));
        let terminal = Arc::new(RwLock::new(HashSet::new()));

        let (resolved, claim) = LiveChatService::claim_run_for_abort(
            &active_runs,
            &by_session,
            &terminal,
            Arc::new(Semaphore::new(1)),
            Some("run-1"),
            Some("session-1"),
        )
        .await;

        assert_eq!(resolved.as_deref(), Some("run-1"));
        assert_eq!(claim.disposition, RunHandleDisposition::Stale);
        drop(claim.cleanup.unwrap().acquire().await.unwrap());
        assert!(active_runs.read().await.is_empty());
        assert!(by_session.read().await.is_empty());
    }

    #[tokio::test]
    async fn shared_success_commit_transitions_then_persists_before_final() {
        let terminal = Arc::new(RwLock::new(HashSet::new()));
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let persist_events = Arc::clone(&events);
        let final_events = Arc::clone(&events);
        let terminal_at_persistence = Arc::clone(&terminal);
        let terminal_at_broadcast = Arc::clone(&terminal);

        commit_successful_turn(
            &terminal,
            "run-1",
            async move {
                assert!(terminal_at_persistence.read().await.contains("run-1"));
                persist_events
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push("persist");
            },
            async move {
                assert!(terminal_at_broadcast.read().await.contains("run-1"));
                final_events
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push("final");
            },
        )
        .await;

        assert_eq!(*events.lock().unwrap_or_else(|error| error.into_inner()), [
            "persist", "final"
        ]);
    }

    #[tokio::test]
    async fn abort_during_shared_delivery_prevents_persist_and_final() {
        let semaphore = Arc::new(Semaphore::new(1));
        let permit = Arc::clone(&semaphore).acquire_owned().await.unwrap();
        let terminal = Arc::new(RwLock::new(HashSet::new()));
        let active_runs = Arc::new(RwLock::new(HashMap::new()));
        let by_session = Arc::new(RwLock::new(HashMap::new()));
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let task_events = Arc::clone(&events);
        let task_terminal = Arc::clone(&terminal);
        let (delivery_started, started) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _permit = permit;
            let _ = delivery_started.send(());
            std::future::pending::<()>().await;
            commit_successful_turn(
                &task_terminal,
                "run-1",
                async {
                    task_events
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .push("persist");
                },
                async {
                    task_events
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .push("final");
                },
            )
            .await;
        });
        LiveChatService::register_run_handle(
            &active_runs,
            &by_session,
            "run-1",
            "session-1",
            task.abort_handle(),
        )
        .await;
        assert!(started.await.is_ok());

        let (_, claim) = LiveChatService::claim_run_for_abort(
            &active_runs,
            &by_session,
            &terminal,
            semaphore,
            Some("run-1"),
            Some("session-1"),
        )
        .await;
        assert_eq!(claim.disposition, RunHandleDisposition::Aborted);
        drop(claim.cleanup.unwrap().acquire().await.unwrap());
        assert!(task.await.is_err());
        assert!(
            events
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .is_empty()
        );
        assert!(terminal.read().await.is_empty());
    }

    #[tokio::test]
    async fn concurrent_forwarder_waiters_share_run_scoped_completion() {
        let (release, released) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = released.await;
            "reasoning".to_string()
        });
        let completion = EventForwarder::new(task, "session-1".to_string());
        let forwarders = Arc::new(RwLock::new(HashMap::from([(
            "run-1".to_string(),
            completion,
        )])));
        let first = LiveChatService::wait_for_event_forwarder(&forwarders, "run-1");
        let second = LiveChatService::wait_for_event_forwarder(&forwarders, "run-1");
        tokio::pin!(first, second);
        std::future::poll_fn(|cx| {
            assert!(matches!(first.as_mut().poll(cx), Poll::Pending));
            assert!(matches!(second.as_mut().poll(cx), Poll::Pending));
            Poll::Ready(())
        })
        .await;

        release.send(()).unwrap();
        let (first_result, second_result) = tokio::join!(&mut first, &mut second);
        assert_eq!(first_result, "reasoning");
        assert_eq!(second_result, "reasoning");
        assert!(forwarders.read().await.is_empty());
    }

    #[tokio::test]
    async fn cancelling_forwarder_aborts_pending_delivery_work() {
        struct DropNotice(Option<tokio::sync::oneshot::Sender<()>>);
        impl Drop for DropNotice {
            fn drop(&mut self) {
                if let Some(sender) = self.0.take() {
                    let _ = sender.send(());
                }
            }
        }

        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _notice = DropNotice(Some(dropped_tx));
            let _ = started_tx.send(());
            std::future::pending::<String>().await
        });
        let forwarders = Arc::new(RwLock::new(HashMap::from([(
            "run-1".to_string(),
            EventForwarder::new(task, "session-1".to_string()),
        )])));
        assert!(started_rx.await.is_ok());

        LiveChatService::cancel_event_forwarder(&forwarders, "run-1").await;

        assert!(forwarders.read().await.is_empty());
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), dropped_rx)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn run_cannot_finish_before_abort_handle_registration() {
        let active_runs = Arc::new(RwLock::new(HashMap::new()));
        let by_session = Arc::new(RwLock::new(HashMap::new()));
        let active_runs_for_task = Arc::clone(&active_runs);
        let by_session_for_task = Arc::clone(&by_session);
        let (start_run, run_registered) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            if run_registered.await.is_err() {
                return;
            }
            active_runs_for_task.write().await.remove("run-1");
            by_session_for_task.write().await.remove("session-1");
        });
        LiveChatService::register_run_handle(
            &active_runs,
            &by_session,
            "run-1",
            "session-1",
            task.abort_handle(),
        )
        .await;
        assert!(active_runs.read().await.contains_key("run-1"));
        assert_eq!(
            by_session.read().await.get("session-1").map(String::as_str),
            Some("run-1")
        );

        assert!(start_run.send(()).is_ok());
        assert!(task.await.is_ok());
        assert!(active_runs.read().await.is_empty());
        assert!(by_session.read().await.is_empty());
    }

    #[tokio::test]
    async fn registered_abort_cleanup_waiter_blocks_try_acquire_race() {
        let semaphore = Arc::new(Semaphore::new(1));
        let active_permit = Arc::clone(&semaphore).acquire_owned().await.unwrap();
        let cleanup_waiter = Arc::clone(&semaphore).acquire_owned();
        tokio::pin!(cleanup_waiter);
        std::future::poll_fn(|cx| {
            assert!(matches!(cleanup_waiter.as_mut().poll(cx), Poll::Pending));
            Poll::Ready(())
        })
        .await;

        drop(active_permit);
        assert!(Arc::clone(&semaphore).try_acquire_owned().is_err());
        let cleanup_permit = cleanup_waiter.await.unwrap();
        drop(cleanup_permit);
        assert!(semaphore.try_acquire_owned().is_ok());
    }

    #[test]
    fn cancellation_keeps_in_progress_replay_reserved() {
        let mut queues = HashMap::from([("s".to_string(), SessionMessageQueue {
            messages: [QueuedMessage {
                params: json!({"text": "cancel"}),
            }]
            .into_iter()
            .collect(),
            draining: true,
        })]);
        let removed = cancel_queued_messages(&mut queues, "s");
        assert_eq!(removed.len(), 1);
        assert!(queues["s"].draining);
        assert!(queues["s"].messages.is_empty());
    }

    #[test]
    fn tool_call_assistant_message_omits_cache_usage_fields() {
        let message = build_tool_call_assistant_message(
            "tool-1",
            "exec",
            Some(serde_json::json!({"cmd": "ls"})),
            None,
            Some(3),
            Some("run-1"),
        );

        match message {
            PersistedMessage::Assistant {
                cache_read_tokens,
                cache_write_tokens,
                request_cache_read_tokens,
                request_cache_write_tokens,
                tool_calls,
                ..
            } => {
                assert_eq!(cache_read_tokens, None);
                assert_eq!(cache_write_tokens, None);
                assert_eq!(request_cache_read_tokens, None);
                assert_eq!(request_cache_write_tokens, None);
                assert_eq!(tool_calls.as_ref().map(Vec::len), Some(1));
            },
            _ => panic!("expected assistant message"),
        }
    }

    #[test]
    fn persisted_assistant_message_includes_cache_usage_fields() {
        let message = build_persisted_assistant_message(
            AssistantTurnOutput {
                text: "hello".to_string(),
                input_tokens: 1200,
                output_tokens: 80,
                cache_read_tokens: 1050,
                cache_write_tokens: 4,
                duration_ms: 250,
                request_input_tokens: 900,
                request_output_tokens: 60,
                request_cache_read_tokens: 850,
                request_cache_write_tokens: 2,
                audio_path: None,
                reasoning: Some("thinking".to_string()),
                llm_api_response: None,
                final_broadcast: None,
            },
            Some("gpt-4.1".to_string()),
            Some("openai".to_string()),
            Some(7),
            Some("run-1".to_string()),
        );

        match message {
            PersistedMessage::Assistant {
                cache_read_tokens,
                cache_write_tokens,
                request_cache_read_tokens,
                request_cache_write_tokens,
                ..
            } => {
                assert_eq!(cache_read_tokens, Some(1050));
                assert_eq!(cache_write_tokens, Some(4));
                assert_eq!(request_cache_read_tokens, Some(850));
                assert_eq!(request_cache_write_tokens, Some(2));
            },
            _ => panic!("expected assistant message"),
        }
    }
}
