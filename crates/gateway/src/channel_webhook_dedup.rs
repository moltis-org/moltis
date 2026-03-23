//! Channel webhook idempotency deduplication store.
//!
//! Provides TTL-based deduplication for channel webhook requests keyed by
//! provider-specific message IDs (e.g. Slack `event_id`, Teams `activity.id`).
//!
//! Extracted from the former `DedupeCache` in `state.rs`.

use std::{collections::HashMap, time::Instant};

struct DedupeEntry {
    inserted_at: Instant,
}

/// TTL-based idempotency store for channel webhook deduplication.
///
/// Tracks provider-specific message IDs to detect duplicate deliveries.
/// Entries expire after `ttl` and the store caps at `max_entries`.
pub struct ChannelWebhookDedupeStore {
    entries: HashMap<String, DedupeEntry>,
    ttl: std::time::Duration,
    max_entries: usize,
}

impl Default for ChannelWebhookDedupeStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelWebhookDedupeStore {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            ttl: std::time::Duration::from_millis(moltis_protocol::DEDUPE_TTL_MS),
            max_entries: moltis_protocol::DEDUPE_MAX_ENTRIES,
        }
    }

    /// Returns `true` if the key is a duplicate (already seen within TTL).
    /// If new, inserts the key and returns `false`.
    pub fn check_and_insert(&mut self, key: &str) -> bool {
        self.evict_expired();
        if self.entries.contains_key(key) {
            return true;
        }
        if self.entries.len() >= self.max_entries
            && let Some(oldest_key) = self
                .entries
                .iter()
                .min_by_key(|(_, v)| v.inserted_at)
                .map(|(k, _)| k.clone())
        {
            self.entries.remove(&oldest_key);
        }
        self.entries.insert(key.to_string(), DedupeEntry {
            inserted_at: Instant::now(),
        });
        false
    }

    fn evict_expired(&mut self) {
        let cutoff = Instant::now() - self.ttl;
        self.entries.retain(|_, v| v.inserted_at > cutoff);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn new_key_returns_false() {
        let mut store = ChannelWebhookDedupeStore::new();
        assert!(!store.check_and_insert("key1"));
    }

    #[test]
    fn duplicate_key_returns_true() {
        let mut store = ChannelWebhookDedupeStore::new();
        assert!(!store.check_and_insert("key1"));
        assert!(store.check_and_insert("key1"));
    }

    #[test]
    fn different_keys_are_independent() {
        let mut store = ChannelWebhookDedupeStore::new();
        assert!(!store.check_and_insert("key1"));
        assert!(!store.check_and_insert("key2"));
        assert!(store.check_and_insert("key1"));
    }

    #[test]
    fn evicts_oldest_at_capacity() {
        let mut store = ChannelWebhookDedupeStore {
            entries: HashMap::new(),
            ttl: std::time::Duration::from_secs(300),
            max_entries: 2,
        };
        assert!(!store.check_and_insert("a"));
        // Small sleep so "a" is strictly oldest
        std::thread::sleep(std::time::Duration::from_millis(1));
        assert!(!store.check_and_insert("b"));
        // At capacity — inserting "c" should evict "a"
        assert!(!store.check_and_insert("c"));
        assert!(!store.check_and_insert("a")); // "a" was evicted, so it's new again
        assert!(store.check_and_insert("c")); // "c" is still there
    }
}
