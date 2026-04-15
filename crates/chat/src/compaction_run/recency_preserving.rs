//! `CompactionMode::RecencyPreserving` — head + middle-marker + tail.
//!
//! Zero-LLM strategy inspired by hermes-agent's `ContextCompressor`:
//! protect the head (system prompt + first exchange) and a token-budget
//! tail verbatim, collapse the middle into a single marker message, prune
//! any bulky tool-result content that survives in the retained slice, and
//! repair orphaned tool_use / tool_result pairs so strict providers
//! accept the retry.

use {
    moltis_config::{CompactionConfig, CompactionMode},
    serde_json::Value,
    tracing::info,
};

use super::{
    CompactionOutcome, CompactionRunError,
    shared::{
        HeadTailBounds, build_middle_marker, compute_boundaries, finalize_kept,
        in_place_prune_or_err,
    },
};

/// Run the recency-preserving strategy against `history`.
///
/// `context_window` is the provider's model-window size, used to size
/// the token-budget tail cut. Strategy callers resolve it from the
/// session provider and fall back to a sensible default when no
/// provider is available. Token counts on the returned outcome are
/// always zero — no LLM calls are made.
pub(super) fn run(
    history: &[Value],
    config: &CompactionConfig,
    context_window: u32,
) -> Result<CompactionOutcome, CompactionRunError> {
    let bounds = compute_boundaries(history, config, context_window);
    let HeadTailBounds {
        head_end,
        tail_start,
        protect_tail_min,
        ..
    } = bounds;
    let n = history.len();

    let kept = if head_end >= tail_start {
        in_place_prune_or_err(history, config, &bounds)?
    } else {
        let mut kept: Vec<Value> = Vec::with_capacity(head_end + 1 + (n - tail_start));
        kept.extend(history[..head_end].iter().cloned());

        let middle = &history[head_end..tail_start];
        if !middle.is_empty() {
            kept.push(build_middle_marker(middle));
        }

        kept.extend(history[tail_start..].iter().cloned());

        finalize_kept(kept, config, protect_tail_min)?
    };

    info!(
        input_messages = n,
        output_messages = kept.len(),
        head = head_end,
        middle = tail_start - head_end,
        tail = n - tail_start,
        "chat.compact: recency_preserving"
    );

    Ok(CompactionOutcome {
        history: kept,
        effective_mode: CompactionMode::RecencyPreserving,
        input_tokens: 0,
        output_tokens: 0,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use {
        super::{
            super::shared::{is_tool_role_value, tool_result_call_id},
            *,
        },
        moltis_config::CompactionMode,
        serde_json::json,
    };

    /// Context window small enough to force the tail budget to consume
    /// only a handful of messages in tests. With default
    /// `threshold_percent=0.95` and `tail_budget_ratio=0.20`, 150 tokens
    /// of context means a tail budget of `150 × 0.95 × 0.20 ≈ 29`
    /// tokens — roughly two small messages after the 10-token metadata
    /// overhead per message, which is exactly what the happy-path
    /// tests assert.
    const TEST_CONTEXT_WINDOW_TINY: u32 = 150;

    fn mk_user(text: &str) -> Value {
        json!({"role": "user", "content": text})
    }

    fn mk_assistant(text: &str) -> Value {
        json!({"role": "assistant", "content": text})
    }

    fn mk_assistant_with_tool_call(text: &str, call_id: &str, tool: &str) -> Value {
        json!({
            "role": "assistant",
            "content": text,
            "tool_calls": [{
                "id": call_id,
                "type": "function",
                "function": { "name": tool, "arguments": "{}" }
            }]
        })
    }

    fn mk_tool_result(call_id: &str, content: &str) -> Value {
        json!({
            "role": "tool",
            "tool_call_id": call_id,
            "content": content,
        })
    }

    fn sample_short_history() -> Vec<Value> {
        vec![
            mk_user("hello"),
            mk_assistant("hi there"),
            mk_user("what is 2+2"),
            mk_assistant("4"),
        ]
    }

    #[tokio::test]
    async fn recency_preserving_tiny_history_returns_too_small_error() {
        // A 4-message sample with no tool-result content is below the
        // 3+20 default floor and has nothing to prune — expect a clear
        // error pointing the user at a different mode.
        let history = sample_short_history();
        let config = CompactionConfig {
            mode: CompactionMode::RecencyPreserving,
            ..Default::default()
        };
        let err = super::super::run_compaction(&history, &config, None)
            .await
            .unwrap_err();
        match err {
            CompactionRunError::TooSmallToCompact { messages, .. } => {
                assert_eq!(messages, history.len());
            },
            other => panic!("expected TooSmallToCompact, got {other:?}"),
        }
    }

    #[test]
    fn recency_preserving_splices_marker_between_head_and_tail() {
        // 10 messages: head=2, tail=2 → middle=6 collapsed into 1 marker.
        let mut history = Vec::new();
        for i in 0..5 {
            history.push(mk_user(&format!("user {i}")));
            history.push(mk_assistant(&format!("assistant {i}")));
        }

        let config = CompactionConfig {
            mode: CompactionMode::RecencyPreserving,
            protect_head: 2,
            protect_tail_min: 2,
            tool_prune_char_threshold: 20,
            ..Default::default()
        };
        let outcome = run(&history, &config, TEST_CONTEXT_WINDOW_TINY).unwrap();
        assert_eq!(outcome.effective_mode, CompactionMode::RecencyPreserving);
        assert_eq!(outcome.input_tokens, 0);
        assert_eq!(outcome.output_tokens, 0);
        let result = outcome.history;

        // 2 head + 1 marker + 2 tail = 5 messages.
        assert_eq!(result.len(), 5, "result: {result:#?}");

        // Head is verbatim.
        assert_eq!(
            result[0].get("content").and_then(Value::as_str),
            Some("user 0")
        );
        assert_eq!(
            result[1].get("content").and_then(Value::as_str),
            Some("assistant 0")
        );

        // Marker has the right shape.
        let marker = result[2]
            .get("content")
            .and_then(Value::as_str)
            .expect("marker content");
        assert!(
            marker.starts_with("[Conversation Compacted]"),
            "got: {marker}"
        );
        assert!(marker.contains("6 earlier messages"), "got: {marker}");

        // Tail is verbatim — the LAST two source messages.
        assert_eq!(
            result[3].get("content").and_then(Value::as_str),
            Some("user 4")
        );
        assert_eq!(
            result[4].get("content").and_then(Value::as_str),
            Some("assistant 4")
        );
    }

    #[tokio::test]
    async fn recency_preserving_prunes_oversized_tool_results_in_head_only_session() {
        // Only 3 messages total — head+tail cover everything, but the
        // middle tool output is bulky enough to be worth pruning in place.
        let oversized = "x".repeat(500);
        let history = vec![
            mk_user("first user"),
            mk_assistant_with_tool_call("calling tool", "call_1", "read_file"),
            mk_tool_result("call_1", &oversized),
        ];

        let config = CompactionConfig {
            mode: CompactionMode::RecencyPreserving,
            protect_head: 3,
            protect_tail_min: 3,
            tool_prune_char_threshold: 20,
            ..Default::default()
        };
        let outcome = super::super::run_compaction(&history, &config, None)
            .await
            .unwrap();
        let result = outcome.history;

        // Same number of messages, but the tool content is now a placeholder.
        assert_eq!(result.len(), 3);
        let tool_content = result[2]
            .get("content")
            .and_then(Value::as_str)
            .expect("tool content");
        assert_eq!(tool_content, super::super::shared::PRUNED_TOOL_PLACEHOLDER);
    }

    #[test]
    fn recency_preserving_drops_orphaned_tool_results() {
        // Head keeps first 2 messages. A later tool result references a
        // parent that lives in the dropped middle — it must be removed so
        // strict providers don't reject the retry.
        let history = vec![
            mk_user("u0"),
            mk_assistant("a0"),
            mk_user("u1"),
            mk_assistant_with_tool_call("mid-a", "orphan_call", "exec"),
            mk_tool_result("orphan_call", "mid tool out"),
            mk_user("u2"),
            mk_assistant("mid-a2"),
            mk_tool_result("orphan_call", "late tail out"),
            mk_user("u3"),
        ];

        let config = CompactionConfig {
            mode: CompactionMode::RecencyPreserving,
            protect_head: 2,
            protect_tail_min: 2,
            tool_prune_char_threshold: 10_000,
            ..Default::default()
        };
        let result = run(&history, &config, TEST_CONTEXT_WINDOW_TINY)
            .unwrap()
            .history;

        for msg in &result {
            if is_tool_role_value(msg) {
                assert_ne!(
                    tool_result_call_id(msg),
                    Some("orphan_call"),
                    "orphaned tool result should be dropped, got: {msg:#?}"
                );
            }
        }
    }

    #[test]
    fn recency_preserving_stubs_missing_tool_results_for_surviving_assistant_calls() {
        // Assistant with tool_call survives in the head; its tool result is
        // in the middle and gets dropped. Sanitizer must insert a stub so
        // the call_id is satisfied.
        let history = vec![
            mk_user("start"),
            mk_assistant_with_tool_call("running", "head_call", "exec"),
            mk_tool_result("head_call", "result body"),
            mk_user("filler 1"),
            mk_assistant("filler reply 1"),
            mk_user("filler 2"),
            mk_assistant("filler reply 2"),
            mk_user("tail user"),
            mk_assistant("tail assistant"),
        ];

        let config = CompactionConfig {
            mode: CompactionMode::RecencyPreserving,
            protect_head: 2,
            protect_tail_min: 2,
            tool_prune_char_threshold: 10_000,
            ..Default::default()
        };
        let result = run(&history, &config, TEST_CONTEXT_WINDOW_TINY)
            .unwrap()
            .history;

        let stub = result
            .iter()
            .find(|m| is_tool_role_value(m) && tool_result_call_id(m) == Some("head_call"));
        assert!(
            stub.is_some(),
            "expected a stub tool result for head_call, got: {result:#?}"
        );
    }
}
