//! Channel notices for session compaction.
//!
//! Split from the main channel delivery module to keep both inside the
//! file-size limit; the compaction notice is a self-contained concern.

use {std::sync::Arc, tracing::warn};

use crate::{compaction_run, runtime::ChatRuntime};

fn format_channel_compaction_notice(
    outcome: &compaction_run::CompactionOutcome,
    include_settings_hint: bool,
) -> String {
    let mode_label = match outcome.effective_mode {
        moltis_config::CompactionMode::Deterministic => "Deterministic",
        moltis_config::CompactionMode::RecencyPreserving => "Recency preserving",
        moltis_config::CompactionMode::Structured => "Structured",
        moltis_config::CompactionMode::LlmReplace => "LLM replace",
    };
    let total = outcome.total_tokens();
    let token_line = if total == 0 {
        // Any strategy that made no LLM calls ends up here: Deterministic,
        // RecencyPreserving, or a Structured run that fell back to
        // recency_preserving before the LLM call landed. Report the
        // actual effective mode so users don't see "deterministic
        // strategy" when they picked recency_preserving.
        format!(
            "No LLM tokens used ({} strategy)",
            mode_label.to_lowercase()
        )
    } else {
        format!(
            "Used {total} tokens ({input} in + {output} out)",
            total = total,
            input = outcome.input_tokens,
            output = outcome.output_tokens,
        )
    };
    let body = format!(
        "🧹 Conversation compacted\n\
         Mode: {mode_label}\n\
         {token_line}",
    );
    if include_settings_hint {
        format!("{body}\n{hint}", hint = compaction_run::SETTINGS_HINT)
    } else {
        body
    }
}

/// Send a silent "session compacted" notice to pending channel targets
/// without draining them.
///
/// Mirrors [`send_retry_status_to_channels`]: the targets are *peeked*,
/// not drained, so the in-flight agent run can still deliver its final
/// reply to them afterward. Uses `send_text_silent` so the channel
/// integration doesn't count it toward user-visible interactive replies
/// (no TTS, no delivery receipts beyond the channel's own).
pub(crate) async fn notify_channels_of_compaction(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    outcome: &compaction_run::CompactionOutcome,
    include_settings_hint: bool,
) {
    let targets = state.peek_channel_replies(session_key).await;
    if targets.is_empty() {
        return;
    }

    let Some(outbound) = state.channel_outbound() else {
        return;
    };

    let message = format_channel_compaction_notice(outcome, include_settings_hint);
    let mut tasks = Vec::with_capacity(targets.len());
    for target in targets {
        let outbound = Arc::clone(&outbound);
        let message = message.clone();
        let to = target.outbound_to().into_owned();
        tasks.push(tokio::spawn(async move {
            let reply_to = target.message_id.as_deref();
            if let Err(e) = outbound
                .send_text_silent(&target.account_id, &to, &message, reply_to)
                .await
            {
                warn!(
                    account_id = target.account_id,
                    chat_id = target.chat_id,
                    thread_id = target.thread_id.as_deref().unwrap_or("-"),
                    "failed to send compaction notice to channel: {e}"
                );
            }
        }));
    }

    for task in tasks {
        if let Err(e) = task.await {
            warn!(error = %e, "channel compaction notice task join failed");
        }
    }
}
