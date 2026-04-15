//! Brute-force protection for authentication endpoints.
//!
//! Provides two complementary defences:
//!
//! 1. **IP ban** — after [`IP_BAN_THRESHOLD`] consecutive failures from a single
//!    IP address the IP is banned with exponential back-off
//!    (5 min → 15 min → 1 hour → 24 hours, capped).
//!
//! 2. **Account lockout** — after [`ACCOUNT_LOCKOUT_IP_THRESHOLD`] *distinct* IPs
//!    fail for the same account within [`ACCOUNT_LOCKOUT_WINDOW`] the account is
//!    locked for [`ACCOUNT_LOCKOUT_DURATION`].
//!
//! Both layers are in-memory ([`DashMap`]) and cleaned up periodically.

use std::{
    collections::HashSet,
    net::IpAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use dashmap::DashMap;

// ── Tuning constants ─────────────────────────────────────────────────────────

/// Consecutive failures from a single IP before it is banned.
const IP_BAN_THRESHOLD: u32 = 10;

/// Base ban duration (first offence).
const IP_BAN_BASE: Duration = Duration::from_secs(5 * 60); // 5 minutes

/// Maximum ban duration after repeated escalations.
const IP_BAN_CAP: Duration = Duration::from_secs(24 * 60 * 60); // 24 hours

/// Number of distinct IPs that must fail for the same account
/// within [`ACCOUNT_LOCKOUT_WINDOW`] to trigger a lockout.
const ACCOUNT_LOCKOUT_IP_THRESHOLD: usize = 3;

/// Sliding window for counting distinct IPs per account.
const ACCOUNT_LOCKOUT_WINDOW: Duration = Duration::from_secs(15 * 60); // 15 minutes

/// How long a locked-out account stays locked.
const ACCOUNT_LOCKOUT_DURATION: Duration = Duration::from_secs(15 * 60); // 15 minutes

/// Run cleanup every N `record_failure` calls.
const CLEANUP_EVERY: u64 = 128;

// ── Per-IP state ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct IpRecord {
    /// Consecutive failures (reset on success).
    consecutive_failures: u32,
    /// Number of bans this IP has accumulated (drives escalation).
    ban_count: u32,
    /// If banned, when the ban expires.
    banned_until: Option<Instant>,
    /// When the last failure was recorded (for staleness cleanup).
    last_failure_at: Instant,
}

// ── Per-account state ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct AccountRecord {
    /// `(ip, first_failure_time)` pairs within the window.
    recent_ips: Vec<(IpAddr, Instant)>,
    /// If locked, when the lockout expires.
    locked_until: Option<Instant>,
}

// ── Why a login attempt was rejected ─────────────────────────────────────────

/// Reason a login attempt was blocked before credentials were checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    /// The source IP is temporarily banned.
    IpBanned { retry_after: Duration },
    /// The target account is temporarily locked.
    AccountLocked { retry_after: Duration },
}

// ── LoginGuard ───────────────────────────────────────────────────────────────

/// Thread-safe, in-memory brute-force guard.
///
/// Call [`check`] before verifying credentials and [`record_failure`] /
/// [`record_success`] after the outcome is known.
#[derive(Clone)]
pub struct LoginGuard {
    ips: Arc<DashMap<IpAddr, IpRecord>>,
    /// Keyed by "account identifier" — for password auth this is a fixed
    /// sentinel (`"__password__"`) because moltis has a single admin account.
    accounts: Arc<DashMap<String, AccountRecord>>,
    calls: Arc<AtomicU64>,
}

impl LoginGuard {
    #[must_use]
    pub fn new() -> Self {
        Self {
            ips: Arc::new(DashMap::new()),
            accounts: Arc::new(DashMap::new()),
            calls: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Check whether the request should be blocked *before* credential
    /// verification.  Returns `None` if the attempt is allowed.
    pub fn check(&self, ip: IpAddr, account: &str) -> Option<BlockReason> {
        self.check_at(ip, account, Instant::now())
    }

    fn check_at(&self, ip: IpAddr, account: &str, now: Instant) -> Option<BlockReason> {
        // Check IP ban first.
        if let Some(rec) = self.ips.get(&ip)
            && let Some(until) = rec.banned_until
            && now < until
        {
            return Some(BlockReason::IpBanned {
                retry_after: until.duration_since(now),
            });
        }

        // Check account lockout.
        if let Some(rec) = self.accounts.get(account)
            && let Some(until) = rec.locked_until
            && now < until
        {
            return Some(BlockReason::AccountLocked {
                retry_after: until.duration_since(now),
            });
        }

        None
    }

    /// Record a failed authentication attempt.
    pub fn record_failure(&self, ip: IpAddr, account: &str) {
        self.record_failure_at(ip, account, Instant::now());
    }

    fn record_failure_at(&self, ip: IpAddr, account: &str, now: Instant) {
        // ── Update per-IP record ─────────────────────────────────────────
        let mut ip_entry = self.ips.entry(ip).or_insert_with(|| IpRecord {
            consecutive_failures: 0,
            ban_count: 0,
            banned_until: None,
            last_failure_at: now,
        });

        ip_entry.consecutive_failures += 1;
        ip_entry.last_failure_at = now;

        if ip_entry.consecutive_failures >= IP_BAN_THRESHOLD {
            ip_entry.ban_count += 1;
            let multiplier = 1u32 << ip_entry.ban_count.saturating_sub(1).min(10);
            let ban_duration = (IP_BAN_BASE * multiplier).min(IP_BAN_CAP);
            ip_entry.banned_until = Some(now + ban_duration);
            // Reset counter so the next window starts fresh after the ban.
            ip_entry.consecutive_failures = 0;
        }
        drop(ip_entry);

        // ── Update per-account record ────────────────────────────────────
        let mut acct_entry =
            self.accounts
                .entry(account.to_owned())
                .or_insert_with(|| AccountRecord {
                    recent_ips: Vec::new(),
                    locked_until: None,
                });

        // Prune IPs outside the window.
        acct_entry
            .recent_ips
            .retain(|&(_, t)| now.duration_since(t) < ACCOUNT_LOCKOUT_WINDOW);

        // Add this IP if not already tracked in the current window.
        if !acct_entry
            .recent_ips
            .iter()
            .any(|&(recorded_ip, _)| recorded_ip == ip)
        {
            acct_entry.recent_ips.push((ip, now));
        }

        let distinct_ips: HashSet<IpAddr> =
            acct_entry.recent_ips.iter().map(|&(ip, _)| ip).collect();
        if distinct_ips.len() >= ACCOUNT_LOCKOUT_IP_THRESHOLD {
            acct_entry.locked_until = Some(now + ACCOUNT_LOCKOUT_DURATION);
            acct_entry.recent_ips.clear();
        }
        drop(acct_entry);

        // Periodic cleanup.
        let calls = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
        if calls.is_multiple_of(CLEANUP_EVERY) {
            self.cleanup(now);
        }
    }

    /// Record a successful authentication — clears IP failure counter.
    pub fn record_success(&self, ip: IpAddr) {
        if let Some(mut rec) = self.ips.get_mut(&ip) {
            rec.consecutive_failures = 0;
            // Clear the ban if it's expired (or let it ride if still active —
            // a successful login means the request wasn't blocked, so the ban
            // must have already expired).
            rec.banned_until = None;
        }
    }

    fn cleanup(&self, now: Instant) {
        // Remove expired IP records. Keep if:
        // - actively banned, OR
        // - has recent failures (within the lockout window).
        self.ips.retain(|_, rec| {
            if let Some(until) = rec.banned_until
                && now < until
            {
                return true;
            }
            // Keep if there were recent failures (could still contribute
            // to a ban if more arrive soon).
            rec.consecutive_failures > 0
                && now.duration_since(rec.last_failure_at) < ACCOUNT_LOCKOUT_WINDOW
        });

        // Remove expired account lockouts with no recent IPs.
        self.accounts.retain(|_, rec| {
            rec.recent_ips
                .retain(|&(_, t)| now.duration_since(t) < ACCOUNT_LOCKOUT_WINDOW);
            if !rec.recent_ips.is_empty() {
                return true;
            }
            match rec.locked_until {
                Some(until) => now < until,
                None => false,
            }
        });
    }
}

impl Default for LoginGuard {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    fn ip(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, last))
    }

    #[test]
    fn allows_initial_attempt() {
        let guard = LoginGuard::new();
        assert!(guard.check(ip(1), "admin").is_none());
    }

    #[test]
    fn bans_ip_after_threshold() {
        let guard = LoginGuard::new();
        let now = Instant::now();

        // 10 failures should trigger ban.
        for i in 0..IP_BAN_THRESHOLD {
            assert!(
                guard.check_at(ip(1), "admin", now).is_none(),
                "attempt {i} should be allowed"
            );
            guard.record_failure_at(ip(1), "admin", now);
        }

        // Next attempt is blocked.
        let block = guard.check_at(ip(1), "admin", now);
        assert!(
            matches!(block, Some(BlockReason::IpBanned { .. })),
            "expected IP ban, got {block:?}"
        );

        // A different IP is not affected.
        assert!(guard.check_at(ip(2), "admin", now).is_none());
    }

    #[test]
    fn ip_ban_expires() {
        let guard = LoginGuard::new();
        let now = Instant::now();

        for _ in 0..IP_BAN_THRESHOLD {
            guard.record_failure_at(ip(1), "admin", now);
        }
        assert!(guard.check_at(ip(1), "admin", now).is_some());

        // After ban duration + 1 second, should be allowed again.
        let after = now + IP_BAN_BASE + Duration::from_secs(1);
        assert!(guard.check_at(ip(1), "admin", after).is_none());
    }

    #[test]
    fn ip_ban_escalates() {
        let guard = LoginGuard::new();
        let mut now = Instant::now();

        // First ban: 5 minutes.
        for _ in 0..IP_BAN_THRESHOLD {
            guard.record_failure_at(ip(1), "admin", now);
        }
        if let Some(BlockReason::IpBanned { retry_after }) = guard.check_at(ip(1), "admin", now) {
            assert!(
                retry_after <= IP_BAN_BASE,
                "first ban should be ~5min, got {retry_after:?}"
            );
        } else {
            panic!("expected ban");
        }

        // Advance past first ban.
        now += IP_BAN_BASE + Duration::from_secs(1);

        // Second ban: 10 minutes.
        for _ in 0..IP_BAN_THRESHOLD {
            guard.record_failure_at(ip(1), "admin", now);
        }
        if let Some(BlockReason::IpBanned { retry_after }) = guard.check_at(ip(1), "admin", now) {
            assert!(
                retry_after > IP_BAN_BASE,
                "second ban should be escalated, got {retry_after:?}"
            );
        } else {
            panic!("expected escalated ban");
        }
    }

    #[test]
    fn success_clears_ip_state() {
        let guard = LoginGuard::new();
        let now = Instant::now();

        for _ in 0..(IP_BAN_THRESHOLD - 1) {
            guard.record_failure_at(ip(1), "admin", now);
        }

        // One more failure would ban — but success resets.
        guard.record_success(ip(1));
        guard.record_failure_at(ip(1), "admin", now);
        assert!(
            guard.check_at(ip(1), "admin", now).is_none(),
            "counter should have reset on success"
        );
    }

    #[test]
    fn account_locks_after_distinct_ips() {
        let guard = LoginGuard::new();
        let now = Instant::now();

        // Failures from 3 distinct IPs targeting the same account.
        guard.record_failure_at(ip(1), "admin", now);
        guard.record_failure_at(ip(2), "admin", now);
        assert!(
            guard.check_at(ip(4), "admin", now).is_none(),
            "should not be locked after 2 IPs"
        );

        guard.record_failure_at(ip(3), "admin", now);

        // Account is now locked — even a new IP is blocked.
        let block = guard.check_at(ip(4), "admin", now);
        assert!(
            matches!(block, Some(BlockReason::AccountLocked { .. })),
            "expected account lockout, got {block:?}"
        );
    }

    #[test]
    fn account_lockout_expires() {
        let guard = LoginGuard::new();
        let now = Instant::now();

        guard.record_failure_at(ip(1), "admin", now);
        guard.record_failure_at(ip(2), "admin", now);
        guard.record_failure_at(ip(3), "admin", now);

        assert!(guard.check_at(ip(4), "admin", now).is_some());

        let after = now + ACCOUNT_LOCKOUT_DURATION + Duration::from_secs(1);
        assert!(guard.check_at(ip(4), "admin", after).is_none());
    }

    #[test]
    fn account_lockout_requires_ips_within_window() {
        let guard = LoginGuard::new();
        let now = Instant::now();

        // IP 1 fails at t=0.
        guard.record_failure_at(ip(1), "admin", now);

        // IP 2 fails after the window expires — IP 1 should be pruned.
        let later = now + ACCOUNT_LOCKOUT_WINDOW + Duration::from_secs(1);
        guard.record_failure_at(ip(2), "admin", later);
        guard.record_failure_at(ip(3), "admin", later);

        // Only 2 IPs in window (ip(1) expired), so no lockout.
        assert!(guard.check_at(ip(4), "admin", later).is_none());
    }

    #[test]
    fn ip_ban_checked_before_account_lockout() {
        let guard = LoginGuard::new();
        let now = Instant::now();

        // Ban IP first.
        for _ in 0..IP_BAN_THRESHOLD {
            guard.record_failure_at(ip(1), "admin", now);
        }

        let block = guard.check_at(ip(1), "admin", now);
        assert!(
            matches!(block, Some(BlockReason::IpBanned { .. })),
            "IP ban should take precedence"
        );
    }

    #[test]
    fn cleanup_removes_stale_entries() {
        let guard = LoginGuard::new();
        let now = Instant::now();

        guard.record_failure_at(ip(1), "admin", now);

        // Well past all windows and bans.
        let far_future = now + Duration::from_secs(100_000);
        guard.cleanup(far_future);

        assert!(guard.ips.is_empty());
        assert!(guard.accounts.is_empty());
    }
}
