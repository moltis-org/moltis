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
            let keys = crate::channel_acks::ack_keys_from_params(&first.params);
            let mut replay_params = first.params;
            replay_params["_queued_replay"] = Value::Bool(true);
            let result = chat.send(replay_params).await;
            settle_replay(state, session_key, keys, &result).await;
        },
        MessageQueueMode::Collect => {
            // Merge *all* content, not just text. `send` prefers `content` over
            // `text` when both are present, so a multimodal message anywhere in
            // the batch would otherwise discard every other message's words
            // while their acknowledgments still reported success.
            let multimodal = queued.iter().any(|m| m.params.get("content").is_some());
            let combined: Vec<&str> = queued
                .iter()
                .filter_map(|m| m.params.get("text").and_then(|v| v.as_str()))
                .collect();
            if multimodal {
                let blocks = merge_content_blocks(&queued);
                if blocks.is_empty() {
                    abandon(state, &queued).await;
                    return;
                }
                let Some(last) = queued.last() else {
                    abandon(state, &queued).await;
                    return;
                };
                info!(
                    session = %session_key,
                    count = queued.len(),
                    "replaying collected messages (multimodal)"
                );
                let mut merged = last.params.clone();
                merged["content"] = Value::Array(blocks);
                // `text` would be ignored anyway; drop it so the two cannot
                // disagree about what was actually sent.
                if let Some(obj) = merged.as_object_mut() {
                    obj.remove("text");
                }
                merged["_queued_replay"] = Value::Bool(true);
                merged[crate::channel_acks::ACK_KEYS_PARAM] = serde_json::json!(
                    crate::channel_acks::merged_ack_keys(queued.iter().map(|m| &m.params))
                );
                let keys = crate::channel_acks::ack_keys_from_params(&merged);
                let result = chat.send(merged).await;
                settle_replay(state, session_key, keys, &result).await;
                return;
            }
            if combined.is_empty() {
                // Nothing replayable (e.g. every queued message was non-text),
                // so no reply will ever come. Resolve their acknowledgments
                // instead of leaving markers to expire.
                abandon(state, &queued).await;
                return;
            }
            info!(
                session = %session_key,
                count = combined.len(),
                "replaying collected messages"
            );
            // Use the last queued message as the base params, override text.
            let Some(last) = queued.last() else {
                abandon(state, &queued).await;
                return;
            };
            let mut merged = last.params.clone();
            merged["text"] = Value::String(combined.join("\n\n"));
            merged["_queued_replay"] = Value::Bool(true);
            // One run answers all of them, so it owns every acknowledgment.
            merged[crate::channel_acks::ACK_KEYS_PARAM] = serde_json::json!(
                crate::channel_acks::merged_ack_keys(queued.iter().map(|m| &m.params))
            );
            let keys = crate::channel_acks::ack_keys_from_params(&merged);
            let result = chat.send(merged).await;
            settle_replay(state, session_key, keys, &result).await;
        },
    }
}

/// Flatten every queued message into one ordered list of content blocks.
///
/// Text-only messages contribute a single text block; multimodal messages
/// contribute their blocks verbatim, so images survive the merge.
fn merge_content_blocks(queued: &[QueuedMessage]) -> Vec<Value> {
    let mut blocks = Vec::new();
    for m in queued {
        match m.params.get("content").and_then(Value::as_array) {
            Some(content) => blocks.extend(content.iter().cloned()),
            None => {
                if let Some(text) = m.params.get("text").and_then(Value::as_str)
                    && !text.is_empty()
                {
                    blocks.push(serde_json::json!({ "type": "text", "text": text }));
                }
            },
        }
    }
    blocks
}

/// Resolve acknowledgments for a replay that never became a run.
///
/// A replay can fail outright, or return a terminal payload (a rejected hook,
/// an error) with no run behind it. Either way nothing else will finalize the
/// reaction, so it is settled here. A replay that queues again keeps its
/// acknowledgment: it will be claimed when it finally runs.
async fn settle_replay(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    keys: Vec<String>,
    result: &moltis_service_traits::ServiceResult,
) {
    let failed = match result {
        Err(e) => {
            warn!(session = %session_key, error = %e, "failed to replay queued message");
            true
        },
        Ok(payload) => {
            if payload.get("queued").and_then(Value::as_bool) == Some(true) {
                false
            } else {
                payload.get("rejected").and_then(Value::as_bool) == Some(true)
                    || matches!(
                        payload.get("state").and_then(Value::as_str),
                        Some("rejected" | "error" | "blocked")
                    )
            }
        },
    };
    if failed && !keys.is_empty() {
        state
            .finalize_channel_acks(keys, moltis_channels::ChannelAckOutcome::Failure)
            .await;
    }
}

/// Resolve the acknowledgments of queued messages that will never be answered.
async fn abandon(state: &Arc<dyn ChatRuntime>, queued: &[QueuedMessage]) {
    let keys = crate::channel_acks::merged_ack_keys(queued.iter().map(|m| &m.params));
    if !keys.is_empty() {
        state
            .finalize_channel_acks(keys, moltis_channels::ChannelAckOutcome::Failure)
            .await;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn msg(v: Value) -> QueuedMessage {
        QueuedMessage { params: v }
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
        // Nothing is dropped: three texts plus the image.
        assert_eq!(blocks.len(), 4);
        assert_eq!(blocks[0]["text"], "first");
        assert_eq!(blocks[1]["text"], "second");
        assert_eq!(blocks[2]["type"], "image_url");
        assert_eq!(blocks[3]["text"], "third");
    }

    #[test]
    fn skips_empty_text_messages() {
        let queued = vec![
            msg(serde_json::json!({ "text": "" })),
            msg(serde_json::json!({ "text": "kept" })),
        ];
        let blocks = merge_content_blocks(&queued);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["text"], "kept");
    }
}
