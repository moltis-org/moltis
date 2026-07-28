use std::collections::HashSet;

use serde_json::Value;

pub(crate) fn mark_public_channel(params: &mut Value) {
    if !params.get("channel").is_some_and(Value::is_object) {
        params["channel"] = serde_json::json!({});
    }
    if let Some(channel) = params.get_mut("channel").and_then(Value::as_object_mut) {
        channel.insert("private_context".to_string(), Value::Bool(false));
    }
}

pub(crate) fn filter_public_history(history: Vec<Value>) -> Vec<Value> {
    let public_run_ids: HashSet<String> = history
        .iter()
        .filter(|message| {
            message
                .get("channel")
                .and_then(|channel| channel.get("private_context"))
                .and_then(Value::as_bool)
                == Some(false)
        })
        .filter_map(|message| message.get("run_id").and_then(Value::as_str))
        .map(str::to_string)
        .collect();

    history
        .into_iter()
        .filter(|message| {
            message
                .get("run_id")
                .and_then(Value::as_str)
                .is_some_and(|run_id| public_run_ids.contains(run_id))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{filter_public_history, mark_public_channel};

    #[test]
    fn public_marker_is_created_when_channel_metadata_is_absent() {
        let mut params = serde_json::json!({"text": "webhook"});
        mark_public_channel(&mut params);
        assert_eq!(params["channel"]["private_context"], false);
    }

    #[test]
    fn keeps_only_explicitly_public_runs() {
        let history = vec![
            serde_json::json!({
                "role": "user",
                "content": "private request",
                "run_id": "private-run",
                "channel": {"sender_id": "owner"},
            }),
            serde_json::json!({
                "role": "assistant",
                "content": "private response",
                "run_id": "private-run",
            }),
            serde_json::json!({
                "role": "user",
                "content": "public request",
                "run_id": "public-run",
                "channel": {"sender_id": "guest", "private_context": false},
            }),
            serde_json::json!({
                "role": "assistant",
                "content": "public response",
                "run_id": "public-run",
            }),
        ];

        let filtered = filter_public_history(history);
        assert_eq!(filtered.len(), 2);
        assert!(
            filtered
                .iter()
                .all(|message| message["run_id"] == "public-run")
        );
    }
}
