//! `CompactionMode::LlmReplace` — pre-PR-#653 streaming-summary replace-all.
//!
//! Streams a plain-text summary from the session provider, then replaces
//! the entire history with a single user message containing that summary.
//! Preserved as the minimal-code LLM option for users who explicitly want
//! the pre-PR behaviour or need maximum token reduction.

use {
    moltis_agents::model::{ChatMessage, LlmProvider, StreamEvent, Usage, values_to_chat_messages},
    moltis_config::{CompactionConfig, CompactionMode},
    serde_json::Value,
    tokio_stream::StreamExt,
    tracing::info,
};

use super::{CompactionOutcome, CompactionRunError, shared::build_summary_message};

/// Run the streaming-summary replace-all strategy against `history`.
///
/// Builds a system / history / directive prompt using structured
/// `ChatMessage` objects so role boundaries stay intact (prevents
/// prompt injection via role prefixes in user content), streams the
/// summary, and wraps it in a single user message. The returned
/// outcome surfaces the provider's Usage report so the compaction
/// broadcast can show how many tokens were spent.
pub(super) async fn run(
    history: &[Value],
    config: &CompactionConfig,
    provider: &dyn LlmProvider,
) -> Result<CompactionOutcome, CompactionRunError> {
    let mut summary_messages = vec![ChatMessage::system(
        "You are a conversation summarizer. The messages that follow are a \
         conversation you must summarize. Preserve all key facts, decisions, \
         and context. After the conversation, you will receive a final \
         instruction.",
    )];
    summary_messages.extend(values_to_chat_messages(history));
    summary_messages.push(ChatMessage::user(
        "Summarize the conversation above into a concise form. Output only \
         the summary, no preamble.",
    ));

    let mut stream = provider.stream(summary_messages);
    let mut summary = String::new();
    let mut usage = Usage::default();
    while let Some(event) = stream.next().await {
        match event {
            StreamEvent::Delta(delta) => summary.push_str(&delta),
            StreamEvent::Done(u) => {
                usage = u;
                break;
            },
            StreamEvent::Error(e) => {
                return Err(CompactionRunError::LlmFailed(e.to_string()));
            },
            // Tool events aren't expected on a summary stream; drop them.
            StreamEvent::ToolCallStart { .. }
            | StreamEvent::ToolCallArgumentsDelta { .. }
            | StreamEvent::ToolCallComplete { .. }
            // Provider raw payloads are debug metadata, not summary text.
            | StreamEvent::ProviderRaw(_)
            // Ignore provider reasoning blocks; summary body should only
            // include final answer text.
            | StreamEvent::ReasoningDelta(_) => {},
        }
    }

    // `config.summary_model` / `max_summary_tokens` aren't wired yet —
    // tracked by beads issue moltis-8me.
    let _ = config;

    if summary.is_empty() {
        return Err(CompactionRunError::EmptySummary);
    }

    info!(
        messages = history.len(),
        chars = summary.len(),
        input_tokens = usage.input_tokens,
        output_tokens = usage.output_tokens,
        "chat.compact: llm_replace summary"
    );

    Ok(CompactionOutcome {
        history: vec![build_summary_message(&summary)],
        effective_mode: CompactionMode::LlmReplace,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use {
        super::{super::test_support::StubProvider, *},
        moltis_config::CompactionMode,
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
    async fn llm_replace_mode_without_provider_returns_provider_required() {
        let history = sample_history();
        let config = CompactionConfig {
            mode: CompactionMode::LlmReplace,
            ..Default::default()
        };
        let err = super::super::run_compaction(&history, &config, None)
            .await
            .unwrap_err();
        match err {
            CompactionRunError::ProviderRequired { mode } => assert_eq!(mode, "llm_replace"),
            other => panic!("expected ProviderRequired, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn llm_replace_mode_with_stub_provider_returns_single_message() {
        let history = sample_history();
        let config = CompactionConfig {
            mode: CompactionMode::LlmReplace,
            ..Default::default()
        };
        let provider = StubProvider::new_ok("stubbed summary body");
        let outcome = super::super::run_compaction(&history, &config, Some(&provider))
            .await
            .expect("llm_replace succeeds with stub provider");
        assert_eq!(outcome.effective_mode, CompactionMode::LlmReplace);
        assert_eq!(outcome.history.len(), 1);
        let text = outcome.history[0]
            .get("content")
            .and_then(Value::as_str)
            .expect("summary content");
        assert!(text.contains("stubbed summary body"), "got: {text}");
    }
}
