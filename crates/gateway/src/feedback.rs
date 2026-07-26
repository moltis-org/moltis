//! Reaction feedback: recording which trace a reply came from, and turning a
//! later reaction on that reply into a score.
//!
//! The two halves are deliberately in one place because they share the
//! correlation key. The send side writes (channel, account, chat, message) →
//! trace; the reaction side reads it back.

use std::sync::Arc;

use {
    moltis_channels::{
        ChannelType,
        trace_link::{TraceLink, TraceLinkStore},
    },
    moltis_config::FeedbackSettings,
    moltis_observability::{
        FeedbackSignal, FeedbackVocabulary, TraceId, exporters::langfuse::LangfuseClient,
        feedback_score, feedback_score_id,
    },
    tracing::{debug, warn},
};

/// What happened to an inbound reaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedbackOutcome {
    /// A score was recorded for the trace.
    Recorded(FeedbackSignal),
    /// A previously recorded score was withdrawn.
    Retracted,
    /// The reaction carried no feedback signal.
    NotFeedback,
    /// The reply is no longer attributable to a trace.
    UnknownMessage,
    /// Feedback collection is switched off.
    Disabled,
}

/// Records reply/trace links and converts reactions into scores.
pub struct FeedbackService {
    links: Arc<dyn TraceLinkStore>,
    vocabulary: FeedbackVocabulary,
    enabled: bool,
    environment: Option<String>,
}

impl FeedbackService {
    /// Build a service from the instrumentation settings.
    #[must_use]
    pub fn new(
        links: Arc<dyn TraceLinkStore>,
        settings: &FeedbackSettings,
        environment: Option<String>,
    ) -> Self {
        Self {
            links,
            vocabulary: FeedbackVocabulary::from_config(&settings.positive, &settings.negative),
            enabled: settings.enabled,
            environment,
        }
    }

    /// Whether feedback collection is on.
    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    /// Record that `message_ids` were produced by `trace_id`.
    ///
    /// Failures are logged, never propagated: losing attribution for a reply
    /// must not fail the reply itself, which the user has already received.
    pub async fn record_reply(
        &self,
        channel_type: &str,
        account_id: &str,
        chat_id: &str,
        message_ids: &[String],
        trace_id: &TraceId,
        session_key: Option<&str>,
    ) {
        if !self.enabled || message_ids.is_empty() {
            return;
        }
        let created_at = now_unix();
        for message_id in message_ids {
            let link = TraceLink {
                channel_type: channel_type.to_string(),
                account_id: account_id.to_string(),
                chat_id: chat_id.to_string(),
                message_id: message_id.clone(),
                trace_id: trace_id.0.clone(),
                session_key: session_key.map(str::to_string),
                created_at,
            };
            if let Err(error) = self.links.link(link).await {
                warn!(%error, channel_type, "failed to record trace link for reply");
            }
        }
    }

    /// Handle a reaction change on a channel message.
    ///
    /// `langfuse` is only needed to retract: adding a score goes through the
    /// instrumentation sink like any other event, but deletion has no sink
    /// representation and must call the API directly.
    pub async fn on_reaction(
        &self,
        channel_type: ChannelType,
        account_id: &str,
        chat_id: &str,
        message_id: &str,
        emoji: &str,
        user_id: &str,
        added: bool,
        langfuse: Option<&Arc<LangfuseClient>>,
    ) -> FeedbackOutcome {
        if !self.enabled {
            return FeedbackOutcome::Disabled;
        }

        // Classify before the database lookup: most reactions in a busy chat
        // are not feedback, and they should not cost a query each.
        let Some(signal) = self.vocabulary.classify(emoji) else {
            return FeedbackOutcome::NotFeedback;
        };

        let channel = channel_type.as_str();
        let link = match self
            .links
            .lookup(channel, account_id, chat_id, message_id)
            .await
        {
            Ok(Some(link)) => link,
            Ok(None) => {
                debug!(
                    channel,
                    message_id, "reaction on a message with no trace link"
                );
                return FeedbackOutcome::UnknownMessage;
            },
            Err(error) => {
                warn!(%error, channel, "failed to look up trace link for reaction");
                return FeedbackOutcome::UnknownMessage;
            },
        };

        let trace_id = TraceId(link.trace_id);
        // Namespaced so the same numeric user id on two channels is two people.
        let scoped_user = format!("{channel}:{user_id}");

        if added {
            let score = feedback_score(
                &trace_id,
                signal,
                Some(&scoped_user),
                Some(format!("{channel} reaction {emoji}")),
                self.environment.clone(),
            );
            moltis_observability::record(moltis_observability::Event::Score(Box::new(score)));
            debug!(channel, ?signal, "recorded reaction feedback");
            return FeedbackOutcome::Recorded(signal);
        }

        let score_id = feedback_score_id(&trace_id, Some(&scoped_user));
        let Some(client) = langfuse else {
            // Nothing to retract against; the score sink has no delete path.
            return FeedbackOutcome::Retracted;
        };
        if let Err(error) = client.delete_score(&score_id).await {
            warn!(%error, channel, "failed to retract reaction feedback");
        }
        FeedbackOutcome::Retracted
    }

    /// Drop links older than `retention_days`.
    pub async fn prune(&self, retention_days: u32) -> u64 {
        let cutoff = now_unix() - i64::from(retention_days) * 86_400;
        match self.links.prune(cutoff).await {
            Ok(removed) => removed,
            Err(error) => {
                warn!(%error, "failed to prune trace links");
                0
            },
        }
    }
}

fn now_unix() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {moltis_channels::Result as ChannelResult, std::sync::Mutex, tokio::sync::OnceCell};

    use super::*;

    #[derive(Default)]
    struct MemoryLinks {
        rows: Mutex<Vec<TraceLink>>,
    }

    #[async_trait::async_trait]
    impl TraceLinkStore for MemoryLinks {
        async fn link(&self, link: TraceLink) -> ChannelResult<()> {
            let mut rows = self.rows.lock().unwrap_or_else(|e| e.into_inner());
            rows.retain(|r| {
                !(r.channel_type == link.channel_type
                    && r.account_id == link.account_id
                    && r.chat_id == link.chat_id
                    && r.message_id == link.message_id)
            });
            rows.push(link);
            Ok(())
        }

        async fn lookup(
            &self,
            channel_type: &str,
            account_id: &str,
            chat_id: &str,
            message_id: &str,
        ) -> ChannelResult<Option<TraceLink>> {
            let rows = self.rows.lock().unwrap_or_else(|e| e.into_inner());
            Ok(rows
                .iter()
                .find(|r| {
                    r.channel_type == channel_type
                        && r.account_id == account_id
                        && r.chat_id == chat_id
                        && r.message_id == message_id
                })
                .cloned())
        }

        async fn prune(&self, cutoff: i64) -> ChannelResult<u64> {
            let mut rows = self.rows.lock().unwrap_or_else(|e| e.into_inner());
            let before = rows.len();
            rows.retain(|r| r.created_at >= cutoff);
            Ok((before - rows.len()) as u64)
        }
    }

    fn service(enabled: bool) -> (FeedbackService, Arc<MemoryLinks>) {
        let links = Arc::new(MemoryLinks::default());
        let settings = FeedbackSettings {
            enabled,
            ..FeedbackSettings::default()
        };
        let service = FeedbackService::new(
            Arc::clone(&links) as Arc<dyn TraceLinkStore>,
            &settings,
            Some("production".into()),
        );
        (service, links)
    }

    /// Collects scores emitted through the global sink.
    ///
    /// The sink is process-wide, so the tests that assert on emitted scores
    /// share one and run behind a lock rather than racing each other.
    static SINK_LOCK: OnceCell<tokio::sync::Mutex<()>> = OnceCell::const_new();

    async fn sink_guard() -> tokio::sync::MutexGuard<'static, ()> {
        SINK_LOCK
            .get_or_init(|| async { tokio::sync::Mutex::new(()) })
            .await
            .lock()
            .await
    }

    #[derive(Default)]
    struct CollectingSink {
        events: Mutex<Vec<moltis_observability::Event>>,
    }

    #[async_trait::async_trait]
    impl moltis_observability::ObservationSink for CollectingSink {
        fn name(&self) -> &str {
            "collecting"
        }

        fn record(&self, event: moltis_observability::Event) {
            self.events
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(event);
        }

        async fn flush(&self, _timeout: std::time::Duration) -> anyhow::Result<()> {
            Ok(())
        }
    }

    async fn link_reply(links: &MemoryLinks, message_id: &str, trace_id: &str) {
        links
            .link(TraceLink {
                channel_type: "telegram".into(),
                account_id: "bot-1".into(),
                chat_id: "chat-1".into(),
                message_id: message_id.into(),
                trace_id: trace_id.into(),
                session_key: Some("agent:main:main".into()),
                created_at: now_unix(),
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn every_chunk_of_a_reply_is_linked() {
        // Long replies are split across messages and a reader may react to any
        // of them, so all ids must resolve to the turn.
        let (service, links) = service(true);
        service
            .record_reply(
                "telegram",
                "bot-1",
                "chat-1",
                &["1".into(), "2".into(), "3".into()],
                &TraceId("trace-1".into()),
                Some("agent:main:main"),
            )
            .await;

        for id in ["1", "2", "3"] {
            let found = links
                .lookup("telegram", "bot-1", "chat-1", id)
                .await
                .unwrap();
            assert_eq!(found.expect("linked").trace_id, "trace-1");
        }
    }

    #[tokio::test]
    async fn a_thumbs_up_records_a_positive_score() {
        let _guard = sink_guard().await;
        let sink = Arc::new(CollectingSink::default());
        moltis_observability::set_global_sink(Arc::clone(&sink) as Arc<_>);

        let (service, links) = service(true);
        link_reply(&links, "42", "trace-1").await;

        let outcome = service
            .on_reaction(
                ChannelType::Telegram,
                "bot-1",
                "chat-1",
                "42",
                "\u{1f44d}",
                "99",
                true,
                None,
            )
            .await;

        assert_eq!(outcome, FeedbackOutcome::Recorded(FeedbackSignal::Positive));
        let events = sink.events.lock().unwrap_or_else(|e| e.into_inner());
        let scored = events
            .iter()
            .filter_map(|e| match e {
                moltis_observability::Event::Score(s) => Some(s),
                _ => None,
            })
            .count();
        assert_eq!(scored, 1);

        moltis_observability::clear_global_sink();
    }

    #[tokio::test]
    async fn a_reaction_that_is_not_feedback_costs_no_lookup() {
        let (service, _links) = service(true);
        // No link recorded at all: if this returned UnknownMessage it would
        // mean the vocabulary check ran after the database query.
        let outcome = service
            .on_reaction(
                ChannelType::Telegram,
                "bot-1",
                "chat-1",
                "42",
                "\u{1f389}",
                "99",
                true,
                None,
            )
            .await;

        assert_eq!(outcome, FeedbackOutcome::NotFeedback);
    }

    #[tokio::test]
    async fn a_reaction_on_an_unlinked_message_is_ignored() {
        let (service, _links) = service(true);
        let outcome = service
            .on_reaction(
                ChannelType::Telegram,
                "bot-1",
                "chat-1",
                "unknown",
                "\u{1f44d}",
                "99",
                true,
                None,
            )
            .await;

        assert_eq!(outcome, FeedbackOutcome::UnknownMessage);
    }

    #[tokio::test]
    async fn removing_a_reaction_retracts_rather_than_scoring_again() {
        let (service, links) = service(true);
        link_reply(&links, "42", "trace-1").await;

        let outcome = service
            .on_reaction(
                ChannelType::Telegram,
                "bot-1",
                "chat-1",
                "42",
                "\u{1f44d}",
                "99",
                false,
                None,
            )
            .await;

        assert_eq!(outcome, FeedbackOutcome::Retracted);
    }

    #[tokio::test]
    async fn feedback_can_be_switched_off() {
        let (service, links) = service(false);
        link_reply(&links, "42", "trace-1").await;

        let outcome = service
            .on_reaction(
                ChannelType::Telegram,
                "bot-1",
                "chat-1",
                "42",
                "\u{1f44d}",
                "99",
                true,
                None,
            )
            .await;

        assert_eq!(outcome, FeedbackOutcome::Disabled);
    }

    #[tokio::test]
    async fn disabled_feedback_records_no_links() {
        let (service, links) = service(false);
        service
            .record_reply(
                "telegram",
                "bot-1",
                "chat-1",
                &["1".into()],
                &TraceId("trace-1".into()),
                None,
            )
            .await;

        assert!(
            links
                .lookup("telegram", "bot-1", "chat-1", "1")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn pruning_removes_links_past_the_retention_window() {
        let (service, links) = service(true);
        links
            .link(TraceLink {
                channel_type: "telegram".into(),
                account_id: "bot-1".into(),
                chat_id: "chat-1".into(),
                message_id: "old".into(),
                trace_id: "trace-old".into(),
                session_key: None,
                created_at: now_unix() - 60 * 86_400,
            })
            .await
            .unwrap();
        link_reply(&links, "new", "trace-new").await;

        let removed = service.prune(30).await;

        assert_eq!(removed, 1);
        assert!(
            links
                .lookup("telegram", "bot-1", "chat-1", "new")
                .await
                .unwrap()
                .is_some()
        );
    }
}
