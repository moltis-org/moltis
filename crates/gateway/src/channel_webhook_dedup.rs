//! Channel webhook idempotency deduplication store.
//!
//! Provides TTL-based deduplication for channel webhook requests keyed by
//! provider-specific message IDs (e.g. Slack `event_id`, Teams `activity.id`).
//!
//! Extracted from the former `DedupeCache` in `state.rs`.

use std::{collections::HashMap, time::Instant};

/// Slack documents callback throughput at roughly 2,500 deliveries per account
/// during the five-minute retry window. Keep that full window independently for
/// every account and endpoint.
const MAX_ENTRIES_PER_SCOPE: usize = 2_500;

struct DedupeEntry {
    inserted_at: Instant,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct DedupeScope {
    channel: String,
    account_id: String,
    endpoint: String,
}

impl DedupeScope {
    fn new(channel: &str, account_id: &str, endpoint: &str) -> Self {
        Self {
            channel: channel.to_owned(),
            account_id: account_id.to_owned(),
            endpoint: endpoint.to_owned(),
        }
    }

    fn unscoped() -> Self {
        Self::new("", "", "")
    }
}

/// Result of atomically admitting work and committing its idempotency key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelWebhookAdmission {
    Duplicate,
    Admitted,
    Rejected,
}

/// TTL-based idempotency store for channel webhook deduplication.
///
/// Tracks provider-specific message IDs to detect duplicate deliveries.
/// Entries expire after `ttl`. Each `(channel, account, endpoint)` partition
/// has its own capacity so traffic in one partition cannot evict another.
pub struct ChannelWebhookDedupeStore {
    partitions: HashMap<DedupeScope, HashMap<String, DedupeEntry>>,
    ttl: std::time::Duration,
    max_entries_per_scope: usize,
}

impl Default for ChannelWebhookDedupeStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelWebhookDedupeStore {
    pub fn new() -> Self {
        Self {
            partitions: HashMap::new(),
            ttl: std::time::Duration::from_millis(moltis_protocol::DEDUPE_TTL_MS),
            max_entries_per_scope: MAX_ENTRIES_PER_SCOPE,
        }
    }

    /// Returns `true` if the key is a duplicate (already seen within TTL).
    /// If new, inserts the key and returns `false`.
    pub fn check_and_insert(&mut self, key: &str) -> bool {
        self.evict_expired();
        let scope = DedupeScope::unscoped();
        if self
            .partitions
            .get(&scope)
            .is_some_and(|entries| entries.contains_key(key))
        {
            return true;
        }
        self.insert_new(scope, key.to_owned());
        false
    }

    /// Check a provider key within a channel account and callback endpoint.
    pub fn check_and_insert_scoped(
        &mut self,
        channel: &str,
        account_id: &str,
        endpoint: &str,
        provider_key: &str,
    ) -> bool {
        self.evict_expired();
        let scope = DedupeScope::new(channel, account_id, endpoint);
        if self
            .partitions
            .get(&scope)
            .is_some_and(|entries| entries.contains_key(provider_key))
        {
            return true;
        }
        self.insert_new(scope, provider_key.to_owned());
        false
    }

    /// Record the idempotency key only when bounded work admission succeeds.
    pub fn admit_scoped(
        &mut self,
        channel: &str,
        account_id: &str,
        endpoint: &str,
        provider_key: &str,
        admit: impl FnOnce() -> bool,
    ) -> ChannelWebhookAdmission {
        self.evict_expired();
        let scope = DedupeScope::new(channel, account_id, endpoint);
        if self
            .partitions
            .get(&scope)
            .is_some_and(|entries| entries.contains_key(provider_key))
        {
            return ChannelWebhookAdmission::Duplicate;
        }
        if !admit() {
            return ChannelWebhookAdmission::Rejected;
        }
        self.insert_new(scope, provider_key.to_owned());
        ChannelWebhookAdmission::Admitted
    }

    fn insert_new(&mut self, scope: DedupeScope, key: String) {
        let entries = self.partitions.entry(scope).or_default();
        if entries.len() >= self.max_entries_per_scope
            && let Some(oldest_key) = entries
                .iter()
                .min_by_key(|(_, entry)| entry.inserted_at)
                .map(|(key, _)| key.clone())
        {
            entries.remove(&oldest_key);
        }
        entries.insert(key, DedupeEntry {
            inserted_at: Instant::now(),
        });
    }

    fn evict_expired(&mut self) {
        let cutoff = Instant::now() - self.ttl;
        self.partitions.retain(|_, entries| {
            entries.retain(|_, entry| entry.inserted_at > cutoff);
            !entries.is_empty()
        });
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
    fn scoped_keys_are_isolated_by_account_and_endpoint() {
        let mut store = ChannelWebhookDedupeStore::new();
        assert!(!store.check_and_insert_scoped("slack", "a", "events", "trigger"));
        assert!(store.check_and_insert_scoped("slack", "a", "events", "trigger"));
        assert!(!store.check_and_insert_scoped("slack", "b", "events", "trigger"));
        assert!(!store.check_and_insert_scoped("slack", "a", "commands", "trigger"));
    }

    #[test]
    fn scoped_keys_are_unambiguous() {
        let mut store = ChannelWebhookDedupeStore::new();
        assert!(!store.check_and_insert_scoped("slack", "a:events", "commands", "id"));
        assert!(!store.check_and_insert_scoped("slack", "a", "events:commands", "id"));
    }

    #[test]
    fn rejected_admission_does_not_consume_dedup_key() {
        let mut store = ChannelWebhookDedupeStore::new();
        assert_eq!(
            store.admit_scoped("slack", "a", "events", "event-1", || false),
            ChannelWebhookAdmission::Rejected
        );
        assert_eq!(
            store.admit_scoped("slack", "a", "events", "event-1", || true),
            ChannelWebhookAdmission::Admitted
        );
        assert_eq!(
            store.admit_scoped("slack", "a", "events", "event-1", || false),
            ChannelWebhookAdmission::Duplicate
        );
    }

    #[test]
    fn evicts_oldest_at_capacity() {
        let mut store = ChannelWebhookDedupeStore {
            partitions: HashMap::new(),
            ttl: std::time::Duration::from_secs(300),
            max_entries_per_scope: 2,
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

    #[test]
    fn retains_retry_window_capacity_independently_per_account_and_endpoint() {
        let mut store = ChannelWebhookDedupeStore::new();

        for index in 0..MAX_ENTRIES_PER_SCOPE {
            let key = format!("account-a-event-{index}");
            assert!(!store.check_and_insert_scoped("slack", "a", "events", &key));
        }
        for index in 0..MAX_ENTRIES_PER_SCOPE {
            let key = format!("account-b-event-{index}");
            assert!(!store.check_and_insert_scoped("slack", "b", "events", &key));
        }
        for index in 0..MAX_ENTRIES_PER_SCOPE {
            let key = format!("account-a-command-{index}");
            assert!(!store.check_and_insert_scoped("slack", "a", "commands", &key));
        }

        assert!(store.check_and_insert_scoped("slack", "a", "events", "account-a-event-0"));
        assert!(store.check_and_insert_scoped("slack", "b", "events", "account-b-event-0"));
        assert!(store.check_and_insert_scoped("slack", "a", "commands", "account-a-command-0"));
    }
}
