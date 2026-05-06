//! Per-webhook and global rate limiting.

use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
};

/// Combined state for per-webhook and global sliding windows.
struct RateLimitState {
    per_webhook: HashMap<i64, VecDeque<u64>>,
    global: VecDeque<u64>,
}

/// Per-webhook sliding-window rate limiter.
///
/// Uses a single lock to avoid TOCTOU gaps between the global and
/// per-webhook checks.
pub struct WebhookRateLimiter {
    state: Mutex<RateLimitState>,
    global_max: u32,
}

impl WebhookRateLimiter {
    /// Create a new rate limiter with a global max requests per minute.
    pub fn new(global_max: u32) -> Self {
        Self {
            state: Mutex::new(RateLimitState {
                per_webhook: HashMap::new(),
                global: VecDeque::new(),
            }),
            global_max,
        }
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    /// Check if a request is allowed for the given webhook.
    /// Returns `true` if allowed, `false` if rate limited.
    pub fn check(&self, webhook_id: i64, per_webhook_max: u32) -> bool {
        let now = Self::now_ms();
        let window_start = now.saturating_sub(60_000);

        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());

        // Prune and check global limit.
        while state.global.front().is_some_and(|&t| t < window_start) {
            state.global.pop_front();
        }
        if state.global.len() >= self.global_max as usize {
            return false;
        }

        // Prune and check per-webhook limit.
        let window = state.per_webhook.entry(webhook_id).or_default();
        while window.front().is_some_and(|&t| t < window_start) {
            window.pop_front();
        }
        if window.len() >= per_webhook_max as usize {
            return false;
        }

        // Record in both windows atomically.
        window.push_back(now);
        state.global.push_back(now);

        true
    }
}

impl Default for WebhookRateLimiter {
    fn default() -> Self {
        Self::new(300) // 300 global requests per minute
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_rate_limit() {
        let limiter = WebhookRateLimiter::new(100);
        // Should allow requests up to per-webhook max
        for _ in 0..5 {
            assert!(limiter.check(1, 5));
        }
        // 6th request should be rate limited
        assert!(!limiter.check(1, 5));
        // Different webhook should still be allowed
        assert!(limiter.check(2, 5));
    }

    #[test]
    fn test_global_limit() {
        let limiter = WebhookRateLimiter::new(3);
        assert!(limiter.check(1, 10));
        assert!(limiter.check(2, 10));
        assert!(limiter.check(3, 10));
        // 4th request should hit global limit
        assert!(!limiter.check(4, 10));
    }
}
