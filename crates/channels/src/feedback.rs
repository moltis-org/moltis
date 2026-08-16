//! Reaction feedback: recording which trace a reply came from, and turning a
//! later reaction on that reply into a score.
//!
//! The two halves are deliberately in one place because they share the
//! correlation key. The send side writes (channel, account, chat, message) →
//! trace; the reaction side reads it back.

use std::sync::{Arc, RwLock};

use {
    moltis_config::FeedbackSettings,
    moltis_observability::{
        FeedbackSignal, FeedbackVocabulary, ScoreDeleteRecord, TraceId, feedback_score,
        feedback_score_id,
    },
    tracing::{debug, warn},
};

use crate::trace_link::{TraceLink, TraceLinkStore};

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

/// The parts of the service that only exist once startup has a database.
struct Active {
    links: Arc<dyn TraceLinkStore>,
    vocabulary: FeedbackVocabulary,
    enabled: bool,
    environment: Option<String>,
    retention_days: u32,
}

/// Records reply/trace links and converts reactions into scores.
///
/// Constructed empty and filled in by [`FeedbackService::apply`] once startup
/// has a database pool and the resolved configuration, mirroring how
/// instrumentation itself is installed. Every operation is inert until then,
/// so a reaction arriving mid-startup is ignored rather than panicking.
#[derive(Default)]
pub struct FeedbackService {
    active: RwLock<Option<Active>>,
}

impl FeedbackService {
    /// Install the link store and settings.
    ///
    /// Replaces any previous setup so the settings UI can reconfigure feedback
    /// without a restart.
    pub fn apply(
        &self,
        links: Arc<dyn TraceLinkStore>,
        settings: &FeedbackSettings,
        environment: Option<String>,
    ) {
        let next = Some(Active {
            links,
            vocabulary: FeedbackVocabulary::from_config(&settings.positive, &settings.negative),
            enabled: settings.enabled,
            environment,
            retention_days: settings.link_retention_days,
        });
        match self.active.write() {
            Ok(mut guard) => *guard = next,
            Err(poisoned) => *poisoned.into_inner() = next,
        }
    }

    /// Whether feedback collection is on.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.read(|active| active.enabled).unwrap_or(false)
    }

    /// Run `f` against the installed configuration, if there is one.
    fn read<T>(&self, f: impl FnOnce(&Active) -> T) -> Option<T> {
        let guard = self
            .active
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.as_ref().map(f)
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
        let Some((links, enabled)) =
            self.read(|active| (Arc::clone(&active.links), active.enabled))
        else {
            return;
        };
        if !enabled || message_ids.is_empty() {
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
            if let Err(error) = links.link(link).await {
                warn!(%error, channel_type, "failed to record trace link for reply");
            }
        }
    }

    /// Handle a reaction change on a channel message.
    ///
    /// `scores_available` says whether a backend that can store a score is
    /// running. Without one, both halves of the toggle would be recorded into
    /// nothing, so the reaction is reported as disabled instead.
    pub async fn on_reaction(
        &self,
        channel: &str,
        account_id: &str,
        chat_id: &str,
        message_id: &str,
        emoji: &str,
        user_id: &str,
        added: bool,
        scores_available: bool,
    ) -> FeedbackOutcome {
        // Classify before the database lookup: most reactions in a busy chat
        // are not feedback, and they should not cost a query each.
        let Some((enabled, signal)) =
            self.read(|active| (active.enabled, active.vocabulary.classify(emoji)))
        else {
            return FeedbackOutcome::Disabled;
        };
        if !enabled {
            return FeedbackOutcome::Disabled;
        }
        let Some(signal) = signal else {
            return FeedbackOutcome::NotFeedback;
        };

        self.submit_signal(
            channel,
            account_id,
            chat_id,
            message_id,
            added.then_some(signal),
            user_id,
            Some(format!("{channel} reaction {emoji}")),
            scores_available,
        )
        .await
    }

    /// Submit already-classified feedback from a typed UI boundary.
    #[allow(clippy::too_many_arguments)]
    pub async fn submit_signal(
        &self,
        channel: &str,
        account_id: &str,
        chat_id: &str,
        message_id: &str,
        signal: Option<FeedbackSignal>,
        user_id: &str,
        comment: Option<String>,
        scores_available: bool,
    ) -> FeedbackOutcome {
        let Some((links, enabled, environment, retention_days)) = self.read(|active| {
            (
                Arc::clone(&active.links),
                active.enabled,
                active.environment.clone(),
                active.retention_days,
            )
        }) else {
            return FeedbackOutcome::Disabled;
        };
        if !enabled || !scores_available {
            return FeedbackOutcome::Disabled;
        }

        let link = match links.lookup(channel, account_id, chat_id, message_id).await {
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
        if link.created_at < retention_cutoff(retention_days) {
            return FeedbackOutcome::UnknownMessage;
        }

        let trace_id = TraceId(link.trace_id);
        // Namespaced so the same numeric user id on two channels is two people.
        let scoped_user = format!("{channel}:{user_id}");

        if let Some(signal) = signal {
            let score = feedback_score(&trace_id, signal, Some(&scoped_user), comment, environment);
            moltis_observability::record(moltis_observability::Event::Score(Box::new(score)));
            debug!(channel, ?signal, "recorded reaction feedback");
            return FeedbackOutcome::Recorded(signal);
        }

        let score_id = feedback_score_id(&trace_id, Some(&scoped_user));
        moltis_observability::record(moltis_observability::Event::ScoreDelete(Box::new(
            ScoreDeleteRecord::new(trace_id, score_id),
        )));
        FeedbackOutcome::Retracted
    }

    /// Drop links older than the active retention window.
    pub async fn prune(&self) -> u64 {
        let Some((links, retention_days)) =
            self.read(|active| (Arc::clone(&active.links), active.retention_days))
        else {
            return 0;
        };
        let cutoff = retention_cutoff(retention_days);
        match links.prune(cutoff).await {
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

fn retention_cutoff(retention_days: u32) -> i64 {
    time::OffsetDateTime::now_utc()
        .checked_sub(time::Duration::days(i64::from(retention_days)))
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
        .unix_timestamp()
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {crate::Result as ChannelResult, std::sync::Mutex, tokio::sync::OnceCell};

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
        let service = FeedbackService::default();
        service.apply(
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
                "telegram",
                "bot-1",
                "chat-1",
                "42",
                "\u{1f44d}",
                "99",
                true,
                true,
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
                "telegram",
                "bot-1",
                "chat-1",
                "42",
                "\u{1f389}",
                "99",
                true,
                false,
            )
            .await;

        assert_eq!(outcome, FeedbackOutcome::NotFeedback);
    }

    #[tokio::test]
    async fn a_reaction_on_an_unlinked_message_is_ignored() {
        let (service, _links) = service(true);
        let outcome = service
            .on_reaction(
                "telegram",
                "bot-1",
                "chat-1",
                "unknown",
                "\u{1f44d}",
                "99",
                true,
                true,
            )
            .await;

        assert_eq!(outcome, FeedbackOutcome::UnknownMessage);
    }

    #[tokio::test]
    async fn removing_a_reaction_retracts_rather_than_scoring_again() {
        let _guard = sink_guard().await;
        let sink = Arc::new(CollectingSink::default());
        moltis_observability::set_global_sink(Arc::clone(&sink) as Arc<_>);
        let (service, links) = service(true);
        link_reply(&links, "42", "trace-1").await;

        let outcome = service
            .on_reaction(
                "telegram",
                "bot-1",
                "chat-1",
                "42",
                "\u{1f44d}",
                "99",
                false,
                true,
            )
            .await;

        assert_eq!(outcome, FeedbackOutcome::Retracted);
        let events = sink.events.lock().unwrap_or_else(|e| e.into_inner());
        assert!(matches!(events.as_slice(), [
            moltis_observability::Event::ScoreDelete(_)
        ]));
        drop(events);
        moltis_observability::clear_global_sink();
    }

    #[tokio::test]
    async fn typed_feedback_does_not_depend_on_the_emoji_vocabulary() {
        let _guard = sink_guard().await;
        let sink = Arc::new(CollectingSink::default());
        moltis_observability::set_global_sink(Arc::clone(&sink) as Arc<_>);
        let links = Arc::new(MemoryLinks::default());
        let service = FeedbackService::default();
        service.apply(
            Arc::clone(&links) as Arc<dyn TraceLinkStore>,
            &FeedbackSettings {
                positive: vec!["party".into()],
                negative: vec!["boo".into()],
                ..FeedbackSettings::default()
            },
            Some("production".into()),
        );
        link_reply(&links, "42", "trace-1").await;

        let outcome = service
            .submit_signal(
                "telegram",
                "bot-1",
                "chat-1",
                "42",
                Some(FeedbackSignal::Positive),
                "99",
                Some("typed feedback".into()),
                true,
            )
            .await;

        assert_eq!(outcome, FeedbackOutcome::Recorded(FeedbackSignal::Positive));
        assert!(matches!(
            sink.events
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_slice(),
            [moltis_observability::Event::Score(_)]
        ));
        moltis_observability::clear_global_sink();
    }

    #[tokio::test]
    async fn expired_links_are_rejected_even_before_pruning() {
        let (service, links) = service(true);
        links
            .link(TraceLink {
                channel_type: "telegram".into(),
                account_id: "bot-1".into(),
                chat_id: "chat-1".into(),
                message_id: "old".into(),
                trace_id: "trace-old".into(),
                session_key: None,
                created_at: retention_cutoff(30) - 1,
            })
            .await
            .unwrap();

        let outcome = service
            .on_reaction(
                "telegram",
                "bot-1",
                "chat-1",
                "old",
                "\u{1f44d}",
                "99",
                true,
                true,
            )
            .await;

        assert_eq!(outcome, FeedbackOutcome::UnknownMessage);
    }

    #[tokio::test]
    async fn score_feedback_is_disabled_without_a_score_backend() {
        let (service, links) = service(true);
        link_reply(&links, "42", "trace-1").await;

        let outcome = service
            .on_reaction(
                "telegram",
                "bot-1",
                "chat-1",
                "42",
                "\u{1f44d}",
                "99",
                true,
                false,
            )
            .await;

        assert_eq!(outcome, FeedbackOutcome::Disabled);
    }

    #[tokio::test]
    async fn feedback_can_be_switched_off() {
        let (service, links) = service(false);
        link_reply(&links, "42", "trace-1").await;

        let outcome = service
            .on_reaction(
                "telegram",
                "bot-1",
                "chat-1",
                "42",
                "\u{1f44d}",
                "99",
                true,
                true,
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
    async fn an_uninitialized_service_is_inert() {
        // A reaction arriving before startup finishes must be ignored rather
        // than panicking or scoring against nothing.
        let service = FeedbackService::default();

        assert!(!service.enabled());
        assert_eq!(
            service
                .on_reaction(
                    "telegram",
                    "bot-1",
                    "chat-1",
                    "42",
                    "\u{1f44d}",
                    "99",
                    true,
                    true,
                )
                .await,
            FeedbackOutcome::Disabled
        );
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
        assert_eq!(service.prune().await, 0);
    }

    #[tokio::test]
    async fn reapplying_settings_takes_effect_without_a_restart() {
        let (service, links) = service(true);
        link_reply(&links, "42", "trace-1").await;

        service.apply(
            Arc::clone(&links) as Arc<dyn TraceLinkStore>,
            &FeedbackSettings {
                enabled: false,
                ..FeedbackSettings::default()
            },
            None,
        );

        assert_eq!(
            service
                .on_reaction(
                    "telegram",
                    "bot-1",
                    "chat-1",
                    "42",
                    "\u{1f44d}",
                    "99",
                    true,
                    true,
                )
                .await,
            FeedbackOutcome::Disabled
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
                created_at: now_unix() - time::Duration::days(60).whole_seconds(),
            })
            .await
            .unwrap();
        link_reply(&links, "new", "trace-new").await;

        let removed = service.prune().await;

        assert_eq!(removed, 1);
        assert!(
            links
                .lookup("telegram", "bot-1", "chat-1", "new")
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn pruning_uses_reconfigured_retention_window() {
        let (service, links) = service(true);
        links
            .link(TraceLink {
                channel_type: "telegram".into(),
                account_id: "bot-1".into(),
                chat_id: "chat-1".into(),
                message_id: "middle-aged".into(),
                trace_id: "trace-middle-aged".into(),
                session_key: None,
                created_at: now_unix() - time::Duration::days(60).whole_seconds(),
            })
            .await
            .unwrap();
        service.apply(
            Arc::clone(&links) as Arc<dyn TraceLinkStore>,
            &FeedbackSettings {
                link_retention_days: 90,
                ..FeedbackSettings::default()
            },
            None,
        );

        assert_eq!(service.prune().await, 0);
        assert!(
            links
                .lookup("telegram", "bot-1", "chat-1", "middle-aged")
                .await
                .unwrap()
                .is_some()
        );
    }
}
