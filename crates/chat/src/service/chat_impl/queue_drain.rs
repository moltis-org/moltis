//! Replay of messages that queued behind an active run.

use std::{collections::HashMap, sync::Arc};

use {
    serde_json::Value,
    tokio::sync::{OwnedSemaphorePermit, RwLock},
    tracing::{info, warn},
};

use moltis_config::MessageQueueMode;

use crate::{
    runtime::ChatRuntime,
    service::{
        LiveChatService,
        types::{QueuedMessage, SessionMessageQueue},
    },
};

type MessageQueues = Arc<RwLock<HashMap<String, SessionMessageQueue>>>;

impl LiveChatService {
    pub(super) async fn finish_unstarted_turn(
        &self,
        activity_id: &str,
        session_key: &str,
        permit: OwnedSemaphorePermit,
        queued_replay: bool,
    ) {
        self.state
            .finalize_active_channel_acks(activity_id, moltis_channels::ChannelAckOutcome::Failure)
            .await;
        drop(permit);
        if !queued_replay {
            drain_and_replay(
                &self.message_queue,
                session_key,
                self.config.chat.message_queue_mode,
                &self.state,
            )
            .await;
        }
    }
}

/// Drain in FIFO order. A replay that starts a run delegates the remaining
/// drain to that run; a replay with no run settles its ack and continues here.
pub(super) async fn drain_and_replay(
    message_queue: &MessageQueues,
    session_key: &str,
    mode: MessageQueueMode,
    state: &Arc<dyn ChatRuntime>,
) {
    let chat = state.chat_service().await;
    loop {
        let queued = take_next(message_queue, session_key, mode).await;
        if queued.is_empty() {
            finish_draining(message_queue, session_key).await;
            return;
        }

        let (replay_params, keys) = match mode {
            MessageQueueMode::Followup => {
                let Some(first) = queued.into_iter().next() else {
                    continue;
                };
                info!(session = %session_key, "replaying queued message (followup)");
                let keys = crate::channel_acks::ack_keys_from_params(&first.params);
                let mut params = first.params;
                params["_queued_replay"] = Value::Bool(true);
                (Some(params), keys)
            },
            MessageQueueMode::Collect => prepare_collected(state, session_key, queued).await,
        };
        let Some(replay_params) = replay_params else {
            continue;
        };

        let result = chat.send(replay_params).await;
        settle_replay(state, session_key, keys, &result).await;
        match replay_disposition(&result) {
            ReplayDisposition::Terminal => continue,
            ReplayDisposition::Started | ReplayDisposition::Queued => {
                finish_draining(message_queue, session_key).await;
                return;
            },
        }
    }
}

async fn take_next(
    message_queue: &MessageQueues,
    session_key: &str,
    mode: MessageQueueMode,
) -> Vec<QueuedMessage> {
    let mut queues = message_queue.write().await;
    let Some(queue) = queues.get_mut(session_key) else {
        return Vec::new();
    };
    queue.draining = true;
    match mode {
        MessageQueueMode::Followup => queue.messages.pop_front().into_iter().collect(),
        MessageQueueMode::Collect => {
            let queued = queue.messages.drain(..).collect();
            let (group, rest) = super::tool_policy::split_by_request_security_context(queued);
            // Keep older authorization groups ahead of arrivals that append
            // while this group is replaying.
            queue.messages.extend(rest);
            group
        },
    }
}

async fn finish_draining(message_queue: &MessageQueues, session_key: &str) {
    let mut queues = message_queue.write().await;
    let remove = if let Some(queue) = queues.get_mut(session_key) {
        queue.draining = false;
        queue.messages.is_empty()
    } else {
        false
    };
    if remove {
        queues.remove(session_key);
    }
}

async fn prepare_collected(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    queued: Vec<QueuedMessage>,
) -> (Option<Value>, Vec<String>) {
    let multimodal = queued.iter().any(|m| m.params.get("content").is_some());
    let merged = if multimodal {
        merge_collected_content(&queued)
    } else {
        merge_collected_text(&queued)
    };
    let Some(mut merged) = merged else {
        abandon(state, &queued).await;
        return (None, Vec::new());
    };
    info!(
        session = %session_key,
        count = queued.len(),
        multimodal,
        "replaying collected messages"
    );

    merged["_queued_replay"] = Value::Bool(true);
    // One run now answers every collected message, so it owns all of their
    // acknowledgments and each inbound message gets the terminal reaction.
    let keys = crate::channel_acks::merged_ack_keys(queued.iter().map(|m| &m.params));
    merged[crate::channel_acks::ACK_KEYS_PARAM] = serde_json::json!(keys);
    (Some(merged), keys)
}

/// Merge queued messages into a single multimodal `content` array, keeping the
/// last message's other params (model, session, channel target) as the base.
fn merge_collected_content(queued: &[QueuedMessage]) -> Option<Value> {
    let blocks = merge_content_blocks(queued);
    if blocks.is_empty() {
        return None;
    }
    let mut merged = queued.last()?.params.clone();
    merged["content"] = Value::Array(blocks);
    if let Some(object) = merged.as_object_mut() {
        // The text is now carried as a content block; leaving either alias in
        // place would send it twice.
        object.remove("text");
        object.remove("message");
    }
    Some(merged)
}

fn message_text(params: &Value) -> Option<&str> {
    params
        .get("text")
        .or_else(|| params.get("message"))
        .and_then(Value::as_str)
}

fn merge_collected_text(queued: &[QueuedMessage]) -> Option<Value> {
    let combined: Vec<&str> = queued
        .iter()
        .filter_map(|message| message_text(&message.params))
        .collect();
    if combined.is_empty() {
        return None;
    }
    let mut merged = queued.last()?.params.clone();
    merged["text"] = Value::String(combined.join("\n\n"));
    if let Some(object) = merged.as_object_mut() {
        object.remove("message");
    }
    Some(merged)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplayDisposition {
    Started,
    Queued,
    Terminal,
}

fn replay_disposition(result: &moltis_service_traits::ServiceResult) -> ReplayDisposition {
    let Ok(payload) = result else {
        return ReplayDisposition::Terminal;
    };
    if payload.get("queued").and_then(Value::as_bool) == Some(true) {
        ReplayDisposition::Queued
    } else if payload.get("runId").and_then(Value::as_str).is_some() {
        ReplayDisposition::Started
    } else {
        ReplayDisposition::Terminal
    }
}

async fn settle_replay(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    keys: Vec<String>,
    result: &moltis_service_traits::ServiceResult,
) {
    if let Err(error) = result {
        warn!(session = %session_key, error = %error, "failed to replay queued message");
    }
    if replay_disposition(result) == ReplayDisposition::Terminal && !keys.is_empty() {
        state
            .finalize_channel_acks(keys, moltis_channels::ChannelAckOutcome::Failure)
            .await;
    }
}

async fn abandon(state: &Arc<dyn ChatRuntime>, queued: &[QueuedMessage]) {
    let keys = crate::channel_acks::merged_ack_keys(queued.iter().map(|m| &m.params));
    if !keys.is_empty() {
        state
            .finalize_channel_acks(keys, moltis_channels::ChannelAckOutcome::Failure)
            .await;
    }
}

fn merge_content_blocks(queued: &[QueuedMessage]) -> Vec<Value> {
    let mut blocks = Vec::new();
    for message in queued {
        match message.params.get("content").and_then(Value::as_array) {
            Some(content) => blocks.extend(content.iter().cloned()),
            None => {
                if let Some(text) = message_text(&message.params)
                    && !text.is_empty()
                {
                    blocks.push(serde_json::json!({ "type": "text", "text": text }));
                }
            },
        }
    }
    blocks
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use {
        super::*,
        async_trait::async_trait,
        moltis_service_traits::{ChatService, ServiceResult},
        std::{collections::HashSet, sync::Mutex},
        tokio::sync::{OwnedSemaphorePermit, Semaphore},
    };

    struct ReplayChat {
        semaphore: Arc<Semaphore>,
        sent: Mutex<Vec<Value>>,
        permits: Mutex<Vec<OwnedSemaphorePermit>>,
    }

    #[async_trait]
    impl ChatService for ReplayChat {
        async fn send(&self, params: Value) -> ServiceResult {
            let permit = Arc::clone(&self.semaphore)
                .try_acquire_owned()
                .map_err(|_| "session permit unavailable during replay")?;
            self.sent
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(params);
            self.permits
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(permit);
            Ok(serde_json::json!({"runId": "replayed-run"}))
        }

        async fn abort(&self, _params: Value) -> ServiceResult {
            Ok(Value::Null)
        }

        async fn cancel_queued(&self, _params: Value) -> ServiceResult {
            Ok(Value::Null)
        }

        async fn history(&self, _params: Value) -> ServiceResult {
            Ok(serde_json::json!([]))
        }

        async fn inject(&self, _params: Value) -> ServiceResult {
            Ok(Value::Null)
        }

        async fn clear(&self, _params: Value) -> ServiceResult {
            Ok(Value::Null)
        }

        async fn compact(&self, _params: Value) -> ServiceResult {
            Ok(Value::Null)
        }

        async fn context(&self, _params: Value) -> ServiceResult {
            Ok(Value::Null)
        }

        async fn raw_prompt(&self, _params: Value) -> ServiceResult {
            Ok(Value::Null)
        }

        async fn full_context(&self, _params: Value) -> ServiceResult {
            Ok(Value::Null)
        }
    }

    struct ReplayRuntime {
        chat: Arc<ReplayChat>,
        tts: moltis_service_traits::NoopTtsService,
        project: moltis_service_traits::NoopProjectService,
        mcp: moltis_service_traits::NoopMcpService,
    }

    #[async_trait]
    impl ChatRuntime for ReplayRuntime {
        async fn broadcast(&self, _topic: &str, _payload: Value) {}

        async fn push_channel_reply(
            &self,
            _session_key: &str,
            _target: moltis_channels::ChannelReplyTarget,
        ) {
        }

        async fn drain_channel_replies(
            &self,
            _session_key: &str,
        ) -> Vec<moltis_channels::ChannelReplyTarget> {
            Vec::new()
        }

        async fn peek_channel_replies(
            &self,
            _session_key: &str,
        ) -> Vec<moltis_channels::ChannelReplyTarget> {
            Vec::new()
        }

        async fn push_channel_status_log(&self, _session_key: &str, _message: String) {}

        async fn drain_channel_status_log(&self, _session_key: &str) -> Vec<String> {
            Vec::new()
        }

        async fn set_run_error(&self, _run_id: &str, _error: String) {}

        async fn active_session_key(&self, _conn_id: &str) -> Option<String> {
            None
        }

        async fn active_project_id(&self, _conn_id: &str) -> Option<String> {
            None
        }

        fn hostname(&self) -> &str {
            "test"
        }

        fn sandbox_router(&self) -> Option<&Arc<moltis_tools::sandbox::SandboxRouter>> {
            None
        }

        fn memory_manager(&self) -> Option<&moltis_memory::runtime::DynMemoryRuntime> {
            None
        }

        async fn cached_location(&self) -> Option<moltis_config::GeoLocation> {
            None
        }

        async fn tts_overrides(
            &self,
            _session_key: &str,
            _channel_key: &str,
        ) -> (
            Option<crate::runtime::TtsOverride>,
            Option<crate::runtime::TtsOverride>,
        ) {
            (None, None)
        }

        fn channel_outbound(&self) -> Option<Arc<dyn moltis_channels::ChannelOutbound>> {
            None
        }

        fn channel_stream_outbound(
            &self,
        ) -> Option<Arc<dyn moltis_channels::ChannelStreamOutbound>> {
            None
        }

        fn tts_service(&self) -> &dyn moltis_service_traits::TtsService {
            &self.tts
        }

        fn project_service(&self) -> &dyn moltis_service_traits::ProjectService {
            &self.project
        }

        fn mcp_service(&self) -> &dyn moltis_service_traits::McpService {
            &self.mcp
        }

        async fn chat_service(&self) -> Arc<dyn ChatService> {
            self.chat.clone()
        }

        async fn last_run_error(&self, _run_id: &str) -> Option<String> {
            None
        }

        async fn send_push_notification(
            &self,
            _title: &str,
            _body: &str,
            _url: Option<&str>,
            _session_key: Option<&str>,
        ) -> crate::error::Result<usize> {
            Ok(0)
        }

        async fn ensure_local_model_cached(&self, _model_id: &str) -> crate::error::Result<bool> {
            Ok(false)
        }

        async fn connected_nodes(&self) -> Vec<crate::runtime::ConnectedNodeSummary> {
            Vec::new()
        }
    }

    fn msg(value: Value) -> QueuedMessage {
        QueuedMessage { params: value }
    }

    #[test]
    fn terminal_results_require_same_drainer_to_continue() {
        assert_eq!(
            replay_disposition(&Err("failed".into())),
            ReplayDisposition::Terminal
        );
        assert_eq!(
            replay_disposition(&Ok(serde_json::json!({ "rejected": true }))),
            ReplayDisposition::Terminal
        );
        assert_eq!(
            replay_disposition(&Ok(serde_json::json!({ "runId": "next" }))),
            ReplayDisposition::Started
        );
    }

    #[tokio::test]
    async fn followup_take_preserves_fifo_after_terminal_replay() {
        let queues = Arc::new(RwLock::new(HashMap::from([(
            "s".to_string(),
            SessionMessageQueue {
                messages: [
                    msg(serde_json::json!({"text": "one"})),
                    msg(serde_json::json!({"text": "two"})),
                ]
                .into_iter()
                .collect(),
                draining: false,
            },
        )])));

        let first = take_next(&queues, "s", MessageQueueMode::Followup).await;
        let terminal_result = Err("no run spawned".into());
        let second = if replay_disposition(&terminal_result) == ReplayDisposition::Terminal {
            take_next(&queues, "s", MessageQueueMode::Followup).await
        } else {
            Vec::new()
        };
        assert_eq!(first[0].params["text"], "one");
        assert_eq!(second[0].params["text"], "two");
    }

    #[tokio::test]
    async fn collect_take_splits_authorization_groups_without_reordering() {
        let queues = Arc::new(RwLock::new(HashMap::from([(
            "s".to_string(),
            SessionMessageQueue {
                messages: [
                    msg(serde_json::json!({
                        "text": "guest-one",
                        "_tool_audience": "public",
                        "_tool_policy": {"deny": ["*"]},
                    })),
                    msg(serde_json::json!({
                        "text": "guest-two",
                        "_tool_audience": "public",
                        "_tool_policy": {"deny": ["*"]},
                    })),
                    msg(serde_json::json!({"text": "operator"})),
                ]
                .into_iter()
                .collect(),
                draining: false,
            },
        )])));

        let guest = take_next(&queues, "s", MessageQueueMode::Collect).await;
        queues
            .write()
            .await
            .get_mut("s")
            .unwrap()
            .messages
            .push_back(msg(serde_json::json!({"text": "new-arrival"})));
        let operator = take_next(&queues, "s", MessageQueueMode::Collect).await;

        assert_eq!(guest.len(), 2);
        assert_eq!(guest[0].params["text"], "guest-one");
        assert_eq!(guest[1].params["text"], "guest-two");
        assert_eq!(operator[0].params["text"], "operator");
        assert_eq!(operator[1].params["text"], "new-arrival");
    }

    #[tokio::test]
    async fn unavailable_abort_does_not_strand_replay_when_run_completes() {
        let semaphore = Arc::new(Semaphore::new(1));
        let active_permit = Arc::clone(&semaphore).acquire_owned().await.unwrap();
        let task = tokio::spawn(std::future::pending::<()>());
        let active_runs = Arc::new(RwLock::new(HashMap::from([(
            "run-1".to_string(),
            task.abort_handle(),
        )])));
        let by_session = Arc::new(RwLock::new(HashMap::from([(
            "s".to_string(),
            "run-1".to_string(),
        )])));
        let terminal = Arc::new(RwLock::new(HashSet::from(["run-1".to_string()])));
        let (_, claim) = LiveChatService::claim_run_for_abort(
            &active_runs,
            &by_session,
            &terminal,
            Arc::clone(&semaphore),
            Some("run-1"),
            Some("s"),
        )
        .await;
        assert_eq!(
            claim.disposition,
            crate::service::types::RunHandleDisposition::Unavailable
        );
        assert!(claim.cleanup.is_none());

        let queues = Arc::new(RwLock::new(HashMap::from([(
            "s".to_string(),
            SessionMessageQueue {
                messages: [msg(serde_json::json!({"text": "queued"}))]
                    .into_iter()
                    .collect(),
                draining: false,
            },
        )])));
        let chat = Arc::new(ReplayChat {
            semaphore,
            sent: Mutex::new(Vec::new()),
            permits: Mutex::new(Vec::new()),
        });
        let runtime: Arc<dyn ChatRuntime> = Arc::new(ReplayRuntime {
            chat: Arc::clone(&chat),
            tts: moltis_service_traits::NoopTtsService,
            project: moltis_service_traits::NoopProjectService,
            mcp: moltis_service_traits::NoopMcpService,
        });

        // Completion releases the permit after abort has lost the terminal race,
        // then follows the production queue-drain path.
        drop(active_permit);
        drain_and_replay(&queues, "s", MessageQueueMode::Followup, &runtime).await;

        assert_eq!(
            chat.sent.lock().unwrap_or_else(|error| error.into_inner())[0]["text"],
            "queued"
        );
        assert!(!queues.read().await.contains_key("s"));
        task.abort();
    }

    #[test]
    fn merges_text_and_image_blocks_in_order() {
        let queued = vec![
            msg(serde_json::json!({ "text": "first" })),
            msg(serde_json::json!({ "content": [
                { "type": "text", "text": "second" },
                { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAA" } }
            ]})),
            msg(serde_json::json!({ "text": "third" })),
        ];
        let blocks = merge_content_blocks(&queued);
        assert_eq!(blocks.len(), 4);
        assert_eq!(blocks[0]["text"], "first");
        assert_eq!(blocks[1]["text"], "second");
        assert_eq!(blocks[2]["type"], "image_url");
        assert_eq!(blocks[3]["text"], "third");
    }

    #[test]
    fn collect_normalizes_text_and_message_alias_without_dropping_content() {
        let queued = vec![
            msg(serde_json::json!({ "text": "first", "source": 1 })),
            msg(serde_json::json!({ "message": "second", "source": 2 })),
            msg(serde_json::json!({ "text": "third", "message": "ignored alias", "source": 3 })),
        ];

        let Some(merged) = merge_collected_text(&queued) else {
            panic!("expected collected text");
        };
        assert_eq!(merged["text"], "first\n\nsecond\n\nthird");
        assert!(merged.get("message").is_none());
        assert_eq!(merged["source"], 3);
    }

    #[test]
    fn multimodal_collect_preserves_message_alias_as_text_block() {
        let queued = vec![
            msg(serde_json::json!({ "message": "aliased" })),
            msg(serde_json::json!({ "content": [
                { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAA" } }
            ]})),
        ];

        let blocks = merge_content_blocks(&queued);
        assert_eq!(
            blocks[0],
            serde_json::json!({ "type": "text", "text": "aliased" })
        );
        assert_eq!(blocks[1]["type"], "image_url");
    }
}
