//! Replay of messages that queued behind an active run.
//!
//! Shared by both run paths (model turns and explicit `/sh` commands), which
//! previously carried identical copies of this logic.

use std::{collections::HashMap, sync::Arc};

use {
    serde_json::Value,
    tokio::sync::RwLock,
    tracing::{info, warn},
};

use moltis_config::MessageQueueMode;

use crate::{runtime::ChatRuntime, service::types::QueuedMessage};

/// Drain this session's queue and replay it according to `mode`.
///
/// `Followup` replays the oldest message and puts the rest back, so the
/// replayed run drains them in turn. `Collect` merges every queued message into
/// one turn — which therefore owns all of their acknowledgment identities, so a
/// reaction is resolved on each of the combined messages rather than only the
/// last.
pub(super) async fn drain_and_replay(
    message_queue: &Arc<RwLock<HashMap<String, Vec<QueuedMessage>>>>,
    session_key: &str,
    mode: MessageQueueMode,
    state: &Arc<dyn ChatRuntime>,
) {
    let queued = message_queue
        .write()
        .await
        .remove(session_key)
        .unwrap_or_default();
    if queued.is_empty() {
        return;
    }
    let chat = state.chat_service().await;
    match mode {
        MessageQueueMode::Followup => {
            let mut iter = queued.into_iter();
            let Some(first) = iter.next() else {
                return;
            };
            // Put remaining messages back so the replayed run's own drain loop
            // picks them up after it completes.
            let rest: Vec<QueuedMessage> = iter.collect();
            if !rest.is_empty() {
                message_queue
                    .write()
                    .await
                    .entry(session_key.to_string())
                    .or_default()
                    .extend(rest);
            }
            info!(session = %session_key, "replaying queued message (followup)");
            let mut replay_params = first.params;
            replay_params["_queued_replay"] = Value::Bool(true);
            if let Err(e) = chat.send(replay_params).await {
                warn!(session = %session_key, error = %e, "failed to replay queued message");
            }
        },
        MessageQueueMode::Collect => {
            let combined: Vec<&str> = queued
                .iter()
                .filter_map(|m| m.params.get("text").and_then(|v| v.as_str()))
                .collect();
            if combined.is_empty() {
                return;
            }
            info!(
                session = %session_key,
                count = combined.len(),
                "replaying collected messages"
            );
            // Use the last queued message as the base params, override text.
            let Some(last) = queued.last() else {
                return;
            };
            let mut merged = last.params.clone();
            merged["text"] = Value::String(combined.join("\n\n"));
            merged["_queued_replay"] = Value::Bool(true);
            // One run answers all of them, so it owns every acknowledgment.
            merged[crate::channel_acks::ACK_KEYS_PARAM] = serde_json::json!(
                crate::channel_acks::merged_ack_keys(queued.iter().map(|m| &m.params))
            );
            if let Err(e) = chat.send(merged).await {
                warn!(session = %session_key, error = %e, "failed to replay collected messages");
            }
        },
    }
}
