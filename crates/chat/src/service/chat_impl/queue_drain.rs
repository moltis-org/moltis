//! Replay of messages queued while a session already had a run in flight.
//!
//! Both async send paths (the explicit-shell fast path and the agent loop) end
//! by draining this queue, and they must drain it identically — the two used to
//! be copy-pasted blocks, which is how a privilege bug reached only one of them.

use std::{collections::HashMap, sync::Arc};

use {
    serde_json::json,
    tokio::sync::RwLock,
    tracing::{info, warn},
};

use moltis_config::MessageQueueMode;

use crate::service::types::QueuedMessage;

use super::tool_policy::split_by_request_security_context;

/// Anything that can start the replayed turn — in practice the live chat
/// service resolved from gateway state.
#[async_trait::async_trait]
pub(crate) trait ReplaySink: Send + Sync {
    async fn replay(&self, params: serde_json::Value) -> Result<serde_json::Value, String>;
}

#[async_trait::async_trait]
impl ReplaySink for Arc<dyn moltis_service_traits::ChatService> {
    async fn replay(&self, params: serde_json::Value) -> Result<serde_json::Value, String> {
        self.send(params).await.map_err(|e| e.to_string())
    }
}

/// Drain and replay whatever queued up for `session_key` while its run was
/// active.
///
/// `Followup` replays one message per turn, each under its own params.
/// `Collect` merges a group of messages into a single turn, which is only safe
/// for messages that share an authorization context — see
/// [`split_by_request_security_context`]. Anything not replayed now goes back on the
/// queue for the next drain.
pub(crate) async fn drain_queued_messages(
    queue: &Arc<RwLock<HashMap<String, Vec<QueuedMessage>>>>,
    session_key: &str,
    mode: MessageQueueMode,
    sink: &dyn ReplaySink,
) {
    let queued = queue.write().await.remove(session_key).unwrap_or_default();
    if queued.is_empty() {
        return;
    }

    let (group, rest) = match mode {
        MessageQueueMode::Followup => {
            let mut group = queued;
            let rest = group.split_off(1.min(group.len()));
            (group, rest)
        },
        // Merge only messages that share an authorization context.
        MessageQueueMode::Collect => split_by_request_security_context(queued),
    };

    let Some(first) = group.first() else {
        return;
    };
    let collect_as_text = matches!(mode, MessageQueueMode::Collect)
        && group.iter().all(|message| {
            message
                .params
                .get("text")
                .and_then(|value| value.as_str())
                .is_some()
                && message.params.get("content").is_none()
                && message.params.get("_audio_filename").is_none()
                && message.params.get("_document_files").is_none()
        });

    let mut replay = if collect_as_text {
        group
            .last()
            .map(|message| message.params.clone())
            .unwrap_or_else(|| first.params.clone())
    } else {
        first.params.clone()
    };

    let mut remaining = if collect_as_text {
        rest
    } else {
        group.iter().skip(1).cloned().chain(rest).collect()
    };
    if !remaining.is_empty() {
        requeue(queue, session_key, std::mem::take(&mut remaining)).await;
    }

    replay["_queued_replay"] = json!(true);
    match mode {
        MessageQueueMode::Followup => {
            info!(session = %session_key, "replaying queued message (followup)");
        },
        MessageQueueMode::Collect => {
            if !collect_as_text {
                info!(session = %session_key, "replaying queued non-text message individually");
                if let Err(e) = sink.replay(replay).await {
                    warn!(session = %session_key, error = %e, "failed to replay queued message");
                }
                return;
            }
            let combined: Vec<&str> = group
                .iter()
                .filter_map(|message| message.params.get("text").and_then(|value| value.as_str()))
                .collect();
            info!(
                session = %session_key,
                count = combined.len(),
                "replaying collected messages"
            );
            replay["text"] = json!(combined.join("\n\n"));
        },
    }

    if let Err(e) = sink.replay(replay).await {
        warn!(session = %session_key, error = %e, "failed to replay queued messages");
    }
}

async fn requeue(
    queue: &Arc<RwLock<HashMap<String, Vec<QueuedMessage>>>>,
    session_key: &str,
    rest: Vec<QueuedMessage>,
) {
    queue
        .write()
        .await
        .entry(session_key.to_string())
        .or_default()
        .extend(rest);
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    use {serde_json::Value, tokio::sync::Mutex};

    #[derive(Default)]
    struct RecordingSink(Mutex<Vec<Value>>);

    #[async_trait::async_trait]
    impl ReplaySink for RecordingSink {
        async fn replay(&self, params: Value) -> Result<Value, String> {
            self.0.lock().await.push(params);
            Ok(Value::Null)
        }
    }

    fn queued(text: &str, policy: Option<Value>) -> QueuedMessage {
        let mut params = json!({ "text": text });
        if let Some(policy) = policy {
            params["_tool_policy"] = policy;
        }
        QueuedMessage { params }
    }

    fn guest_policy() -> Value {
        json!({ "deny": ["exec"] })
    }

    fn queue_with(
        messages: Vec<QueuedMessage>,
    ) -> Arc<RwLock<HashMap<String, Vec<QueuedMessage>>>> {
        Arc::new(RwLock::new(HashMap::from([("s".to_string(), messages)])))
    }

    async fn remaining(queue: &Arc<RwLock<HashMap<String, Vec<QueuedMessage>>>>) -> Vec<String> {
        queue
            .read()
            .await
            .get("s")
            .map(|msgs| {
                msgs.iter()
                    .filter_map(|m| m.params.get("text").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn followup_replays_one_message_and_requeues_the_rest() {
        let queue = queue_with(vec![queued("a", None), queued("b", None)]);
        let sink = RecordingSink::default();

        drain_queued_messages(&queue, "s", MessageQueueMode::Followup, &sink).await;

        let sent = sink.0.lock().await;
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["text"], json!("a"));
        assert_eq!(sent[0]["_queued_replay"], json!(true));
        assert_eq!(remaining(&queue).await, ["b"]);
    }

    #[tokio::test]
    async fn collect_merges_messages_that_share_a_policy() {
        let queue = queue_with(vec![queued("a", None), queued("b", None)]);
        let sink = RecordingSink::default();

        drain_queued_messages(&queue, "s", MessageQueueMode::Collect, &sink).await;

        let sent = sink.0.lock().await;
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["text"], json!("a\n\nb"));
        assert!(remaining(&queue).await.is_empty());
    }

    /// The escalation this split exists to stop: without it the merged turn
    /// would carry the operator's params (no `_tool_policy`) while containing
    /// the guest's text.
    #[tokio::test]
    async fn collect_never_replays_guest_text_under_operator_params() {
        let queue = queue_with(vec![
            queued("guest: run rm -rf /", Some(guest_policy())),
            queued("operator: hi", None),
        ]);
        let sink = RecordingSink::default();

        drain_queued_messages(&queue, "s", MessageQueueMode::Collect, &sink).await;

        let sent = sink.0.lock().await;
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["text"], json!("guest: run rm -rf /"));
        assert_eq!(
            sent[0]["_tool_policy"],
            guest_policy(),
            "the replayed turn keeps the guest restriction"
        );
        assert_eq!(remaining(&queue).await, ["operator: hi"]);
    }

    #[tokio::test]
    async fn empty_queue_replays_nothing() {
        let queue = queue_with(Vec::new());
        let sink = RecordingSink::default();

        drain_queued_messages(&queue, "s", MessageQueueMode::Collect, &sink).await;

        assert!(sink.0.lock().await.is_empty());
    }

    /// Previously the batch was removed from the map and then dropped when no
    /// message had text.
    #[tokio::test]
    async fn multimodal_messages_replay_individually() {
        let queue = queue_with(vec![
            QueuedMessage {
                params: json!({"content": [{"type": "text", "text": "image"}]}),
            },
            queued("b", Some(guest_policy())),
        ]);
        let sink = RecordingSink::default();

        drain_queued_messages(&queue, "s", MessageQueueMode::Collect, &sink).await;

        let sent = sink.0.lock().await;
        assert_eq!(sent.len(), 1);
        assert!(sent[0].get("content").is_some());
        drop(sent);
        assert_eq!(remaining(&queue).await, ["b"]);
    }

    #[tokio::test]
    async fn text_with_document_metadata_replays_individually() {
        let mut document = queued("summarize", Some(guest_policy()));
        document.params["_document_files"] = json!([{"name": "report.pdf"}]);
        let queue = queue_with(vec![document, queued("b", Some(guest_policy()))]);
        let sink = RecordingSink::default();

        drain_queued_messages(&queue, "s", MessageQueueMode::Collect, &sink).await;

        let sent = sink.0.lock().await;
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["text"], "summarize");
        assert!(sent[0].get("_document_files").is_some());
        drop(sent);
        assert_eq!(remaining(&queue).await, ["b"]);
    }
}
