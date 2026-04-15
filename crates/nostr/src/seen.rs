//! LRU dedup tracker for Nostr event IDs.
//!
//! Prevents processing the same event twice when relays replay events
//! on reconnect. Uses a bounded HashMap with insertion-order eviction.

use std::collections::HashMap;

use nostr_sdk::prelude::EventId;

/// Maximum number of event IDs to track.
const DEFAULT_CAPACITY: usize = 100_000;

/// How long an event ID is considered "seen" (1 hour).
const TTL_SECS: i64 = 3600;

/// LRU-style dedup tracker keyed by Nostr event ID.
pub struct SeenTracker {
    entries: HashMap<EventId, i64>,
    capacity: usize,
}

fn now_unix() -> i64 {
    ::time::OffsetDateTime::now_utc().unix_timestamp()
}

impl SeenTracker {
    /// Create a new tracker with default capacity.
    pub fn new() -> Self {
        Self {
            entries: HashMap::with_capacity(1024),
            capacity: DEFAULT_CAPACITY,
        }
    }

    /// Create a tracker with custom capacity.
    #[cfg(test)]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: HashMap::with_capacity(capacity.min(1024)),
            capacity,
        }
    }

    /// Returns `true` if this event ID has already been seen (within TTL).
    /// Inserts it if not yet seen.
    pub fn check_and_insert(&mut self, id: &EventId) -> bool {
        let now = now_unix();

        // Already seen and within TTL
        if let Some(&ts) = self.entries.get(id)
            && now - ts < TTL_SECS
        {
            return true;
        }
        // Expired entries will be re-inserted below

        // Evict oldest entries if at capacity
        if self.entries.len() >= self.capacity {
            self.evict_expired(now);
        }
        // If still at capacity after expiry sweep, remove ~10% oldest
        if self.entries.len() >= self.capacity {
            self.evict_oldest();
        }

        self.entries.insert(*id, now);
        false
    }

    /// Remove entries older than TTL.
    fn evict_expired(&mut self, now: i64) {
        self.entries.retain(|_, ts| now - *ts < TTL_SECS);
    }

    /// Remove the oldest ~10% of entries by timestamp.
    fn evict_oldest(&mut self) {
        let to_remove = self.capacity / 10;
        let mut by_age: Vec<(EventId, i64)> = self.entries.iter().map(|(k, v)| (*k, *v)).collect();
        by_age.sort_by_key(|(_, ts)| *ts);
        for (id, _) in by_age.into_iter().take(to_remove) {
            self.entries.remove(&id);
        }
    }

    /// Number of tracked event IDs.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the tracker is empty.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for SeenTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use nostr_sdk::prelude::Keys;

    use super::*;

    fn random_event_id() -> EventId {
        EventId::from_byte_array(Keys::generate().public_key().to_bytes())
    }

    #[test]
    fn first_insert_returns_false() {
        let mut tracker = SeenTracker::new();
        let id = random_event_id();
        assert!(!tracker.check_and_insert(&id));
    }

    #[test]
    fn second_insert_returns_true() {
        let mut tracker = SeenTracker::new();
        let id = random_event_id();
        assert!(!tracker.check_and_insert(&id));
        assert!(tracker.check_and_insert(&id));
    }

    #[test]
    fn different_ids_not_confused() {
        let mut tracker = SeenTracker::new();
        let id1 = random_event_id();
        let id2 = random_event_id();
        assert!(!tracker.check_and_insert(&id1));
        assert!(!tracker.check_and_insert(&id2));
        assert!(tracker.check_and_insert(&id1));
        assert!(tracker.check_and_insert(&id2));
    }

    #[test]
    fn capacity_eviction() {
        let mut tracker = SeenTracker::with_capacity(10);
        for _ in 0..20 {
            let id = random_event_id();
            tracker.check_and_insert(&id);
        }
        // Should never exceed capacity (plus small margin from eviction batching)
        assert!(tracker.len() <= 10);
    }
}
