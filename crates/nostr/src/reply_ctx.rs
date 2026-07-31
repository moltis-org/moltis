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
    /// (reacted-to event, emoji) -> our reaction's own event id.
    ///
    /// NIP-25 has no "unreact": retracting means publishing a NIP-09 deletion
    /// that references the reaction event, so its id has to be kept around
    /// between adding 👀 and replacing it with ✅/❌.
    reactions: HashMap<(EventId, String), (EventId, u64)>,
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
            reactions: HashMap::new(),
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

    /// Remember the reaction event we published, so it can be retracted later.
    ///
    /// Bounded like `entries`: the gateway retracts the 👀 it adds on receipt,
    /// but the terminal ✅/❌ is never retracted, so without a ceiling this map
    /// would grow by one entry per completed group turn for the life of the
    /// process — driven entirely by remote traffic. Losing an old id only costs
    /// a best-effort retraction that nothing is waiting on.
    pub fn record_reaction(&mut self, target: EventId, emoji: &str, reaction: EventId) {
        if self.reactions.len() >= self.capacity {
            self.evict_oldest_reactions();
        }
        self.seq = self.seq.saturating_add(1);
        self.reactions
            .insert((target, emoji.to_string()), (reaction, self.seq));
    }

    /// Drop the oldest ~10% of remembered reactions (at least one).
    fn evict_oldest_reactions(&mut self) {
        let to_remove = (self.capacity / 10).max(1);
        let mut by_age: Vec<((EventId, String), u64)> = self
            .reactions
            .iter()
            .map(|(key, (_, seq))| (key.clone(), *seq))
            .collect();
        by_age.sort_by_key(|(_, seq)| *seq);
        for (key, _) in by_age.into_iter().take(to_remove) {
            self.reactions.remove(&key);
        }
    }

    /// The id of a reaction we published, if we still know it.
    ///
    /// Deliberately non-consuming: the caller has to publish a NIP-09 deletion
    /// before the mapping can be dropped, and that publish can fail. Forgetting
    /// the id first would strand the reaction — this is the only record of it,
    /// so a failed retraction could never be retried and the bot's 👀 would sit
    /// on the user's message for good. Pair with [`Self::forget_reaction`].
    #[must_use]
    pub fn reaction_id(&self, target: EventId, emoji: &str) -> Option<EventId> {
        self.reactions
            .get(&(target, emoji.to_string()))
            .map(|(reaction, _)| *reaction)
    }

    /// Every reaction still recorded for `target` other than `except`.
    ///
    /// The acknowledgement lifecycle is 👀, then ✅ or ❌ replacing it, so by
    /// the time a new emoji is published anything else still recorded for that
    /// message is a retraction that did not go through. Returning them lets the
    /// next publish clear them while it is already talking to the relay —
    /// otherwise nothing revisits a failed retraction, because the gateway's
    /// reaction worker ends the turn immediately after it.
    #[must_use]
    pub fn stale_reactions(&self, target: EventId, except: &str) -> Vec<(String, EventId)> {
        self.reactions
            .iter()
            .filter(|((event, emoji), _)| *event == target && emoji != except)
            .map(|((_, emoji), (reaction, _))| (emoji.clone(), *reaction))
            .collect()
    }

    /// Drop a reaction mapping once its deletion has been published.
    pub fn forget_reaction(&mut self, target: EventId, emoji: &str) {
        self.reactions.remove(&(target, emoji.to_string()));
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
    fn reaction_ids_round_trip_for_retraction() {
        let mut ctxs = ReplyContexts::new();
        let target = event_id();
        let reaction = event_id();
        ctxs.record_reaction(target, "\u{1F440}", reaction);

        // Reading does not forget: the caller has to publish a deletion first,
        // and that can fail — losing the id would strand the reaction.
        assert_eq!(ctxs.reaction_id(target, "\u{1F440}"), Some(reaction));
        assert_eq!(ctxs.reaction_id(target, "\u{1F440}"), Some(reaction));

        ctxs.forget_reaction(target, "\u{1F440}");
        assert_eq!(ctxs.reaction_id(target, "\u{1F440}"), None);
    }

    #[test]
    fn reactions_are_keyed_by_target_and_emoji() {
        let mut ctxs = ReplyContexts::new();
        let target = event_id();
        let eyes = event_id();
        let check = event_id();
        ctxs.record_reaction(target, "\u{1F440}", eyes);
        ctxs.record_reaction(target, "\u{2705}", check);

        assert_eq!(ctxs.reaction_id(target, "\u{2705}"), Some(check));
        // Forgetting one must not disturb the other.
        ctxs.forget_reaction(target, "\u{2705}");
        assert_eq!(ctxs.reaction_id(target, "\u{1F440}"), Some(eyes));
        assert_eq!(ctxs.reaction_id(event_id(), "\u{1F440}"), None);
    }

    /// A failed retraction must stay retryable. The mapping is the only record
    /// of the reaction event, so a caller that publishes a deletion and fails
    /// has to still find the id afterwards.
    #[test]
    fn a_failed_retraction_keeps_the_id_for_a_retry() {
        let mut ctxs = ReplyContexts::new();
        let target = event_id();
        let reaction = event_id();
        ctxs.record_reaction(target, "\u{1F440}", reaction);

        // Deletion failed: nothing was forgotten, so it can be tried again.
        assert_eq!(ctxs.reaction_id(target, "\u{1F440}"), Some(reaction));
        assert_eq!(ctxs.reaction_id(target, "\u{1F440}"), Some(reaction));

        // Deletion succeeded: now it is dropped.
        ctxs.forget_reaction(target, "\u{1F440}");
        assert_eq!(ctxs.reaction_id(target, "\u{1F440}"), None);
    }

    /// The gateway never retracts the terminal ✅/❌, so without a ceiling this
    /// map would grow once per completed group turn, forever.
    #[test]
    fn reactions_are_bounded_too() {
        let mut ctxs = ReplyContexts::with_capacity(10);
        let author = Keys::generate().public_key();
        for _ in 0..200 {
            ctxs.record_reaction(event_id(), "\u{2705}", event_id());
        }
        assert!(
            ctxs.reactions.len() <= 10,
            "reaction ids must not grow without bound, got {}",
            ctxs.reactions.len()
        );
        let _ = author;
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
