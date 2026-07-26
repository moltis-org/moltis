//! Remembers how to answer an inbound group message.
//!
//! Two things about a group message are needed to reply correctly but are not
//! recoverable from the outbound side, where all the gateway hands back is a
//! chat id and the event id being replied to:
//!
//! * **Who wrote it** — so the reply can `p`-tag them, which is what turns a
//!   reply into a notification for that person on NIP-29 clients.
//! * **Which message kind it used** — Buzz posts `kind:40002`
//!   (`KIND_STREAM_MESSAGE_V2`) while plain NIP-29 relays use `kind:9`, and a
//!   client filtering for one does not see the other. Answering in the dialect
//!   we were addressed in keeps the bot visible on both without configuration.
//!
//! Entries are bounded and evicted oldest-first; losing one only costs the
//! `p` tag and falls back to the configured default kind.

use std::collections::HashMap;

use nostr_sdk::prelude::{EventId, Kind, PublicKey};

/// Maximum number of remembered messages.
const DEFAULT_CAPACITY: usize = 10_000;

/// How to answer a specific inbound group message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplyContext {
    /// Author of the message, `p`-tagged so the reply notifies them.
    pub author: PublicKey,
    /// Kind the message used, mirrored so the reply lands in the same view.
    pub kind: Kind,
}

/// Bounded store of per-event reply context, plus the last kind seen per group.
pub struct ReplyContexts {
    /// event id -> (context, insertion sequence)
    entries: HashMap<EventId, (ReplyContext, u64)>,
    /// group id -> most recent message kind observed in that group
    group_kinds: HashMap<String, Kind>,
    capacity: usize,
    seq: u64,
}

impl ReplyContexts {
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            group_kinds: HashMap::new(),
            capacity: capacity.max(1),
            seq: 0,
        }
    }

    /// Record an inbound group message.
    pub fn record(&mut self, event_id: EventId, group_id: &str, author: PublicKey, kind: Kind) {
        if self.entries.len() >= self.capacity {
            self.evict_oldest();
        }
        self.seq = self.seq.saturating_add(1);
        self.entries
            .insert(event_id, (ReplyContext { author, kind }, self.seq));
        self.group_kinds.insert(group_id.to_string(), kind);
    }

    /// Look up how to answer a specific message.
    #[must_use]
    pub fn get(&self, event_id: &EventId) -> Option<ReplyContext> {
        self.entries.get(event_id).map(|(ctx, _)| *ctx)
    }

    /// The kind last seen in a group, for sends that are not replies.
    #[must_use]
    pub fn kind_for_group(&self, group_id: &str) -> Option<Kind> {
        self.group_kinds.get(group_id).copied()
    }

    /// Drop the oldest ~10% of entries (at least one) to stay under capacity.
    fn evict_oldest(&mut self) {
        let to_remove = (self.capacity / 10).max(1);
        let mut by_age: Vec<(EventId, u64)> = self
            .entries
            .iter()
            .map(|(id, (_, seq))| (*id, *seq))
            .collect();
        by_age.sort_by_key(|(_, seq)| *seq);
        for (id, _) in by_age.into_iter().take(to_remove) {
            self.entries.remove(&id);
        }
    }

    /// Number of remembered messages.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether anything is remembered yet.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for ReplyContexts {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use nostr_sdk::prelude::Keys;

    use super::*;

    fn event_id() -> EventId {
        EventId::from_byte_array(Keys::generate().public_key().to_bytes())
    }

    #[test]
    fn records_and_returns_author_and_kind() {
        let mut ctxs = ReplyContexts::new();
        let id = event_id();
        let author = Keys::generate().public_key();
        ctxs.record(id, "grp", author, Kind::from_u16(40002));

        let got = ctxs.get(&id);
        assert_eq!(got.map(|c| c.author), Some(author));
        assert_eq!(got.map(|c| c.kind), Some(Kind::from_u16(40002)));
    }

    #[test]
    fn unknown_event_has_no_context() {
        let ctxs = ReplyContexts::new();
        assert!(ctxs.is_empty());
        assert!(ctxs.get(&event_id()).is_none());
    }

    /// A group's dialect is learned from traffic so non-reply sends still land
    /// in the right view.
    #[test]
    fn tracks_last_kind_per_group() {
        let mut ctxs = ReplyContexts::new();
        ctxs.record(
            event_id(),
            "buzz",
            Keys::generate().public_key(),
            Kind::from_u16(40002),
        );
        ctxs.record(
            event_id(),
            "nip29",
            Keys::generate().public_key(),
            Kind::from_u16(9),
        );

        assert_eq!(ctxs.kind_for_group("buzz"), Some(Kind::from_u16(40002)));
        assert_eq!(ctxs.kind_for_group("nip29"), Some(Kind::from_u16(9)));
        assert_eq!(ctxs.kind_for_group("unseen"), None);
    }

    #[test]
    fn newer_kind_wins_for_group() {
        let mut ctxs = ReplyContexts::new();
        let author = Keys::generate().public_key();
        ctxs.record(event_id(), "grp", author, Kind::from_u16(9));
        ctxs.record(event_id(), "grp", author, Kind::from_u16(40002));
        assert_eq!(ctxs.kind_for_group("grp"), Some(Kind::from_u16(40002)));
    }

    #[test]
    fn evicts_to_stay_within_capacity() {
        let mut ctxs = ReplyContexts::with_capacity(10);
        let author = Keys::generate().public_key();
        for _ in 0..50 {
            ctxs.record(event_id(), "grp", author, Kind::from_u16(9));
        }
        assert!(ctxs.len() <= 10, "capacity must be respected");
    }

    /// Eviction must not lose the group dialect, which outlives individual
    /// messages and is what non-reply sends depend on.
    #[test]
    fn eviction_preserves_group_kind() {
        let mut ctxs = ReplyContexts::with_capacity(4);
        let author = Keys::generate().public_key();
        for _ in 0..20 {
            ctxs.record(event_id(), "grp", author, Kind::from_u16(40002));
        }
        assert_eq!(ctxs.kind_for_group("grp"), Some(Kind::from_u16(40002)));
    }
}
