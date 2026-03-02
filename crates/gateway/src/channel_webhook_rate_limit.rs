//! Per-(channel, account) token-bucket rate limiter for channel webhooks.
//!
//! Each `(channel_type, account_id)` pair gets its own bucket. Buckets are
//! lazily created on first request and automatically evicted when stale.

use std::time::{Duration, Instant};

use dashmap::DashMap;

use moltis_channels::channel_webhook_middleware::{
    ChannelWebhookRatePolicy, ChannelWebhookRejection,
};

/// Composite key for per-(channel, account) rate limiting.
type BucketKey = String;

fn bucket_key(channel: &str, account_id: &str) -> BucketKey {
    format!("{channel}:{account_id}")
}

/// Token-bucket state for a single (channel, account) pair.
struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

/// Concurrent rate limiter using per-account token buckets.
///
/// Thread-safe: uses [`DashMap`] for lock-free concurrent access.
pub struct ChannelWebhookRateLimiter {
    buckets: DashMap<BucketKey, Bucket>,
    /// Maximum number of tracked buckets before eviction.
    max_buckets: usize,
}

impl ChannelWebhookRateLimiter {
    pub fn new() -> Self {
        Self {
            buckets: DashMap::new(),
            max_buckets: 10_000,
        }
    }

    /// Check the rate limit for a (channel, account) pair.
    ///
    /// Returns `Ok(())` if the request is allowed, or
    /// `Err(ChannelWebhookRejection::RateLimited { .. })` if the bucket is exhausted.
    pub fn check(
        &self,
        channel_type: &str,
        account_id: &str,
        policy: &ChannelWebhookRatePolicy,
    ) -> Result<(), ChannelWebhookRejection> {
        let key = bucket_key(channel_type, account_id);
        let rate_per_sec = f64::from(policy.max_requests_per_minute) / 60.0;
        let capacity = f64::from(policy.max_requests_per_minute + policy.burst);

        let mut entry = self.buckets.entry(key).or_insert_with(|| Bucket {
            tokens: capacity,
            last_refill: Instant::now(),
        });

        let bucket = entry.value_mut();

        // Refill tokens based on elapsed time.
        let now = Instant::now();
        let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * rate_per_sec).min(capacity);
        bucket.last_refill = now;

        // Try to consume one token.
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            // Time until one token is available.
            let deficit = 1.0 - bucket.tokens;
            let wait_secs = deficit / rate_per_sec;
            Err(ChannelWebhookRejection::RateLimited {
                retry_after: Duration::from_secs_f64(wait_secs),
            })
        }
    }

    /// Remove stale buckets that haven't been used recently.
    ///
    /// Call this periodically (e.g. every 5 minutes) from a maintenance timer.
    pub fn evict_stale(&self, max_idle: Duration) {
        let cutoff = Instant::now() - max_idle;
        self.buckets.retain(|_, bucket| bucket.last_refill > cutoff);
    }

    /// Number of tracked buckets (for diagnostics).
    pub fn bucket_count(&self) -> usize {
        self.buckets.len()
    }

    /// Hard eviction when bucket count exceeds limit.
    /// Removes the oldest half of buckets.
    pub fn evict_if_full(&self) {
        if self.buckets.len() <= self.max_buckets {
            return;
        }
        // Collect keys with their last_refill times, sort by oldest, remove half.
        let mut entries: Vec<(BucketKey, Instant)> = self
            .buckets
            .iter()
            .map(|e| (e.key().clone(), e.value().last_refill))
            .collect();
        entries.sort_by_key(|(_, t)| *t);
        let remove_count = entries.len() / 2;
        for (key, _) in entries.into_iter().take(remove_count) {
            self.buckets.remove(&key);
        }
    }
}

impl Default for ChannelWebhookRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn default_policy() -> ChannelWebhookRatePolicy {
        ChannelWebhookRatePolicy {
            max_requests_per_minute: 60,
            burst: 10,
        }
    }

    #[test]
    fn allows_requests_within_rate() {
        let limiter = ChannelWebhookRateLimiter::new();
        let policy = default_policy();

        // First request should always be allowed (bucket starts full).
        assert!(limiter.check("slack", "acct1", &policy).is_ok());
    }

    #[test]
    fn rejects_after_burst_exhausted() {
        let limiter = ChannelWebhookRateLimiter::new();
        let policy = ChannelWebhookRatePolicy {
            max_requests_per_minute: 6, // 0.1 per second
            burst: 2,                   // capacity = 8
        };

        // Exhaust the full capacity (rate + burst = 8 tokens).
        for _ in 0..8 {
            assert!(limiter.check("slack", "acct1", &policy).is_ok());
        }

        // Next request should be rejected.
        let result = limiter.check("slack", "acct1", &policy);
        assert!(matches!(
            result,
            Err(ChannelWebhookRejection::RateLimited { .. })
        ));
    }

    #[test]
    fn separate_accounts_have_separate_buckets() {
        let limiter = ChannelWebhookRateLimiter::new();
        let policy = ChannelWebhookRatePolicy {
            max_requests_per_minute: 6,
            burst: 0, // capacity = 6
        };

        // Exhaust account 1.
        for _ in 0..6 {
            assert!(limiter.check("slack", "acct1", &policy).is_ok());
        }
        assert!(limiter.check("slack", "acct1", &policy).is_err());

        // Account 2 should still be fine.
        assert!(limiter.check("slack", "acct2", &policy).is_ok());
    }

    #[test]
    fn separate_channels_have_separate_buckets() {
        let limiter = ChannelWebhookRateLimiter::new();
        let policy = ChannelWebhookRatePolicy {
            max_requests_per_minute: 6,
            burst: 0,
        };

        for _ in 0..6 {
            assert!(limiter.check("slack", "acct1", &policy).is_ok());
        }
        assert!(limiter.check("slack", "acct1", &policy).is_err());

        // Same account, different channel — separate bucket.
        assert!(limiter.check("msteams", "acct1", &policy).is_ok());
    }

    #[test]
    fn evict_stale_removes_old_buckets() {
        let limiter = ChannelWebhookRateLimiter::new();
        let policy = default_policy();

        limiter.check("slack", "acct1", &policy).ok();
        assert_eq!(limiter.bucket_count(), 1);

        // Evict with zero max_idle removes everything.
        limiter.evict_stale(Duration::ZERO);
        assert_eq!(limiter.bucket_count(), 0);
    }

    #[test]
    fn retry_after_is_positive() {
        let limiter = ChannelWebhookRateLimiter::new();
        let policy = ChannelWebhookRatePolicy {
            max_requests_per_minute: 6,
            burst: 0,
        };

        for _ in 0..6 {
            limiter.check("slack", "acct1", &policy).ok();
        }

        if let Err(ChannelWebhookRejection::RateLimited { retry_after }) =
            limiter.check("slack", "acct1", &policy)
        {
            assert!(retry_after.as_secs_f64() > 0.0);
        } else {
            panic!("expected RateLimited");
        }
    }
}
