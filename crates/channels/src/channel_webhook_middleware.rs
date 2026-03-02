//! Channel webhook middleware: shared types for webhook security verification.
//!
//! Channel adapters that receive HTTP webhooks implement [`ChannelWebhookVerifier`]
//! to provide their signature algorithm. The gateway runs the shared pipeline
//! (`channel_webhook_gate`) which handles staleness, idempotency, and rate limiting.
//!
//! This is infrastructure middleware, NOT a user-facing webhook channel.

use std::time::Duration;

use {bytes::Bytes, http::HeaderMap};

use crate::plugin::ChannelType;

// ── Verifier trait ──────────────────────────────────────────────────────────

/// Per-channel webhook signature verification.
///
/// Each webhook channel (Slack, Teams, etc.) implements this trait to provide
/// its platform-specific signature algorithm. The gateway's middleware pipeline
/// calls [`verify`](Self::verify) on every inbound webhook request.
pub trait ChannelWebhookVerifier: Send + Sync {
    /// Verify the cryptographic signature of a webhook request.
    ///
    /// Implementors should:
    /// - Extract the signature from `headers`
    /// - Compute the expected signature from `body` using the channel's secret
    /// - Use constant-time comparison
    ///
    /// Returns a [`VerifiedChannelWebhook`] on success.
    fn verify(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<VerifiedChannelWebhook, ChannelWebhookRejection>;

    /// Maximum acceptable age of the request timestamp (default: 5 minutes).
    ///
    /// Requests older than this are rejected as stale. Slack recommends 5 minutes.
    fn max_timestamp_age(&self) -> Duration {
        Duration::from_secs(300)
    }

    /// Per-account rate policy for this channel (default: 60 req/min, burst 10).
    ///
    /// Channels with higher expected webhook volume can override this.
    fn rate_policy(&self) -> ChannelWebhookRatePolicy {
        ChannelWebhookRatePolicy::default()
    }

    /// The channel type this verifier handles. Used for metrics labels.
    fn channel_type(&self) -> ChannelType;
}

// ── Verified envelope ───────────────────────────────────────────────────────

/// Output of a successfully verified webhook request.
#[derive(Debug, Clone)]
pub struct VerifiedChannelWebhook {
    /// Channel-specific idempotency key (e.g. Slack `event_id`, Teams `activity.id`).
    /// `None` if the channel does not provide one.
    pub idempotency_key: Option<String>,

    /// Verified raw body bytes.
    pub body: Bytes,

    /// Request timestamp extracted from the provider's header (epoch seconds).
    /// `None` if the channel does not provide a timestamp header.
    pub timestamp_epoch: Option<i64>,
}

// ── Dedup result ────────────────────────────────────────────────────────────

/// Result of an idempotency check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelWebhookDedupeResult {
    /// First time seeing this key — process the request.
    New,
    /// Duplicate — return 200 OK without processing.
    Duplicate,
}

// ── Rejection ───────────────────────────────────────────────────────────────

/// Rejection reasons from the channel webhook middleware pipeline.
#[derive(Debug, Clone)]
pub enum ChannelWebhookRejection {
    /// Signature verification failed (bad HMAC, wrong secret, missing header).
    BadSignature(String),
    /// The request timestamp is too old or too far in the future.
    StaleTimestamp { age_seconds: u64, max_seconds: u64 },
    /// Required headers are missing.
    MissingHeaders(String),
    /// The request is a duplicate (idempotency key already seen).
    Duplicate,
    /// Per-account rate limit exceeded.
    RateLimited { retry_after: Duration },
}

impl ChannelWebhookRejection {
    /// Label value for the `rejection_reason` metric label.
    pub fn reason_label(&self) -> &'static str {
        match self {
            Self::BadSignature(_) => "bad_signature",
            Self::StaleTimestamp { .. } => "stale_timestamp",
            Self::MissingHeaders(_) => "missing_headers",
            Self::Duplicate => "duplicate",
            Self::RateLimited { .. } => "rate_limited",
        }
    }
}

impl std::fmt::Display for ChannelWebhookRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadSignature(msg) => write!(f, "bad signature: {msg}"),
            Self::StaleTimestamp {
                age_seconds,
                max_seconds,
            } => write!(
                f,
                "request timestamp too old ({age_seconds}s > {max_seconds}s max)"
            ),
            Self::MissingHeaders(header) => write!(f, "missing required header: {header}"),
            Self::Duplicate => write!(f, "duplicate request"),
            Self::RateLimited { retry_after } => {
                write!(f, "rate limited (retry after {}s)", retry_after.as_secs())
            },
        }
    }
}

// ── Timestamp guard ─────────────────────────────────────────────────────────

/// Checks whether a request timestamp is within the acceptable staleness window.
pub struct TimestampGuard;

impl TimestampGuard {
    /// Returns `Ok(())` if the timestamp is within `max_age` of now.
    ///
    /// If `timestamp_epoch` is `None`, the check passes (channel doesn't
    /// provide timestamps). Rejects both stale and future timestamps.
    pub fn check(
        timestamp_epoch: Option<i64>,
        max_age: Duration,
    ) -> Result<(), ChannelWebhookRejection> {
        let Some(ts) = timestamp_epoch else {
            return Ok(());
        };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let age = (now - ts).unsigned_abs();
        if age > max_age.as_secs() {
            return Err(ChannelWebhookRejection::StaleTimestamp {
                age_seconds: age,
                max_seconds: max_age.as_secs(),
            });
        }
        Ok(())
    }
}

// ── Rate policy ─────────────────────────────────────────────────────────────

/// Rate policy for per-(channel, account) webhook rate limiting.
#[derive(Debug, Clone, Copy)]
pub struct ChannelWebhookRatePolicy {
    /// Maximum sustained requests per minute for a single account.
    pub max_requests_per_minute: u32,
    /// Burst allowance above the sustained rate.
    pub burst: u32,
}

impl Default for ChannelWebhookRatePolicy {
    fn default() -> Self {
        Self {
            max_requests_per_minute: 60,
            burst: 10,
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn now_epoch() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    #[test]
    fn timestamp_guard_accepts_recent() {
        assert!(TimestampGuard::check(Some(now_epoch()), Duration::from_secs(300)).is_ok());
    }

    #[test]
    fn timestamp_guard_rejects_stale() {
        let old = now_epoch() - 600; // 10 minutes ago
        let result = TimestampGuard::check(Some(old), Duration::from_secs(300));
        assert!(matches!(
            result,
            Err(ChannelWebhookRejection::StaleTimestamp { .. })
        ));
    }

    #[test]
    fn timestamp_guard_passes_when_none() {
        assert!(TimestampGuard::check(None, Duration::from_secs(300)).is_ok());
    }

    #[test]
    fn timestamp_guard_boundary_exactly_at_max() {
        // At exactly max_age, age == max so it passes (> not >=).
        let ts = now_epoch() - 300;
        assert!(TimestampGuard::check(Some(ts), Duration::from_secs(300)).is_ok());
    }

    #[test]
    fn timestamp_guard_boundary_one_past() {
        let ts = now_epoch() - 301;
        assert!(matches!(
            TimestampGuard::check(Some(ts), Duration::from_secs(300)),
            Err(ChannelWebhookRejection::StaleTimestamp { .. })
        ));
    }

    #[test]
    fn timestamp_guard_rejects_future_timestamps() {
        let future = now_epoch() + 600; // 10 minutes in the future
        assert!(matches!(
            TimestampGuard::check(Some(future), Duration::from_secs(300)),
            Err(ChannelWebhookRejection::StaleTimestamp { .. })
        ));
    }

    #[test]
    fn default_rate_policy_is_sensible() {
        let policy = ChannelWebhookRatePolicy::default();
        assert_eq!(policy.max_requests_per_minute, 60);
        assert_eq!(policy.burst, 10);
    }

    #[test]
    fn rejection_reason_labels_are_stable() {
        assert_eq!(
            ChannelWebhookRejection::BadSignature("test".into()).reason_label(),
            "bad_signature"
        );
        assert_eq!(
            ChannelWebhookRejection::StaleTimestamp {
                age_seconds: 400,
                max_seconds: 300,
            }
            .reason_label(),
            "stale_timestamp"
        );
        assert_eq!(
            ChannelWebhookRejection::MissingHeaders("x-sig".into()).reason_label(),
            "missing_headers"
        );
        assert_eq!(
            ChannelWebhookRejection::Duplicate.reason_label(),
            "duplicate"
        );
        assert_eq!(
            ChannelWebhookRejection::RateLimited {
                retry_after: Duration::from_secs(30),
            }
            .reason_label(),
            "rate_limited"
        );
    }
}
