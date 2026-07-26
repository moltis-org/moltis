//! Web push notifications for chat responses.
//!
//! Builds the title, body, and click-through URL for the notification sent to
//! subscribed PWA clients when an agent finishes replying.

#[cfg(feature = "push-notifications")]
use {crate::runtime::ChatRuntime, std::sync::Arc, tracing::debug};

use crate::types::truncate_at_char_boundary;

/// Build the SPA URL for a push notification click-through.
///
/// Must match the frontend `sessionPath()` in `router.ts`:
/// `/chats/${key.replace(/:/g, "/")}`.
#[cfg(any(feature = "push-notifications", test))]
pub(crate) fn push_notification_url(session_key: &str) -> String {
    format!("/chats/{}", session_key.replace(':', "/"))
}

/// Build the notification title for a session.
///
/// A generic "Message received" is useless once more than one chat is running,
/// so name the session when we can and fall back to the app name otherwise.
#[cfg(any(feature = "push-notifications", test))]
pub(crate) fn push_notification_title(session_label: Option<&str>) -> String {
    match session_label.map(str::trim).filter(|l| !l.is_empty()) {
        Some(label) => truncate_at_char_boundary(label, 60).to_string(),
        None => "moltis".to_string(),
    }
}

/// Strip markdown noise that reads badly in a notification shade.
///
/// Notification bodies are plain text: fences, heading markers, and emphasis
/// characters all render literally, so a code-heavy reply becomes unreadable.
#[cfg(any(feature = "push-notifications", test))]
pub(crate) fn summarize_for_notification(text: &str, max_chars: usize) -> String {
    let cleaned: String = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("```"))
        .map(|line| line.trim_start_matches('#').trim_start_matches('>').trim())
        .collect::<Vec<_>>()
        .join(" ");
    let cleaned = cleaned.replace(['*', '_', '`'], "");
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");

    if collapsed.chars().count() > max_chars {
        format!(
            "{}…",
            truncate_at_char_boundary(&collapsed, max_chars).trim_end()
        )
    } else {
        collapsed
    }
}

#[cfg(feature = "push-notifications")]
pub(crate) async fn send_chat_push_notification(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    text: &str,
) {
    let summary = summarize_for_notification(text, 140);
    if summary.is_empty() {
        debug!(session_key, "skipping push notification: empty summary");
        return;
    }

    let session_label = state.session_label(session_key).await;
    let title = push_notification_title(session_label.as_deref());
    let url = push_notification_url(session_key);

    match state
        .send_push_notification(&title, &summary, Some(&url), Some(session_key))
        .await
    {
        Ok(sent) => {
            tracing::info!(sent, "push notification sent");
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
    fn notification_title_uses_the_session_label() {
        assert_eq!(push_notification_title(Some("Deploy plan")), "Deploy plan");
    }

    #[test]
    fn notification_title_falls_back_when_label_is_missing_or_blank() {
        assert_eq!(push_notification_title(None), "moltis");
        assert_eq!(push_notification_title(Some("   ")), "moltis");
    }

    #[test]
    fn notification_title_is_truncated_for_the_notification_shade() {
        let long = "a".repeat(200);
        let title = push_notification_title(Some(&long));
        assert!(title.chars().count() <= 60);
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
        assert!(summary.chars().count() <= 41);
    }

    #[test]
    fn notification_summary_is_empty_for_whitespace_only_text() {
        assert_eq!(summarize_for_notification("\n\n   \n", 140), "");
    }

    #[test]
    fn notification_summary_does_not_split_multibyte_characters() {
        let text = "é".repeat(100);
        let summary = summarize_for_notification(&text, 10);
        assert!(summary.chars().count() <= 11, "got: {summary}");
    }
}
