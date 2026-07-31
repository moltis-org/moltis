use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, RwLock},
    time::{Duration, Instant},
};

use {
    moltis_channels::{ChannelEventSink, message_log::MessageLog, otp::OtpState},
    tokio_util::sync::CancellationToken,
};

use crate::config::SlackAccountConfig;

/// Shared account state map.
pub type AccountStateMap = Arc<RwLock<HashMap<String, AccountState>>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamRecipient {
    pub user_id: String,
    pub team_id: String,
}

type StreamRecipientKey = (String, String);

#[derive(Clone)]
struct StreamRecipientEntry {
    recipient: StreamRecipient,
    last_seen: Instant,
}

/// Bounded, TTL-limited recipient cache ordered by last observation.
#[derive(Default)]
pub struct StreamRecipients {
    entries: HashMap<StreamRecipientKey, StreamRecipientEntry>,
    order: VecDeque<StreamRecipientKey>,
}

impl StreamRecipients {
    const MAX: usize = 2048;
    const TTL: Duration = Duration::from_secs(30 * 60);

    fn get(&self, channel_id: &str, thread_root: &str) -> Option<&StreamRecipient> {
        let entry = self
            .entries
            .get(&(channel_id.to_string(), thread_root.to_string()))?;
        (entry.last_seen.elapsed() <= Self::TTL).then_some(&entry.recipient)
    }

    fn note(&mut self, channel_id: &str, thread_root: &str, recipient: StreamRecipient) {
        self.note_at_with_limit(
            channel_id,
            thread_root,
            recipient,
            Instant::now(),
            Self::MAX,
        );
    }

    fn note_at_with_limit(
        &mut self,
        channel_id: &str,
        thread_root: &str,
        recipient: StreamRecipient,
        now: Instant,
        max: usize,
    ) {
        while let Some(oldest) = self.order.front() {
            let expired = self
                .entries
                .get(oldest)
                .is_none_or(|entry| now.saturating_duration_since(entry.last_seen) > Self::TTL);
            if !expired {
                break;
            }
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }

        let key = (channel_id.to_string(), thread_root.to_string());
        if self.entries.contains_key(&key) {
            self.order.retain(|existing| existing != &key);
        } else {
            while self.entries.len() >= max.max(1) {
                let Some(oldest) = self.order.pop_front() else {
                    break;
                };
                self.entries.remove(&oldest);
            }
        }

        self.entries.insert(key.clone(), StreamRecipientEntry {
            recipient,
            last_seen: now,
        });
        self.order.push_back(key);
    }
}

/// Category of Slack callback identifier being deduplicated.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DedupKind {
    Event,
    Command,
    Interaction,
}

#[derive(Default)]
struct DedupPartition {
    seen: HashMap<String, Instant>,
    order: VecDeque<(String, Instant)>,
}

/// Bounded, per-kind set of recently seen Slack callback ids.
///
/// Slack retries callbacks for up to five minutes. Each account and callback
/// kind retains the documented maximum of roughly 2,500 ids for that window,
/// so traffic in one category cannot evict another category's retry keys.
#[derive(Default)]
pub struct EventDedup {
    partitions: HashMap<DedupKind, DedupPartition>,
}

impl EventDedup {
    const MAX_PER_KIND: usize = 2_500;
    const RETRY_WINDOW: Duration = Duration::from_secs(5 * 60);

    /// Record an event id, returning `true` if it had not been seen before.
    pub fn insert_new(&mut self, kind: DedupKind, event_id: &str) -> bool {
        self.insert_new_at(kind, event_id, Instant::now())
    }

    /// Return whether an id is still inside the retry window without recording it.
    pub(crate) fn contains(&mut self, kind: DedupKind, event_id: &str) -> bool {
        self.contains_at(kind, event_id, Instant::now())
    }

    fn contains_at(&mut self, kind: DedupKind, event_id: &str, now: Instant) -> bool {
        let partition = self.partitions.entry(kind).or_default();
        Self::prune_partition(partition, now);
        partition.seen.contains_key(event_id)
    }

    fn insert_new_at(&mut self, kind: DedupKind, event_id: &str, now: Instant) -> bool {
        let partition = self.partitions.entry(kind).or_default();
        Self::prune_partition(partition, now);

        if partition.seen.contains_key(event_id) {
            return false;
        }

        while partition.seen.len() >= Self::MAX_PER_KIND {
            let Some((oldest_id, inserted_at)) = partition.order.pop_front() else {
                break;
            };
            if partition.seen.get(&oldest_id) == Some(&inserted_at) {
                partition.seen.remove(&oldest_id);
            }
        }
        partition.seen.insert(event_id.to_string(), now);
        partition.order.push_back((event_id.to_string(), now));
        true
    }

    fn prune_partition(partition: &mut DedupPartition, now: Instant) {
        while let Some((oldest_id, inserted_at)) = partition.order.front() {
            if now.saturating_duration_since(*inserted_at) <= Self::RETRY_WINDOW {
                break;
            }
            if partition.seen.get(oldest_id) == Some(inserted_at) {
                partition.seen.remove(oldest_id);
            }
            partition.order.pop_front();
        }
    }
}

/// Per-account runtime state.
pub struct AccountState {
    pub account_id: String,
    pub config: SlackAccountConfig,
    pub message_log: Option<Arc<dyn MessageLog>>,
    pub event_sink: Option<Arc<dyn ChannelEventSink>>,
    pub cancel: CancellationToken,
    /// Bot user ID obtained from `auth.test` — signals the connection is ready.
    pub bot_user_id: Option<String>,
    /// Native-stream recipients keyed by the exact `(channel, thread root)`.
    pub stream_recipients: StreamRecipients,
    pub otp: Mutex<OtpState>,
    /// Recently processed event ids, for retry idempotency.
    pub dedup: Mutex<EventDedup>,
}

impl AccountState {
    pub fn note_stream_recipient(
        &mut self,
        channel_id: &str,
        thread_root: &str,
        recipient: StreamRecipient,
    ) {
        self.stream_recipients
            .note(channel_id, thread_root, recipient);
    }

    pub fn stream_recipient(
        &self,
        channel_id: &str,
        thread_root: &str,
    ) -> Option<&StreamRecipient> {
        self.stream_recipients.get(channel_id, thread_root)
    }
}

#[cfg(test)]
mod tests {
    use {
        super::{DedupKind, EventDedup, StreamRecipient, StreamRecipients},
        std::time::{Duration, Instant},
    };

    fn recipient(user_id: &str) -> StreamRecipient {
        StreamRecipient {
            user_id: user_id.to_string(),
            team_id: "T1".to_string(),
        }
    }

    #[test]
    fn event_dedup_retains_documented_five_minute_throughput() {
        let now = Instant::now();
        let mut dedup = EventDedup::default();
        for index in 0..EventDedup::MAX_PER_KIND {
            assert!(dedup.insert_new_at(DedupKind::Event, &index.to_string(), now));
        }
        assert!(!dedup.insert_new_at(DedupKind::Event, "0", now + EventDedup::RETRY_WINDOW));

        let after_window = now + EventDedup::RETRY_WINDOW + Duration::from_millis(1);
        assert!(dedup.insert_new_at(DedupKind::Event, "0", after_window));
    }

    #[test]
    fn event_kinds_have_separate_capacity_partitions() {
        let now = Instant::now();
        let mut dedup = EventDedup::default();
        for index in 0..EventDedup::MAX_PER_KIND {
            assert!(dedup.insert_new_at(DedupKind::Event, &index.to_string(), now));
        }
        assert!(dedup.insert_new_at(DedupKind::Command, "same", now));
        assert!(dedup.insert_new_at(DedupKind::Interaction, "same", now));
        assert!(!dedup.insert_new_at(DedupKind::Command, "same", now));
    }

    #[test]
    fn stream_recipient_refresh_does_not_evict_and_updates_age_order() {
        let now = Instant::now();
        let mut recipients = StreamRecipients::default();
        recipients.note_at_with_limit("C1", "1.0", recipient("U1"), now, 2);
        recipients.note_at_with_limit(
            "C1",
            "2.0",
            recipient("U2"),
            now + Duration::from_secs(1),
            2,
        );

        recipients.note_at_with_limit(
            "C1",
            "1.0",
            recipient("U1-new"),
            now + Duration::from_secs(2),
            2,
        );
        assert_eq!(recipients.get("C1", "1.0"), Some(&recipient("U1-new")));
        assert_eq!(recipients.get("C1", "2.0"), Some(&recipient("U2")));

        recipients.note_at_with_limit(
            "C1",
            "3.0",
            recipient("U3"),
            now + Duration::from_secs(3),
            2,
        );
        assert!(recipients.get("C1", "2.0").is_none());
        assert!(recipients.get("C1", "1.0").is_some());
        assert!(recipients.get("C1", "3.0").is_some());
    }

    #[test]
    fn stream_recipient_evicts_expired_entries_before_fresh_entries() {
        let now = Instant::now();
        let mut recipients = StreamRecipients::default();
        recipients.note_at_with_limit("C1", "old", recipient("U-old"), now, 2);
        recipients.note_at_with_limit(
            "C1",
            "fresh",
            recipient("U-fresh"),
            now + StreamRecipients::TTL,
            2,
        );
        recipients.note_at_with_limit(
            "C1",
            "new",
            recipient("U-new"),
            now + StreamRecipients::TTL + Duration::from_millis(1),
            2,
        );

        assert!(recipients.get("C1", "old").is_none());
        assert!(recipients.get("C1", "fresh").is_some());
        assert!(recipients.get("C1", "new").is_some());
    }
}
