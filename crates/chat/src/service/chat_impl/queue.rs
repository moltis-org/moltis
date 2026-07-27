use std::{collections::HashMap, sync::Arc};

use {
    moltis_config::MessageQueueMode,
    tokio::sync::RwLock,
    tracing::{info, warn},
};

use crate::{ChatRuntime, service::QueuedMessage};

pub(super) async fn drain_queued_messages(
    message_queue: &Arc<RwLock<HashMap<String, Vec<QueuedMessage>>>>,
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    queue_mode: MessageQueueMode,
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
    match queue_mode {
        MessageQueueMode::Followup => {
            let mut iter = queued.into_iter();
            let Some(first) = iter.next() else {
                return;
            };
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
            replay_params["_queued_replay"] = serde_json::json!(true);
            if let Err(error) = chat.send(replay_params).await {
                warn!(session = %session_key, %error, "failed to replay queued message");
            }
        },
        MessageQueueMode::Collect => {
            let combined = queued
                .iter()
                .filter_map(|message| message.params.get("text").and_then(|value| value.as_str()))
                .collect::<Vec<_>>();
            if combined.is_empty() {
                return;
            }
            info!(session = %session_key, count = combined.len(), "replaying collected messages");
            let Some(last) = queued.last() else {
                return;
            };
            let mut merged = last.params.clone();
            merged["text"] = serde_json::json!(combined.join("\n\n"));
            merged["_queued_replay"] = serde_json::json!(true);
            if let Err(error) = chat.send(merged).await {
                warn!(session = %session_key, %error, "failed to replay collected messages");
            }
        },
    }
}
