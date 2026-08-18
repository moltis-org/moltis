//! Agent loop support: model flagging, shell commands, channel streaming, and compaction.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use {
    moltis_config::schema::ToolMode,
    serde_json::Value,
    tokio::sync::{Mutex, RwLock, mpsc},
    tracing::{debug, info, warn},
};

use {
    moltis_agents::{runner::RunnerEvent, tool_registry::ToolRegistry},
    moltis_sessions::{PersistedMessage, store::SessionStore},
};

use crate::{
    channels::{deliver_channel_replies, send_tool_status_to_channels},
    chat_error::parse_chat_error,
    compaction_run, error,
    models::DisabledModelsStore,
    runtime::ChatRuntime,
    service::{build_tool_call_assistant_message, commit_terminal_run, persist_tool_history_pair},
    types::*,
};

pub(crate) async fn mark_unsupported_model(
    state: &Arc<dyn ChatRuntime>,
    model_store: &Arc<RwLock<DisabledModelsStore>>,
    model_id: &str,
    provider_name: &str,
    error_obj: &Value,
) {
    if error_obj.get("type").and_then(|v| v.as_str()) != Some("unsupported_model") {
        return;
    }

    let detail = error_obj
        .get("detail")
        .and_then(|v| v.as_str())
        .unwrap_or("Model is not supported for this account/provider");
    let provider = error_obj
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or(provider_name);

    let mut store = model_store.write().await;
    if store.mark_unsupported(model_id, detail, Some(provider)) {
        let unsupported = store.unsupported_info(model_id).cloned();
        if let Err(err) = store.save() {
            warn!(
                model = model_id,
                provider = provider,
                error = %err,
                "failed to persist unsupported model flag"
            );
        } else {
            info!(
                model = model_id,
                provider = provider,
                "flagged model as unsupported"
            );
        }
        drop(store);
        broadcast(
            state,
            "models.updated",
            serde_json::json!({
                "modelId": model_id,
                "unsupported": true,
                "unsupportedReason": unsupported.as_ref().map(|u| u.detail.as_str()).unwrap_or(detail),
                "unsupportedProvider": unsupported
                    .as_ref()
                    .and_then(|u| u.provider.as_deref())
                    .unwrap_or(provider),
                "unsupportedUpdatedAt": unsupported.map(|u| u.updated_at_ms).unwrap_or_else(now_ms),
            }),
            BroadcastOpts::default(),
        )
        .await;
    }
}

pub(crate) async fn clear_unsupported_model(
    state: &Arc<dyn ChatRuntime>,
    model_store: &Arc<RwLock<DisabledModelsStore>>,
    model_id: &str,
) {
    let mut store = model_store.write().await;
    if store.clear_unsupported(model_id) {
        if let Err(err) = store.save() {
            warn!(
                model = model_id,
                error = %err,
                "failed to persist unsupported model clear"
            );
        } else {
            info!(model = model_id, "cleared unsupported model flag");
        }
        drop(store);
        broadcast(
            state,
            "models.updated",
            serde_json::json!({
                "modelId": model_id,
                "unsupported": false,
            }),
            BroadcastOpts::default(),
        )
        .await;
    }
}

pub(crate) fn ordered_runner_event_callback() -> (
    Box<dyn Fn(RunnerEvent) + Send + Sync>,
    mpsc::UnboundedReceiver<RunnerEvent>,
) {
    let (tx, rx) = mpsc::unbounded_channel::<RunnerEvent>();
    let callback: Box<dyn Fn(RunnerEvent) + Send + Sync> = Box::new(move |event| {
        if tx.send(event).is_err() {
            debug!("runner event dropped because event processor is closed");
        }
    });
    (callback, rx)
}

const CHANNEL_STREAM_BUFFER_SIZE: usize = 64;
const FINAL_CHANNEL_STREAM_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ChannelReplyTargetKey {
    channel_type: moltis_channels::ChannelType,
    account_id: String,
    chat_id: String,
    message_id: Option<String>,
    thread_id: Option<String>,
}

impl From<&moltis_channels::ChannelReplyTarget> for ChannelReplyTargetKey {
    fn from(target: &moltis_channels::ChannelReplyTarget) -> Self {
        Self {
            channel_type: target.channel_type,
            account_id: target.account_id.clone(),
            chat_id: target.chat_id.clone(),
            message_id: target.message_id.clone(),
            thread_id: target.thread_id.clone(),
        }
    }
}

struct ChannelStreamWorker {
    sender: moltis_channels::StreamSender,
    receives_progress_deltas: bool,
    receives_task_updates: bool,
}

struct AbortOnDropTask(Option<tokio::task::JoinHandle<()>>);

impl AbortOnDropTask {
    fn new(task: tokio::task::JoinHandle<()>) -> Self {
        Self(Some(task))
    }

    fn abort(&self) {
        if let Some(task) = &self.0 {
            task.abort();
        }
    }

    async fn join(mut self) -> Result<(), tokio::task::JoinError> {
        let Some(task) = self.0.as_mut() else {
            return Ok(());
        };
        let result = task.await;
        self.0.take();
        result
    }
}

impl Drop for AbortOnDropTask {
    fn drop(&mut self) {
        self.abort();
    }
}

/// Fan out model deltas to channel stream workers (Telegram/Discord edit-in-place).
///
/// Workers are started eagerly so channel typing indicators remain active
/// during long-running tool execution before the first text delta arrives.
/// Stream-dedup only applies after at least one delta has been sent.
pub(crate) struct ChannelStreamDispatcher {
    outbound: Arc<dyn moltis_channels::plugin::ChannelStreamOutbound>,
    targets: Vec<moltis_channels::ChannelReplyTarget>,
    workers: Vec<ChannelStreamWorker>,
    tasks: Vec<AbortOnDropTask>,
    completed: Arc<Mutex<HashSet<ChannelReplyTargetKey>>>,
    feedback: Option<Arc<moltis_channels::FeedbackService>>,
    session_key: String,
    trace_correlation_key: String,
    started: bool,
    sent_final_delta: Arc<AtomicBool>,
    active_tasks: HashMap<String, String>,
    task_ids: HashMap<String, String>,
    next_task_id: usize,
}

impl ChannelStreamDispatcher {
    pub(crate) async fn for_session(
        state: &Arc<dyn ChatRuntime>,
        session_key: &str,
        trace_correlation_key: &str,
    ) -> Option<Self> {
        let outbound = state.channel_stream_outbound()?;
        let targets: Vec<moltis_channels::ChannelReplyTarget> = state
            .peek_channel_replies(session_key)
            .await
            .into_iter()
            .collect();
        if targets.is_empty() {
            return None;
        }
        let mut dispatcher = Self {
            outbound,
            targets,
            workers: Vec::new(),
            tasks: Vec::new(),
            completed: Arc::new(Mutex::new(HashSet::new())),
            feedback: state.feedback(),
            session_key: session_key.to_string(),
            trace_correlation_key: trace_correlation_key.to_string(),
            started: false,
            sent_final_delta: Arc::new(AtomicBool::new(false)),
            active_tasks: HashMap::new(),
            task_ids: HashMap::new(),
            next_task_id: 0,
        };
        dispatcher.ensure_started().await;
        Some(dispatcher)
    }

    async fn ensure_started(&mut self) {
        if self.started {
            return;
        }
        self.started = true;

        for target in self.targets.iter().cloned() {
            if !self.outbound.is_stream_enabled(&target.account_id).await {
                debug!(
                    account_id = target.account_id.as_str(),
                    chat_id = target.chat_id.as_str(),
                    "channel streaming disabled for target account"
                );
                continue;
            }

            let key = ChannelReplyTargetKey::from(&target);
            let streams_final_replies = self
                .outbound
                .streams_final_replies(&target.account_id)
                .await;
            let receives_progress_deltas = self
                .outbound
                .receives_progress_deltas(&target.account_id)
                .await;
            let receives_task_updates = self
                .outbound
                .receives_task_updates(&target.account_id)
                .await;
            let claims_stream_delivery = self
                .outbound
                .claims_stream_delivery(&target.account_id, target.message_id.as_deref())
                .await;
            let (tx, rx) = mpsc::channel(CHANNEL_STREAM_BUFFER_SIZE);
            let outbound = Arc::clone(&self.outbound);
            let completed = Arc::clone(&self.completed);
            let feedback = self.feedback.clone();
            let session_key = self.session_key.clone();
            let trace_correlation_key = self.trace_correlation_key.clone();
            let sent_final_delta = Arc::clone(&self.sent_final_delta);
            let account_id = target.account_id.clone();
            let to = target.outbound_to().into_owned();
            let reply_to = target.message_id.clone();
            let key_for_insert = key.clone();
            let account_for_log = account_id.clone();
            let chat_for_log = target.chat_id.clone();
            let thread_for_log = target.thread_id.clone();

            self.workers.push(ChannelStreamWorker {
                sender: tx,
                receives_progress_deltas,
                receives_task_updates,
            });
            self.tasks
                .push(AbortOnDropTask::new(tokio::spawn(async move {
                    match outbound
                        .send_stream_reporting_ids(&account_id, &to, reply_to.as_deref(), rx)
                        .await
                    {
                        Ok(message_ids) => {
                            if streams_final_replies
                                && (sent_final_delta.load(Ordering::Acquire)
                                    || claims_stream_delivery)
                            {
                                completed.lock().await.insert(key_for_insert);
                                crate::channel_feedback::record_reply_trace(
                                    feedback.as_deref(),
                                    &target,
                                    &message_ids,
                                    &session_key,
                                    &trace_correlation_key,
                                )
                                .await;
                            }
                        },
                        Err(e) => {
                            warn!(
                                account_id = account_for_log,
                                chat_id = chat_for_log,
                                thread_id = thread_for_log.as_deref().unwrap_or("-"),
                                "channel stream outbound failed: {e}"
                            );
                        },
                    }
                })));
        }
    }

    pub(crate) async fn send_delta(&mut self, delta: &str) {
        if delta.is_empty() {
            return;
        }
        self.sent_final_delta.store(true, Ordering::Release);
        self.ensure_started().await;
        self.send_to_workers(
            moltis_channels::StreamEvent::Delta(delta.to_string()),
            "delta",
        )
        .await;
    }

    pub(crate) async fn send_progress_delta(&mut self, delta: &str) {
        if delta.is_empty() {
            return;
        }
        self.ensure_started().await;
        let event = moltis_channels::StreamEvent::ProgressDelta(delta.to_string());
        for worker in &self.workers {
            if worker.receives_progress_deltas && worker.sender.send(event.clone()).await.is_err() {
                debug!("channel stream progress delta dropped: worker closed");
            }
        }
    }

    pub(crate) async fn send_task_update(
        &mut self,
        mut update: moltis_channels::plugin::ChannelTaskUpdate,
    ) {
        use moltis_channels::plugin::ChannelTaskStatus;

        self.ensure_started().await;
        let source_id = update.id.clone();
        update.id = self.opaque_task_id(&source_id);
        match update.status {
            ChannelTaskStatus::InProgress => {
                self.active_tasks
                    .insert(source_id.clone(), update.title.clone());
            },
            ChannelTaskStatus::Complete | ChannelTaskStatus::Error => {
                self.active_tasks.remove(&source_id);
            },
        }
        let event = moltis_channels::StreamEvent::TaskUpdate(update);
        for worker in &self.workers {
            if worker.receives_task_updates && worker.sender.send(event.clone()).await.is_err() {
                debug!("channel stream task update dropped: worker closed");
            }
        }
    }

    fn opaque_task_id(&mut self, source_id: &str) -> String {
        if let Some(id) = self.task_ids.get(source_id) {
            return id.clone();
        }
        self.next_task_id += 1;
        let id = format!("task-{}", self.next_task_id);
        self.task_ids.insert(source_id.to_string(), id.clone());
        id
    }

    async fn send_to_workers(&mut self, event: moltis_channels::StreamEvent, label: &str) {
        for worker in &self.workers {
            if worker.sender.send(event.clone()).await.is_err() {
                debug!("channel stream {label} dropped: worker closed");
            }
        }
    }

    pub(crate) async fn finish(&mut self) {
        if tokio::time::timeout(FINAL_CHANNEL_STREAM_TIMEOUT, self.finish_inner())
            .await
            .is_err()
        {
            warn!("timed out finishing channel stream workers");
            self.abort_workers();
            self.workers.clear();
            self.join_workers().await;
        }
    }

    async fn finish_inner(&mut self) {
        let unfinished = std::mem::take(&mut self.active_tasks);
        for (id, title) in unfinished {
            self.send_task_update(moltis_channels::plugin::ChannelTaskUpdate {
                id,
                title,
                status: moltis_channels::plugin::ChannelTaskStatus::Error,
            })
            .await;
        }
        self.send_terminal(moltis_channels::StreamEvent::Done).await;
        self.join_workers().await;
    }

    async fn send_terminal(&mut self, event: moltis_channels::StreamEvent) {
        if self.workers.is_empty() {
            return;
        }
        let workers = std::mem::take(&mut self.workers);
        for worker in &workers {
            if worker.sender.send(event.clone()).await.is_err() {
                debug!("channel stream terminal event dropped: worker closed");
            }
        }
    }

    async fn join_workers(&mut self) {
        let tasks = std::mem::take(&mut self.tasks);
        for task in tasks {
            if let Err(e) = task.join().await {
                warn!(error = %e, "channel stream worker task join failed");
            }
        }
    }

    fn abort_workers(&self) {
        for task in &self.tasks {
            task.abort();
        }
    }

    pub(crate) async fn completed_target_keys(&self) -> HashSet<ChannelReplyTargetKey> {
        self.completed.lock().await.clone()
    }
}

impl Drop for ChannelStreamDispatcher {
    fn drop(&mut self) {
        // Abort receivers before dropping senders. Some channel implementations
        // treat sender closure like Done and would otherwise publish a late final.
        self.abort_workers();
    }
}

pub(crate) async fn commit_terminal_and_finish_channel_stream(
    terminal_runs: &Arc<RwLock<HashSet<String>>>,
    run_id: &str,
    dispatcher: Option<&mut ChannelStreamDispatcher>,
) -> HashSet<ChannelReplyTargetKey> {
    commit_terminal_run(terminal_runs, run_id).await;
    let Some(dispatcher) = dispatcher else {
        return HashSet::new();
    };
    dispatcher.finish().await;
    dispatcher.completed_target_keys().await
}

pub(crate) async fn run_explicit_shell_command(
    state: &Arc<dyn ChatRuntime>,
    run_id: &str,
    terminal_runs: &Arc<RwLock<HashSet<String>>>,
    tool_registry: &Arc<RwLock<ToolRegistry>>,
    session_store: Option<&Arc<SessionStore>>,
    session_key: &str,
    command: &str,
    user_message_index: usize,
    accept_language: Option<String>,
    conn_id: Option<String>,
    client_seq: Option<u64>,
    working_dir: Option<String>,
) -> AssistantTurnOutput {
    let started = Instant::now();
    let tool_call_id = format!("sh_{}", uuid::Uuid::new_v4().simple());
    let tool_args = serde_json::json!({ "command": command });

    send_tool_status_to_channels(state, run_id, session_key, "exec", &tool_args).await;

    broadcast(
        state,
        "chat",
        serde_json::json!({
            "runId": run_id,
            "sessionKey": session_key,
            "state": "tool_call_start",
            "toolCallId": tool_call_id,
            "toolName": "exec",
            "arguments": tool_args,
            "seq": client_seq,
        }),
        BroadcastOpts::default(),
    )
    .await;

    let mut exec_params = serde_json::json!({
        "command": command,
        "_session_key": session_key,
    });
    if let Some(lang) = accept_language.as_deref() {
        exec_params["_accept_language"] = serde_json::json!(lang);
    }
    if let Some(cid) = conn_id.as_deref() {
        exec_params["_conn_id"] = serde_json::json!(cid);
    }
    if let Some(directory) = working_dir {
        exec_params["_working_dir"] = serde_json::json!(directory);
    }

    let exec_tool = {
        let registry = tool_registry.read().await;
        registry.get("exec")
    };

    let exec_result = match exec_tool {
        Some(tool) => tool.execute(exec_params).await,
        None => Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "exec tool is not registered",
        )
        .into()),
    };

    let has_channel_targets = !state.peek_channel_replies(session_key).await.is_empty();
    let mut final_text = String::new();

    match exec_result {
        Ok(result) => {
            let capped = capped_tool_result_payload(&result, 10_000);
            let assistant_tool_call_msg = build_tool_call_assistant_message(
                tool_call_id.clone(),
                "exec",
                Some(tool_args.clone()),
                None,
                client_seq,
                Some(run_id),
            );
            // Both halves of the pair must carry the run id: run-scoped history
            // filtering keeps or drops a run as a unit, and a result without one
            // would be dropped on its own, orphaning the tool call above.
            let tool_result_msg = PersistedMessage::tool_result_with_run_id(
                tool_call_id.clone(),
                "exec",
                Some(serde_json::json!({ "command": command })),
                true,
                Some(capped.clone()),
                None,
                run_id,
            );
            if let Some(session_store) = session_store {
                persist_tool_history_pair(
                    session_store,
                    session_key,
                    assistant_tool_call_msg,
                    tool_result_msg,
                    "failed to persist direct /sh assistant tool call",
                    "failed to persist direct /sh tool result",
                )
                .await;
            }

            broadcast(
                state,
                "chat",
                serde_json::json!({
                    "runId": run_id,
                    "sessionKey": session_key,
                    "state": "tool_call_end",
                    "toolCallId": tool_call_id,
                    "toolName": "exec",
                    "success": true,
                    "result": capped,
                    "seq": client_seq,
                }),
                BroadcastOpts::default(),
            )
            .await;

            if has_channel_targets {
                final_text = shell_reply_text_from_exec_result(&result);
                if final_text.is_empty() {
                    final_text = "Command completed.".to_string();
                }
            }
        },
        Err(err) => {
            let error_text = err.to_string();
            let parsed_error = parse_chat_error(&error_text, None);
            let assistant_tool_call_msg = build_tool_call_assistant_message(
                tool_call_id.clone(),
                "exec",
                Some(tool_args.clone()),
                None,
                client_seq,
                Some(run_id),
            );
            let tool_result_msg = PersistedMessage::tool_result_with_run_id(
                tool_call_id.clone(),
                "exec",
                Some(serde_json::json!({ "command": command })),
                false,
                None,
                Some(error_text.clone()),
                run_id,
            );
            if let Some(session_store) = session_store {
                persist_tool_history_pair(
                    session_store,
                    session_key,
                    assistant_tool_call_msg,
                    tool_result_msg,
                    "failed to persist direct /sh assistant tool call",
                    "failed to persist direct /sh tool error",
                )
                .await;
            }

            broadcast(
                state,
                "chat",
                serde_json::json!({
                    "runId": run_id,
                    "sessionKey": session_key,
                    "state": "tool_call_end",
                    "toolCallId": tool_call_id,
                    "toolName": "exec",
                    "success": false,
                    "error": parsed_error,
                    "seq": client_seq,
                }),
                BroadcastOpts::default(),
            )
            .await;

            if has_channel_targets {
                final_text = error_text;
            }
        },
    }

    // A channel can accept the reply immediately, so claim terminal ownership
    // before final delivery. This prevents a concurrent abort from emitting a
    // contradictory aborted terminal after the user already received output.
    commit_terminal_run(terminal_runs, run_id).await;
    if !final_text.trim().is_empty() {
        let streamed_target_keys = HashSet::new();
        deliver_channel_replies(
            state,
            run_id,
            session_key,
            &final_text,
            ReplyMedium::Text,
            &streamed_target_keys,
        )
        .await;
    }

    let final_payload = build_chat_final_broadcast(
        run_id,
        session_key,
        final_text.clone(),
        String::new(),
        String::new(),
        UsageSnapshot::new(
            moltis_agents::model::Usage::default(),
            Some(moltis_agents::model::Usage::default()),
        ),
        started.elapsed().as_millis() as u64,
        user_message_index + 3, // +1 tool call assistant, +1 tool result, +1 final assistant
        ReplyMedium::Text,
        Some(1),
        Some(1),
        None,
        None,
        None,
        client_seq,
    );
    #[allow(clippy::unwrap_used)] // serializing known-valid struct
    let payload = serde_json::to_value(&final_payload).unwrap();

    let mut output = build_assistant_turn_output(
        final_text,
        UsageSnapshot::new(
            moltis_agents::model::Usage::default(),
            Some(moltis_agents::model::Usage::default()),
        ),
        started.elapsed().as_millis() as u64,
        None,
        None,
        None,
    );
    output.final_broadcast = Some(payload);
    output
}

/// Resolve the effective tool mode for a provider.
///
/// Combines the provider's `tool_mode()` override with its `supports_tools()`
/// capability to determine how tools should be dispatched:
/// - `Native` — provider handles tool schemas via API (OpenAI function calling, etc.)
/// - `Text` — tools are described in the prompt; the runner parses tool calls from text
/// - `Off` — no tools at all
pub(crate) fn effective_tool_mode(provider: &dyn moltis_agents::model::LlmProvider) -> ToolMode {
    match provider.tool_mode() {
        Some(ToolMode::Native) => ToolMode::Native,
        Some(ToolMode::Text) => ToolMode::Text,
        Some(ToolMode::Off) => ToolMode::Off,
        Some(ToolMode::Auto) | None => {
            if provider.supports_tools() {
                ToolMode::Native
            } else {
                ToolMode::Text
            }
        },
    }
}

pub(crate) async fn compact_session(
    store: &Arc<SessionStore>,
    session_key: &str,
    config: &moltis_config::CompactionConfig,
    provider: Option<&dyn moltis_agents::model::LlmProvider>,
    max_tool_result_bytes: usize,
) -> error::Result<compaction_run::CompactionOutcome> {
    let history = store
        .read(session_key)
        .await
        .map_err(|source| error::Error::external("failed to read session history", source))?;

    let mut outcome =
        compaction_run::run_compaction(&history, config, provider, max_tool_result_bytes)
            .await
            .map_err(|e| error::Error::message(e.to_string()))?;

    // Enforce summary budget discipline on the compacted history.
    outcome.history = compress_summary_in_history(outcome.history);

    store
        .replace_history(session_key, outcome.history.clone())
        .await
        .map_err(|source| error::Error::external("failed to replace compacted history", source))?;

    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use async_trait::async_trait;

    use super::*;

    struct RecordingStreamOutbound {
        streams_final_replies: bool,
        receives_progress_deltas: bool,
        receives_task_updates: bool,
        claims_stream_delivery: bool,
        events: Arc<Mutex<Vec<moltis_channels::StreamEvent>>>,
        reporting_calls: std::sync::atomic::AtomicUsize,
    }

    struct DropNotice(Option<tokio::sync::oneshot::Sender<()>>);

    impl Drop for DropNotice {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    struct ClosureFinalizingOutbound {
        started: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        dropped: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        finalized_on_close: AtomicBool,
    }

    #[async_trait]
    impl moltis_channels::ChannelStreamOutbound for ClosureFinalizingOutbound {
        async fn send_stream(
            &self,
            _account_id: &str,
            _to: &str,
            _reply_to: Option<&str>,
            mut stream: moltis_channels::StreamReceiver,
        ) -> moltis_channels::Result<()> {
            let dropped = self.dropped.lock().await.take();
            let _notice = DropNotice(dropped);
            if let Some(started) = self.started.lock().await.take() {
                let _ = started.send(());
            }
            while let Some(event) = stream.recv().await {
                if matches!(
                    event,
                    moltis_channels::StreamEvent::Done | moltis_channels::StreamEvent::Error(_)
                ) {
                    return Ok(());
                }
            }
            self.finalized_on_close.store(true, Ordering::SeqCst);
            Ok(())
        }

        async fn is_stream_enabled(&self, _account_id: &str) -> bool {
            true
        }
    }

    struct TerminalCheckingOutbound {
        terminal_runs: Arc<RwLock<HashSet<String>>>,
        final_observed_after_terminal: AtomicBool,
    }

    #[async_trait]
    impl moltis_channels::ChannelStreamOutbound for TerminalCheckingOutbound {
        async fn send_stream(
            &self,
            _account_id: &str,
            _to: &str,
            _reply_to: Option<&str>,
            mut stream: moltis_channels::StreamReceiver,
        ) -> moltis_channels::Result<()> {
            while let Some(event) = stream.recv().await {
                if matches!(event, moltis_channels::StreamEvent::Done) {
                    self.final_observed_after_terminal.store(
                        self.terminal_runs.read().await.contains("run-1"),
                        Ordering::SeqCst,
                    );
                    break;
                }
            }
            Ok(())
        }

        async fn is_stream_enabled(&self, _account_id: &str) -> bool {
            true
        }

        async fn streams_final_replies(&self, _account_id: &str) -> bool {
            true
        }
    }

    #[async_trait]
    impl moltis_channels::ChannelStreamOutbound for RecordingStreamOutbound {
        async fn send_stream(
            &self,
            _account_id: &str,
            _to: &str,
            _reply_to: Option<&str>,
            mut stream: moltis_channels::StreamReceiver,
        ) -> moltis_channels::Result<()> {
            while let Some(event) = stream.recv().await {
                self.events.lock().await.push(event.clone());
                if matches!(
                    event,
                    moltis_channels::StreamEvent::Done | moltis_channels::StreamEvent::Error(_)
                ) {
                    break;
                }
            }
            Ok(())
        }

        async fn send_stream_reporting_ids(
            &self,
            account_id: &str,
            to: &str,
            reply_to: Option<&str>,
            stream: moltis_channels::StreamReceiver,
        ) -> moltis_channels::Result<Vec<String>> {
            self.reporting_calls.fetch_add(1, Ordering::SeqCst);
            self.send_stream(account_id, to, reply_to, stream).await?;
            Ok(vec!["stream-message".into()])
        }

        async fn is_stream_enabled(&self, _account_id: &str) -> bool {
            true
        }

        async fn streams_final_replies(&self, _account_id: &str) -> bool {
            self.streams_final_replies
        }

        async fn receives_progress_deltas(&self, _account_id: &str) -> bool {
            self.receives_progress_deltas
        }

        async fn receives_task_updates(&self, _account_id: &str) -> bool {
            self.receives_task_updates
        }

        async fn claims_stream_delivery(&self, _account_id: &str, _reply_to: Option<&str>) -> bool {
            self.claims_stream_delivery
        }
    }

    fn channel_target() -> moltis_channels::ChannelReplyTarget {
        moltis_channels::ChannelReplyTarget {
            ack_message_id: None,
            channel_type: moltis_channels::ChannelType::Telegram,
            account_id: "bot".into(),
            chat_id: "chat".into(),
            message_id: Some("msg".into()),
            thread_id: None,
        }
    }

    fn dispatcher_with(
        outbound: Arc<dyn moltis_channels::ChannelStreamOutbound>,
    ) -> ChannelStreamDispatcher {
        ChannelStreamDispatcher {
            outbound,
            targets: vec![channel_target()],
            workers: Vec::new(),
            tasks: Vec::new(),
            completed: Arc::new(Mutex::new(HashSet::new())),
            feedback: None,
            session_key: "session".into(),
            trace_correlation_key: "run".into(),
            started: false,
            sent_final_delta: Arc::new(AtomicBool::new(false)),
            active_tasks: HashMap::new(),
            task_ids: HashMap::new(),
            next_task_id: 0,
        }
    }

    fn event_kinds(events: &[moltis_channels::StreamEvent]) -> Vec<&'static str> {
        events
            .iter()
            .map(|event| match event {
                moltis_channels::StreamEvent::Delta(_) => "delta",
                moltis_channels::StreamEvent::ProgressDelta(_) => "progress",
                moltis_channels::StreamEvent::TaskUpdate(_) => "task",
                moltis_channels::StreamEvent::Done => "done",
                moltis_channels::StreamEvent::Error(_) => "error",
            })
            .collect()
    }

    #[tokio::test]
    async fn streams_without_final_delivery_do_not_complete_targets() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let outbound = Arc::new(RecordingStreamOutbound {
            streams_final_replies: false,
            claims_stream_delivery: false,
            receives_progress_deltas: true,
            receives_task_updates: false,
            events: Arc::clone(&events),
            reporting_calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let mut dispatcher = dispatcher_with(outbound.clone());

        dispatcher.send_progress_delta("progress").await;
        dispatcher.send_delta("final").await;
        dispatcher.finish().await;

        let events = events.lock().await;
        assert_eq!(event_kinds(&events), vec!["progress", "delta", "done"]);
        assert!(dispatcher.completed_target_keys().await.is_empty());
    }

    #[tokio::test]
    async fn final_stream_workers_receive_progress_and_final_deltas_and_complete_targets() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let outbound = Arc::new(RecordingStreamOutbound {
            streams_final_replies: true,
            claims_stream_delivery: false,
            receives_progress_deltas: true,
            receives_task_updates: false,
            events: Arc::clone(&events),
            reporting_calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let mut dispatcher = dispatcher_with(outbound.clone());

        dispatcher.send_progress_delta("progress").await;
        dispatcher.send_delta("final").await;
        dispatcher.finish().await;

        let events = events.lock().await;
        assert_eq!(event_kinds(&events), vec!["progress", "delta", "done"]);
        assert_eq!(outbound.reporting_calls.load(Ordering::SeqCst), 1);
        assert_eq!(dispatcher.completed_target_keys().await.len(), 1);
    }

    #[tokio::test]
    async fn claimed_stream_delivery_completes_without_a_final_delta() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let outbound = Arc::new(RecordingStreamOutbound {
            streams_final_replies: true,
            claims_stream_delivery: true,
            receives_progress_deltas: true,
            receives_task_updates: false,
            events: Arc::clone(&events),
            reporting_calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let mut dispatcher = dispatcher_with(outbound);

        dispatcher
            .send_progress_delta("retained error response")
            .await;
        dispatcher.finish().await;

        assert_eq!(event_kinds(&events.lock().await), vec!["progress", "done"]);
        assert_eq!(dispatcher.completed_target_keys().await.len(), 1);
    }

    #[tokio::test]
    async fn unfinished_tasks_are_marked_error_before_stream_completion() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let outbound = Arc::new(RecordingStreamOutbound {
            streams_final_replies: true,
            claims_stream_delivery: false,
            receives_progress_deltas: false,
            receives_task_updates: true,
            events: Arc::clone(&events),
            reporting_calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let mut dispatcher = dispatcher_with(outbound);

        dispatcher
            .send_task_update(moltis_channels::plugin::ChannelTaskUpdate {
                id: "call-1".into(),
                title: "web_search".into(),
                status: moltis_channels::plugin::ChannelTaskStatus::InProgress,
            })
            .await;
        dispatcher.finish().await;

        let events = events.lock().await;
        assert_eq!(event_kinds(&events), vec!["task", "task", "done"]);
        let Some(moltis_channels::StreamEvent::TaskUpdate(update)) = events.get(1) else {
            panic!("expected terminal task update");
        };
        let Some(moltis_channels::StreamEvent::TaskUpdate(started)) = events.first() else {
            panic!("expected starting task update");
        };
        assert_eq!(started.id, "task-1");
        assert_eq!(update.id, started.id);
        assert_eq!(
            update.status,
            moltis_channels::plugin::ChannelTaskStatus::Error
        );
    }

    #[tokio::test]
    async fn dropping_dispatcher_aborts_worker_before_sender_closure_can_finalize() {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
        let outbound = Arc::new(ClosureFinalizingOutbound {
            started: Mutex::new(Some(started_tx)),
            dropped: Mutex::new(Some(dropped_tx)),
            finalized_on_close: AtomicBool::new(false),
        });
        let mut dispatcher = dispatcher_with(outbound.clone());
        dispatcher.send_delta("final").await;
        assert!(started_rx.await.is_ok());

        drop(dispatcher);

        assert!(
            tokio::time::timeout(Duration::from_secs(1), dropped_rx)
                .await
                .is_ok()
        );
        assert!(!outbound.finalized_on_close.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn terminal_is_committed_before_channel_stream_final() {
        let terminal_runs = Arc::new(RwLock::new(HashSet::new()));
        let outbound = Arc::new(TerminalCheckingOutbound {
            terminal_runs: Arc::clone(&terminal_runs),
            final_observed_after_terminal: AtomicBool::new(false),
        });
        let mut dispatcher = dispatcher_with(outbound.clone());
        dispatcher.send_delta("final").await;

        let completed = commit_terminal_and_finish_channel_stream(
            &terminal_runs,
            "run-1",
            Some(&mut dispatcher),
        )
        .await;

        assert!(terminal_runs.read().await.contains("run-1"));
        assert!(
            outbound
                .final_observed_after_terminal
                .load(Ordering::SeqCst)
        );
        assert_eq!(completed.len(), 1);
    }
}
