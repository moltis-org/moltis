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

/// Tool call ids requested by an assistant message, if any.
fn requested_tool_call_ids(message: &Value) -> Vec<&str> {
    message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .filter_map(|call| call.get("id").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default()
}

/// Keep only history belonging to runs that were themselves public.
///
/// Runs are the unit of visibility: a message is kept when its `run_id` matches
/// a run whose user message was marked `channel.private_context = false`.
/// Messages with no `run_id` are never public.
///
/// Further passes then repair tool-call pairing, in both directions. Providers
/// reject a `tool_calls` entry with no matching result *and* a tool result with
/// no preceding call, so a filter that can orphan either turns a privacy
/// decision into a hard request failure. Doing it here keeps the output valid by
/// construction, rather than relying on every persistence site remembering to
/// stamp a `run_id`.
pub(crate) fn filter_public_history(history: Vec<Value>) -> Vec<Value> {
    let public_run_ids: HashSet<&str> = history
        .iter()
        .filter(|message| {
            message
                .get("channel")
                .and_then(|channel| channel.get("private_context"))
                .and_then(Value::as_bool)
                == Some(false)
        })
        .filter_map(|message| message.get("run_id").and_then(Value::as_str))
        .collect();

    let public: Vec<&Value> = history
        .iter()
        .filter(|message| {
            message
                .get("run_id")
                .and_then(Value::as_str)
                .is_some_and(|run_id| public_run_ids.contains(run_id))
        })
        .collect();

    // Results that survived the run filter, so we can tell which calls are
    // fully answered. A partially answered call is as invalid as an unanswered
    // one, so it is dropped whole.
    let answered: HashSet<&str> = public
        .iter()
        .filter_map(|message| message.get("tool_call_id").and_then(Value::as_str))
        .collect();
    let kept: Vec<&&Value> = public
        .iter()
        .filter(|message| {
            requested_tool_call_ids(message)
                .iter()
                .all(|id| answered.contains(id))
        })
        .collect();

    // Dropping an assistant message can strand the results it asked for, so the
    // reverse direction needs the same treatment. This cannot cascade: every
    // surviving call had all of its results in `answered`, and only results no
    // surviving call requested are removed here.
    let requested: HashSet<&str> = kept
        .iter()
        .flat_map(|message| requested_tool_call_ids(message))
        .collect();

    kept.iter()
        .filter(|message| {
            message
                .get("tool_call_id")
                .and_then(Value::as_str)
                .is_none_or(|id| requested.contains(id))
        })
        .map(|message| (**message).clone())
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

    #[test]
    fn keeps_a_tool_call_whose_result_is_in_the_same_run() {
        let history = vec![
            serde_json::json!({
                "role": "user",
                "content": "search",
                "run_id": "public-run",
                "channel": {"sender_id": "guest", "private_context": false},
            }),
            serde_json::json!({
                "role": "assistant",
                "tool_calls": [{
                    "id": "call-1",
                    "type": "function",
                    "function": {"name": "web_search", "arguments": "{}"},
                }],
                "run_id": "public-run",
            }),
            serde_json::json!({
                "role": "tool_result",
                "tool_call_id": "call-1",
                "tool_name": "web_search",
                "run_id": "public-run",
            }),
        ];

        assert_eq!(filter_public_history(history).len(), 3);
    }

    /// A tool result persisted without a `run_id` is dropped by the run filter.
    /// The assistant message that requested it must go too — a `tool_calls`
    /// entry with no matching result is rejected by most providers, so leaving
    /// it behind would turn a filtered turn into a failed request.
    #[test]
    fn drops_a_tool_call_whose_result_did_not_survive() {
        let history = vec![
            serde_json::json!({
                "role": "user",
                "content": "search",
                "run_id": "public-run",
                "channel": {"sender_id": "guest", "private_context": false},
            }),
            serde_json::json!({
                "role": "assistant",
                "tool_calls": [{
                    "id": "call-1",
                    "type": "function",
                    "function": {"name": "web_search", "arguments": "{}"},
                }],
                "run_id": "public-run",
            }),
            serde_json::json!({
                "role": "tool_result",
                "tool_call_id": "call-1",
                "tool_name": "web_search",
            }),
            serde_json::json!({
                "role": "assistant",
                "content": "here you go",
                "run_id": "public-run",
            }),
        ];

        let filtered = filter_public_history(history);
        assert_eq!(filtered.len(), 2, "{filtered:?}");
        assert!(
            filtered
                .iter()
                .all(|message| message.get("tool_calls").is_none()),
            "an unanswered tool call must not survive: {filtered:?}"
        );
    }

    /// The reverse orphan: dropping a partially answered assistant message
    /// strands the result that *did* survive. A tool result with no preceding
    /// call is rejected just like an unanswered call, so it has to go too.
    #[test]
    fn drops_a_result_whose_calling_message_was_dropped() {
        let history = vec![
            serde_json::json!({
                "role": "user",
                "content": "two things",
                "run_id": "public-run",
                "channel": {"sender_id": "guest", "private_context": false},
            }),
            serde_json::json!({
                "role": "assistant",
                "tool_calls": [{"id": "call-1"}, {"id": "call-2"}],
                "run_id": "public-run",
            }),
            // Only one of the two results carries the run id.
            serde_json::json!({
                "role": "tool_result",
                "tool_call_id": "call-1",
                "run_id": "public-run",
            }),
            serde_json::json!({
                "role": "tool_result",
                "tool_call_id": "call-2",
            }),
        ];

        let filtered = filter_public_history(history);
        assert!(
            filtered
                .iter()
                .all(|message| message.get("tool_calls").is_none()
                    && message.get("tool_call_id").is_none()),
            "neither half of a broken pair may survive: {filtered:?}"
        );
        assert_eq!(filtered.len(), 1, "only the user message remains");
    }

    /// Partially answered calls are just as invalid as fully unanswered ones.
    #[test]
    fn drops_a_multi_call_message_when_any_result_is_missing() {
        let history = vec![
            serde_json::json!({
                "role": "user",
                "content": "two things",
                "run_id": "public-run",
                "channel": {"sender_id": "guest", "private_context": false},
            }),
            serde_json::json!({
                "role": "assistant",
                "tool_calls": [{"id": "call-1"}, {"id": "call-2"}],
                "run_id": "public-run",
            }),
            serde_json::json!({
                "role": "tool_result",
                "tool_call_id": "call-1",
                "run_id": "public-run",
            }),
        ];

        let filtered = filter_public_history(history);
        assert!(
            filtered
                .iter()
                .all(|message| message.get("tool_calls").is_none()),
            "{filtered:?}"
        );
    }
}
