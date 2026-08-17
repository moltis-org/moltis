//! Correlating delivered replies with the traces that produced them.
//!
//! Feedback attribution is recorded at send time because nothing else in the
//! system remembers which agent run wrote a given chat message, and by the
//! time someone reacts the turn is long finished.

use std::sync::Arc;

use crate::runtime::ChatRuntime;

/// Link a web-delivered assistant message to the trace that produced it.
///
/// The web is treated as one more channel so a thumb in the browser resolves
/// through the same correlation table as a Telegram reaction. The run id is
/// the message identity: unlike a message index it does not shift when older
/// history is compacted away.
pub(crate) async fn record_web_reply_trace(
    state: &Arc<dyn ChatRuntime>,
    session_key: &str,
    run_id: &str,
) {
    let Some(feedback) = state.feedback() else {
        return;
    };
    let Some(trace_id) = moltis_observability::recent_trace(run_id) else {
        return;
    };
    feedback
        .record_reply(
            moltis_channels::trace_link::WEB_CHANNEL,
            moltis_channels::trace_link::WEB_CHANNEL,
            session_key,
            std::slice::from_ref(&run_id.to_string()),
            &trace_id,
            Some(session_key),
        )
        .await;
}

/// Link the messages a reply produced to the trace that generated it.
///
/// Keyed on [`ChannelReplyTarget::chat_id`] rather than the outbound address:
/// for a Telegram forum topic the outbound address carries a `:thread` suffix,
/// but a `message_reaction` update reports only the bare chat id, so a link
/// stored under the suffixed form is one no reaction could ever find.
///
/// Best-effort by design: a missing link costs feedback attribution for one
/// reply, and the user already has the message.
///
/// [`ChannelReplyTarget::chat_id`]: moltis_channels::ChannelReplyTarget::chat_id
pub(crate) async fn record_reply_trace(
    feedback: Option<&moltis_channels::FeedbackService>,
    target: &moltis_channels::ChannelReplyTarget,
    message_ids: &[String],
    session_key: &str,
    trace_correlation_key: &str,
) {
    if message_ids.is_empty() {
        return;
    }
    let Some(feedback) = feedback else {
        return;
    };
    let Some(trace_id) = moltis_observability::recent_trace(trace_correlation_key) else {
        return;
    };
    feedback
        .record_reply(
            target.channel_type.as_str(),
            &target.account_id,
            &target.chat_id,
            message_ids,
            &trace_id,
            Some(session_key),
        )
        .await;
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::Mutex;

    use {
        async_trait::async_trait,
        moltis_channels::trace_link::{TraceLink, TraceLinkStore},
        moltis_config::FeedbackSettings,
        moltis_observability::TraceId,
    };

    use super::*;

    #[derive(Default)]
    struct RecordingLinks {
        links: Mutex<Vec<TraceLink>>,
    }

    #[async_trait]
    impl TraceLinkStore for RecordingLinks {
        async fn link(&self, link: TraceLink) -> moltis_channels::Result<()> {
            self.links.lock().unwrap().push(link);
            Ok(())
        }

        async fn lookup(
            &self,
            _channel_type: &str,
            _account_id: &str,
            _chat_id: &str,
            _message_id: &str,
        ) -> moltis_channels::Result<Option<TraceLink>> {
            Ok(None)
        }

        async fn prune(&self, _cutoff: i64) -> moltis_channels::Result<u64> {
            Ok(0)
        }
    }

    fn telegram_topic_target() -> moltis_channels::ChannelReplyTarget {
        moltis_channels::ChannelReplyTarget {
            ack_message_id: None,
            channel_type: moltis_channels::ChannelType::Telegram,
            account_id: "bot1".into(),
            chat_id: "-1001234".into(),
            message_id: Some("77".into()),
            thread_id: Some("42".into()),
        }
    }

    /// A reply into a Telegram forum topic goes out to `chat:thread`, but the
    /// `message_reaction` update that follows reports only `chat`. Storing the
    /// composite address would make every topic reaction unresolvable.
    #[tokio::test]
    async fn topic_replies_are_keyed_on_the_bare_chat_id() {
        let session_key = "topic-key-test-session";
        let links = Arc::new(RecordingLinks::default());
        let feedback = moltis_channels::FeedbackService::default();
        feedback.apply(
            Arc::clone(&links) as Arc<dyn TraceLinkStore>,
            &FeedbackSettings::default(),
            None,
        );
        moltis_observability::remember_trace(session_key, &TraceId("trace-abc".into()));

        let target = telegram_topic_target();
        assert_eq!(target.outbound_to().as_ref(), "-1001234:42");

        record_reply_trace(
            Some(&feedback),
            &target,
            &["901".to_string(), "902".to_string()],
            session_key,
            session_key,
        )
        .await;

        let recorded = links.links.lock().unwrap();
        assert_eq!(recorded.len(), 2);
        for link in recorded.iter() {
            assert_eq!(link.chat_id, "-1001234");
            assert_eq!(link.account_id, "bot1");
            assert_eq!(link.channel_type, "telegram");
            assert_eq!(link.trace_id, "trace-abc");
        }
        assert_eq!(recorded[0].message_id, "901");
        assert_eq!(recorded[1].message_id, "902");
    }

    #[tokio::test]
    async fn nothing_is_recorded_without_a_feedback_service() {
        record_reply_trace(
            None,
            &telegram_topic_target(),
            &["1".to_string()],
            "none",
            "none",
        )
        .await;
    }
}
