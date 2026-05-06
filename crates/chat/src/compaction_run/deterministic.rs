//! `CompactionMode::Deterministic` — zero-LLM extraction + replace-all.
//!
//! Wraps the structured-extraction helpers in [`crate::compaction`] and
//! produces a single `[Conversation Summary]` user message that replaces
//! the entire history. Current PR #653 default behaviour.

use {moltis_config::CompactionMode, serde_json::Value, tracing::info};

use super::{CompactionOutcome, CompactionRunError, shared::build_summary_message};

/// Run the deterministic extraction strategy against `history`.
///
/// Returns an outcome whose `history` is a single-element vec
/// containing the replacement summary message. Token counts are zero —
/// no LLM calls are made.
pub(super) fn run(history: &[Value]) -> Result<CompactionOutcome, CompactionRunError> {
    let merged = crate::compaction::compute_compaction_summary(history)
        .ok_or(CompactionRunError::EmptySummary)?;
    let summary = crate::compaction::compress_summary(&merged);
    if summary.is_empty() {
        return Err(CompactionRunError::EmptySummary);
    }

    info!(
        messages = history.len(),
        "chat.compact: deterministic summary"
    );

    Ok(CompactionOutcome {
        history: vec![build_summary_message(
            &crate::compaction::get_compact_continuation_message(&summary, false),
        )],
        effective_mode: CompactionMode::Deterministic,
        input_tokens: 0,
        output_tokens: 0,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use {
        super::*,
        moltis_config::{CompactionConfig, CompactionMode},
        serde_json::json,
    };

    fn sample_history() -> Vec<Value> {
        vec![
            json!({"role": "user", "content": "hello"}),
            json!({"role": "assistant", "content": "hi there"}),
            json!({"role": "user", "content": "what is 2+2"}),
            json!({"role": "assistant", "content": "4"}),
        ]
    }

    #[tokio::test]
    async fn deterministic_mode_returns_single_summary_message() {
        let history = sample_history();
        let config = CompactionConfig {
            mode: CompactionMode::Deterministic,
            ..Default::default()
        };
        let outcome = super::super::run_compaction(&history, &config, None)
            .await
            .expect("deterministic dispatch succeeds");
        assert_eq!(
            outcome.history.len(),
            1,
            "deterministic mode replaces history with one message"
        );
        assert_eq!(outcome.effective_mode, CompactionMode::Deterministic);
        assert_eq!(outcome.input_tokens, 0);
        assert_eq!(outcome.output_tokens, 0);
        let text = outcome.history[0]
            .get("content")
            .and_then(Value::as_str)
            .expect("summary has string content");
        assert!(
            text.starts_with("[Conversation Summary]\n\n"),
            "summary is wrapped in the expected preamble, got: {text}"
        );
    }
}
