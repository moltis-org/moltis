//! Web push notifications for chat responses.
//!
//! Builds the title, body, and click-through URL for the notification sent to
//! subscribed PWA clients when an agent finishes replying.

#[cfg(feature = "push-notifications")]
use {
    crate::runtime::ChatRuntime,
    std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    tracing::debug,
};

/// Return an order that remains monotonic when a session's history is reset.
///
/// Assistant message indices restart at zero after reset, so they cannot serve
/// as a durable high-water mark in the service worker. Wall-clock microseconds
/// preserve order across normal restarts, while the atomic increment handles
/// simultaneous completions and local clock granularity.
#[cfg(feature = "push-notifications")]
pub(crate) fn next_push_notification_order() -> u64 {
    static LAST_ORDER: AtomicU64 = AtomicU64::new(0);

    let wall_order = u64::try_from(chrono::Utc::now().timestamp_micros()).unwrap_or_default();
    let mut previous = LAST_ORDER.load(Ordering::Acquire);
    loop {
        let next = wall_order.max(previous.saturating_add(1));
        match LAST_ORDER.compare_exchange_weak(previous, next, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => return next,
            Err(current) => previous = current,
        }
    }
}

/// Build the SPA URL for a push notification click-through.
///
/// Must match the frontend `sessionPath()` in `router.ts`:
/// `/chats/${key.replace(/:/g, "/")}`.
#[cfg(any(feature = "push-notifications", test))]
pub(crate) fn push_notification_url(session_key: &str) -> String {
    format!("/chats/{}", session_key.replace(':', "/"))
}

/// Use a generic title so private session labels never appear on a lock screen.
#[cfg(any(feature = "push-notifications", test))]
pub(crate) fn push_notification_title() -> &'static str {
    "moltis"
}

#[cfg(any(feature = "push-notifications", test))]
fn append_html_text(output: &mut String, html: &str) {
    if let Ok(text) = html2text::config::with_decorator(html2text::render::TrivialDecorator::new())
        .string_from_read(html.as_bytes(), 10_000)
    {
        output.push_str(&text);
    }
}

/// Strip markdown noise that reads badly in a notification shade.
///
/// Notification bodies are plain text: fences, heading markers, and emphasis
/// characters all render literally, so a code-heavy reply becomes unreadable.
#[cfg(any(feature = "push-notifications", test))]
pub(crate) fn summarize_for_notification(text: &str, max_chars: usize) -> String {
    use pulldown_cmark::{Event, Parser, TagEnd};

    let mut plain = String::new();
    for event in Parser::new(text) {
        match event {
            Event::Text(value)
            | Event::Code(value)
            | Event::InlineMath(value)
            | Event::DisplayMath(value)
            | Event::FootnoteReference(value) => plain.push_str(&value),
            Event::SoftBreak | Event::HardBreak | Event::Rule => plain.push(' '),
            Event::TaskListMarker(checked) => {
                plain.push_str(if checked {
                    "[x] "
                } else {
                    "[ ] "
                });
            },
            Event::End(
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::BlockQuote(_)
                | TagEnd::CodeBlock
                | TagEnd::Item
                | TagEnd::TableCell
                | TagEnd::TableRow
                | TagEnd::DefinitionListTitle
                | TagEnd::DefinitionListDefinition,
            ) => plain.push(' '),
            Event::Html(value) | Event::InlineHtml(value) => {
                append_html_text(&mut plain, &value);
            },
            Event::Start(_) | Event::End(_) => {},
        }
    }
    let collapsed = plain.split_whitespace().collect::<Vec<_>>().join(" ");

    if collapsed.chars().count() > max_chars {
        if max_chars == 0 {
            String::new()
        } else {
            let truncated: String = collapsed.chars().take(max_chars - 1).collect();
            format!("{}…", truncated.trim_end())
        }
    } else {
        collapsed
    }
}

#[cfg(feature = "push-notifications")]
pub(crate) async fn send_chat_push_notification(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    text: &str,
    order: u64,
) {
    let summary = summarize_for_notification(text, 140);
    if summary.is_empty() {
        debug!(session_key, "skipping push notification: empty summary");
        return;
    }

    let url = push_notification_url(session_key);

    match state
        .send_ordered_push_notification(
            push_notification_title(),
            &summary,
            Some(&url),
            Some(session_key),
            order,
        )
        .await
    {
        Ok(0) => {
            debug!("push notification had no eligible delivery targets");
        },
        Ok(sent) => {
            tracing::info!(sent, "push notification accepted by push services");
        },
        Err(e) => {
            tracing::warn!("failed to send push notification: {e}");
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_notification_url_uses_chats_prefix_and_replaces_colons() {
        // Must match frontend sessionPath(): `/chats/${key.replace(/:/g, "/")}`
        assert_eq!(push_notification_url("session:42"), "/chats/session/42");
    }

    #[test]
    fn push_notification_url_handles_nested_session_keys() {
        assert_eq!(
            push_notification_url("telegram:bot123:chat456"),
            "/chats/telegram/bot123/chat456"
        );
    }

    #[test]
    fn push_notification_url_handles_key_without_colons() {
        assert_eq!(push_notification_url("main"), "/chats/main");
    }

    #[test]
    fn notification_title_is_generic_to_avoid_label_leaks() {
        assert_eq!(push_notification_title(), "moltis");
    }

    #[test]
    fn notification_summary_strips_markdown_syntax() {
        let text = "## Heading\n\nSome **bold** and `code` text.";
        assert_eq!(
            summarize_for_notification(text, 140),
            "Heading Some bold and code text."
        );
    }

    #[test]
    fn notification_summary_drops_code_fences() {
        let text = "Here you go:\n```rust\nfn main() {}\n```";
        let summary = summarize_for_notification(text, 140);
        assert!(!summary.contains("```"), "got: {summary}");
        assert!(summary.starts_with("Here you go:"), "got: {summary}");
    }

    #[test]
    fn notification_summary_truncates_with_an_ellipsis() {
        let text = "word ".repeat(100);
        let summary = summarize_for_notification(&text, 40);
        assert!(summary.ends_with('…'), "got: {summary}");
        assert!(summary.chars().count() <= 40);
    }

    #[test]
    fn notification_summary_is_empty_for_whitespace_only_text() {
        assert_eq!(summarize_for_notification("\n\n   \n", 140), "");
    }

    #[test]
    fn notification_summary_does_not_split_multibyte_characters() {
        let text = "é".repeat(100);
        let summary = summarize_for_notification(&text, 10);
        assert_eq!(summary.chars().count(), 10, "got: {summary}");
        assert_eq!(summary, format!("{}…", "é".repeat(9)));
    }

    #[test]
    fn notification_summary_preserves_non_markdown_punctuation_and_identifiers() {
        let text = "Use foo_bar, value*ptr, 2 * 3, and an unmatched `backtick.";
        assert_eq!(summarize_for_notification(text, 140), text);
    }

    #[test]
    fn notification_summary_strips_only_actual_inline_markdown() {
        let text = "**bold** _emphasis_ and `code_name`, but foo_bar stays.";
        assert_eq!(
            summarize_for_notification(text, 140),
            "bold emphasis and code_name, but foo_bar stays."
        );
    }

    #[test]
    fn notification_summary_preserves_visible_raw_html_text() {
        let text = "<div>Visible &amp; safe<br>next line</div>";
        assert_eq!(
            summarize_for_notification(text, 140),
            "Visible & safe next line"
        );
    }

    #[test]
    fn zero_character_limit_returns_an_empty_summary() {
        assert_eq!(summarize_for_notification("message", 0), "");
    }

    #[cfg(feature = "push-notifications")]
    #[test]
    fn notification_order_is_monotonic_independent_of_history_indices() {
        let before_reset = next_push_notification_order();
        let after_reset = next_push_notification_order();
        assert!(after_reset > before_reset);
    }
}
