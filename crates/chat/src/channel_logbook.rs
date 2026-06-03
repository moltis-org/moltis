use std::sync::Arc;

use {
    moltis_channels::{
        ActivityLogMode, ChannelReplyTarget, ChannelStatusLogEntry, ChannelType,
        plugin::ChannelOutbound,
    },
    tracing::warn,
};

/// Format buffered status log entries into a Telegram expandable blockquote HTML.
/// Returns an empty string if there are no entries after filtering.
pub(crate) fn format_logbook_html_for_mode(
    entries: &[ChannelStatusLogEntry],
    mode: ActivityLogMode,
) -> String {
    let mut html = String::new();
    for entry in entries.iter().filter(|entry| mode.includes(entry.kind)) {
        if html.is_empty() {
            html.push_str("<blockquote expandable>\n\u{1f4cb} <b>Activity log</b>\n");
        }
        let escaped = entry
            .message
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        html.push_str(&format!("\u{2022} {escaped}\n"));
    }
    if !html.is_empty() {
        html.push_str("</blockquote>");
    }
    html
}

fn format_logbook_plain_text_for_mode(
    entries: &[ChannelStatusLogEntry],
    mode: ActivityLogMode,
) -> String {
    let mut text = String::new();
    for entry in entries.iter().filter(|entry| mode.includes(entry.kind)) {
        if text.is_empty() {
            text.push_str("Activity log\n");
        }
        text.push_str("- ");
        text.push_str(&entry.message);
        text.push('\n');
    }
    let _ = text.pop();
    text
}

fn supports_html_logbook_follow_up(channel_type: ChannelType) -> bool {
    matches!(
        channel_type,
        ChannelType::Telegram | ChannelType::Discord | ChannelType::Matrix
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ChannelLogbookFollowUp {
    Html(String),
    Text(String),
}

pub(crate) fn format_channel_logbook_follow_up(
    entries: &[ChannelStatusLogEntry],
    target: &ChannelReplyTarget,
) -> Option<ChannelLogbookFollowUp> {
    if supports_html_logbook_follow_up(target.channel_type) {
        let html = format_logbook_html_for_mode(entries, target.activity_log);
        if html.is_empty() {
            None
        } else {
            Some(ChannelLogbookFollowUp::Html(html))
        }
    } else {
        let text = format_logbook_plain_text_for_mode(entries, target.activity_log);
        if text.is_empty() {
            None
        } else {
            Some(ChannelLogbookFollowUp::Text(text))
        }
    }
}

pub(crate) async fn send_channel_logbook_follow_up(
    outbound: &dyn ChannelOutbound,
    target: &ChannelReplyTarget,
    to: &str,
    follow_up: ChannelLogbookFollowUp,
) -> moltis_channels::Result<()> {
    match follow_up {
        ChannelLogbookFollowUp::Html(html) => {
            outbound
                .send_html(&target.account_id, to, &html, None)
                .await
        },
        ChannelLogbookFollowUp::Text(text) => {
            outbound
                .send_text(&target.account_id, to, &text, None)
                .await
        },
    }
}

pub(crate) async fn send_channel_logbook_follow_up_to_targets(
    outbound: Arc<dyn ChannelOutbound>,
    targets: Vec<ChannelReplyTarget>,
    status_log: &[ChannelStatusLogEntry],
) {
    if targets.is_empty() || status_log.is_empty() {
        return;
    }

    let mut tasks = Vec::with_capacity(targets.len());
    for target in targets {
        let outbound = Arc::clone(&outbound);
        let Some(follow_up) = format_channel_logbook_follow_up(status_log, &target) else {
            continue;
        };
        let to = target.outbound_to().into_owned();
        tasks.push(tokio::spawn(async move {
            if let Err(e) =
                send_channel_logbook_follow_up(outbound.as_ref(), &target, &to, follow_up).await
            {
                warn!(
                    account_id = target.account_id,
                    chat_id = target.chat_id,
                    thread_id = target.thread_id.as_deref().unwrap_or("-"),
                    "failed to send logbook follow-up: {e}"
                );
            }
        }));
    }

    for task in tasks {
        if let Err(e) = task.await {
            warn!(error = %e, "channel logbook follow-up task join failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use {
        async_trait::async_trait,
        moltis_channels::{ChannelStatusLogKind, ChannelType},
        moltis_common::types::ReplyPayload,
        tokio::sync::Mutex,
    };

    #[derive(Default)]
    struct RecordingOutbound {
        sent_text: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ChannelOutbound for RecordingOutbound {
        async fn send_text(
            &self,
            _account_id: &str,
            _to: &str,
            text: &str,
            _reply_to: Option<&str>,
        ) -> moltis_channels::Result<()> {
            self.sent_text.lock().await.push(text.to_string());
            Ok(())
        }

        async fn send_media(
            &self,
            _account_id: &str,
            _to: &str,
            _payload: &ReplyPayload,
            _reply_to: Option<&str>,
        ) -> moltis_channels::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn non_html_channel_logbook_follow_up_uses_plain_text() {
        let outbound = Arc::new(RecordingOutbound::default());
        let targets = vec![ChannelReplyTarget {
            channel_type: ChannelType::Slack,
            account_id: "slack1".into(),
            chat_id: "C123".into(),
            message_id: None,
            thread_id: None,
            sender_id: None,
            activity_log: ActivityLogMode::All,
        }];
        let entries = [ChannelStatusLogEntry {
            kind: ChannelStatusLogKind::Info,
            message: "Running: `date`".into(),
        }];

        send_channel_logbook_follow_up_to_targets(outbound.clone(), targets, &entries).await;

        let sent = outbound.sent_text.lock().await;
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("Activity log"));
        assert!(sent[0].contains("Running: `date`"));
        assert!(!sent[0].contains("<blockquote"));
        assert!(!sent[0].contains("<b>"));
    }
}
