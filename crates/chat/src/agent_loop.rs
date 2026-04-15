//! Agent loop support: model flagging, shell commands, channel streaming, and compaction.

use std::{collections::HashSet, sync::Arc, time::Instant};

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
    service::{build_tool_call_assistant_message, persist_tool_history_pair},
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
    tasks: Vec<tokio::task::JoinHandle<()>>,
    completed: Arc<Mutex<HashSet<ChannelReplyTargetKey>>>,
    started: bool,
    sent_delta: bool,
}

impl ChannelStreamDispatcher {
    pub(crate) async fn for_session(
        state: &Arc<dyn ChatRuntime>,
        session_key: &str,
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
            started: false,
            sent_delta: false,
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
            let (tx, rx) = mpsc::channel(CHANNEL_STREAM_BUFFER_SIZE);
            let outbound = Arc::clone(&self.outbound);
            let completed = Arc::clone(&self.completed);
            let account_id = target.account_id.clone();
            let to = target.outbound_to().into_owned();
            let reply_to = target.message_id.clone();
            let key_for_insert = key.clone();
            let account_for_log = account_id.clone();
            let chat_for_log = target.chat_id.clone();
            let thread_for_log = target.thread_id.clone();

            self.workers.push(ChannelStreamWorker { sender: tx });
            self.tasks.push(tokio::spawn(async move {
                match outbound
                    .send_stream(&account_id, &to, reply_to.as_deref(), rx)
                    .await
                {
                    Ok(()) => {
                        completed.lock().await.insert(key_for_insert);
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
            }));
        }
    }

    pub(crate) async fn send_delta(&mut self, delta: &str) {
        if delta.is_empty() {
            return;
        }
        self.sent_delta = true;
        self.ensure_started().await;
        let event = moltis_channels::StreamEvent::Delta(delta.to_string());
        for worker in &self.workers {
            if worker.sender.send(event.clone()).await.is_err() {
                debug!("channel stream delta dropped: worker closed");
            }
        }
    }

    pub(crate) async fn finish(&mut self) {
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
            if let Err(e) = task.await {
                warn!(error = %e, "channel stream worker task join failed");
            }
        }
    }

    pub(crate) async fn completed_target_keys(&self) -> HashSet<ChannelReplyTargetKey> {
        if !self.sent_delta {
            return HashSet::new();
        }
        self.completed.lock().await.clone()
    }
}

pub(crate) async fn run_explicit_shell_command(
    state: &Arc<dyn ChatRuntime>,
    run_id: &str,
    tool_registry: &Arc<RwLock<ToolRegistry>>,
    session_store: &Arc<SessionStore>,
    terminal_runs: &Arc<RwLock<HashSet<String>>>,
    session_key: &str,
    command: &str,
    user_message_index: usize,
    accept_language: Option<String>,
    conn_id: Option<String>,
    client_seq: Option<u64>,
) -> AssistantTurnOutput {
    let started = Instant::now();
    let tool_call_id = format!("sh_{}", uuid::Uuid::new_v4().simple());
    let tool_args = serde_json::json!({ "command": command });

    send_tool_status_to_channels(state, session_key, "exec", &tool_args).await;

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
                client_seq,
                Some(run_id),
            );
            let tool_result_msg = PersistedMessage::tool_result(
                tool_call_id.clone(),
                "exec",
                Some(serde_json::json!({ "command": command })),
                true,
                Some(capped.clone()),
                None,
            );
            persist_tool_history_pair(
                session_store,
                session_key,
                assistant_tool_call_msg,
                tool_result_msg,
                "failed to persist direct /sh assistant tool call",
                "failed to persist direct /sh tool result",
            )
            .await;

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
                client_seq,
                Some(run_id),
            );
            let tool_result_msg = PersistedMessage::tool_result(
                tool_call_id.clone(),
                "exec",
                Some(serde_json::json!({ "command": command })),
                false,
                None,
                Some(error_text.clone()),
            );
            persist_tool_history_pair(
                session_store,
                session_key,
                assistant_tool_call_msg,
                tool_result_msg,
                "failed to persist direct /sh assistant tool call",
                "failed to persist direct /sh tool error",
            )
            .await;

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

    if !final_text.trim().is_empty() {
        let streamed_target_keys = HashSet::new();
        deliver_channel_replies(
            state,
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
    terminal_runs.write().await.insert(run_id.to_string());
    broadcast(state, "chat", payload, BroadcastOpts::default()).await;

    build_assistant_turn_output(
        final_text,
        UsageSnapshot::new(
            moltis_agents::model::Usage::default(),
            Some(moltis_agents::model::Usage::default()),
        ),
        started.elapsed().as_millis() as u64,
        None,
        None,
        None,
    )
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
) -> error::Result<compaction_run::CompactionOutcome> {
    let history = store
        .read(session_key)
        .await
        .map_err(|source| error::Error::external("failed to read session history", source))?;

    let mut outcome = compaction_run::run_compaction(&history, config, provider)
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
