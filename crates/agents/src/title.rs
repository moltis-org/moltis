//! Session title generation via a lightweight LLM call.
//!
//! Produces a short descriptive title (3-8 words) from conversation context.
//! No tools, no history persistence — just a single completion call.

use std::sync::Arc;

use {
    anyhow::Result,
    tracing::{debug, info, warn},
};

#[cfg(feature = "metrics")]
use std::time::Instant;

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, histogram, labels, memory as mem_metrics};

use crate::model::{ChatMessage, LlmProvider, UserContent};

const TITLE_SYSTEM_PROMPT: &str = "\
Generate a short, descriptive title (3-8 words) for the following conversation. \
The title should capture the main topic or intent. \
Output ONLY the title text — no quotes, no punctuation at the end, no explanation.";

/// Maximum characters from each message included in the title-generation prompt.
const MAX_CHARS_PER_MESSAGE: usize = 500;

/// Maximum number of messages to include (first user + assistant exchange is enough).
const MAX_MESSAGES: usize = 6;

/// Generate a short title from conversation history.
///
/// Returns `Ok(title)` on success or an error if the LLM call fails.
/// The caller is responsible for persisting the title.
#[tracing::instrument(skip(provider, conversation), fields(messages = conversation.len()))]
pub async fn generate_title(
    provider: Arc<dyn LlmProvider>,
    conversation: &[ChatMessage],
) -> Result<String> {
    if conversation.is_empty() {
        anyhow::bail!("cannot generate title from empty conversation");
    }

    // Build a compact summary of the conversation for the title prompt.
    let mut context = String::new();
    let mut included = 0;
    for msg in conversation {
        if included >= MAX_MESSAGES {
            break;
        }
        let (role, content) = match msg {
            ChatMessage::System { .. } => continue,
            ChatMessage::User {
                content: UserContent::Text(t),
                ..
            } => ("user", t.as_str()),
            ChatMessage::User {
                content: UserContent::Multimodal(_),
                ..
            } => ("user", "[multimodal content]"),
            ChatMessage::Assistant { content, .. } => {
                ("assistant", content.as_deref().unwrap_or(""))
            },
            ChatMessage::Tool { .. } => continue,
        };
        let truncated = &content[..content.floor_char_boundary(MAX_CHARS_PER_MESSAGE)];
        context.push_str(&format!("{role}: {truncated}\n"));
        included += 1;
    }

    if context.is_empty() {
        anyhow::bail!("no user/assistant content to generate title from");
    }

    debug!(context_len = context.len(), "generating session title");

    #[cfg(feature = "metrics")]
    let start = Instant::now();

    let messages = vec![
        ChatMessage::system(TITLE_SYSTEM_PROMPT),
        ChatMessage::user(&context),
    ];
    let result = provider.complete(&messages, &[]).await;

    match result {
        Ok(response) => {
            #[cfg(feature = "metrics")]
            {
                let duration = start.elapsed().as_secs_f64();
                counter!(
                    mem_metrics::SILENT_TURNS_TOTAL,
                    labels::VARIANT => "title-generation",
                    labels::SUCCESS => "true"
                )
                .increment(1);
                histogram!(
                    mem_metrics::SILENT_TURN_DURATION_SECONDS,
                    labels::VARIANT => "title-generation"
                )
                .record(duration);
            }
            let raw = response.text.unwrap_or_default();
            let title = clean_title(&raw);
            if title.is_empty() {
                anyhow::bail!("LLM returned empty title");
            }
            info!(title = %title, "generated session title");
            Ok(title)
        },
        Err(e) => {
            #[cfg(feature = "metrics")]
            {
                let duration = start.elapsed().as_secs_f64();
                counter!(
                    mem_metrics::SILENT_TURNS_TOTAL,
                    labels::VARIANT => "title-generation",
                    labels::SUCCESS => "false"
                )
                .increment(1);
                histogram!(
                    mem_metrics::SILENT_TURN_DURATION_SECONDS,
                    labels::VARIANT => "title-generation"
                )
                .record(duration);
            }
            warn!(error = %e, "title generation LLM call failed");
            Err(e)
        },
    }
}

/// Strip quotes, trailing punctuation, and whitespace from LLM output.
fn clean_title(raw: &str) -> String {
    let trimmed = raw.trim();
    // Strip surrounding quotes (single or double).
    let unquoted = trimmed
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| {
            trimmed
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
        })
        .unwrap_or(trimmed);
    // Strip trailing period or colon.
    let cleaned = unquoted.trim_end_matches('.').trim_end_matches(':').trim();
    // Take only the first line if the model produced multiple.
    cleaned.lines().next().unwrap_or(cleaned).trim().to_string()
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_title_strips_quotes() {
        assert_eq!(clean_title("\"Fix auth module\""), "Fix auth module");
        assert_eq!(clean_title("'Fix auth module'"), "Fix auth module");
    }

    #[test]
    fn clean_title_strips_trailing_punctuation() {
        assert_eq!(clean_title("Fix auth module."), "Fix auth module");
        assert_eq!(clean_title("Fix auth module:"), "Fix auth module");
    }

    #[test]
    fn clean_title_takes_first_line() {
        assert_eq!(
            clean_title("Fix auth module\nThis is extra"),
            "Fix auth module"
        );
    }

    #[test]
    fn clean_title_handles_whitespace() {
        assert_eq!(clean_title("  Fix auth module  "), "Fix auth module");
    }

    #[test]
    fn clean_title_handles_empty() {
        assert_eq!(clean_title(""), "");
        assert_eq!(clean_title("  "), "");
    }

    #[test]
    fn clean_title_quoted_with_trailing_period() {
        assert_eq!(clean_title("\"Fix auth module.\""), "Fix auth module");
    }
}
