//! Channel webhook middleware pipeline.
//!
//! The [`channel_webhook_gate`] function is the single entry point for all
//! webhook-based channel routes. It runs:
//!
//! 1. Signature verification (via the channel's [`ChannelWebhookVerifier`])
//! 2. Timestamp staleness rejection
//! 3. Per-(channel, account) rate limiting
//! 4. Idempotency deduplication

use http::HeaderMap;

use moltis_channels::channel_webhook_middleware::{
    ChannelWebhookDedupeResult, ChannelWebhookRejection, ChannelWebhookVerifier, TimestampGuard,
    VerifiedChannelWebhook,
};

use crate::{
    channel_webhook_dedup::ChannelWebhookDedupeStore,
    channel_webhook_rate_limit::ChannelWebhookRateLimiter,
};

/// Run the full channel webhook verification pipeline.
///
/// Steps:
/// 1. Verify the cryptographic signature via the channel's verifier.
/// 2. Check the timestamp is within the acceptable staleness window.
/// 3. Apply per-(channel, account) rate limiting.
/// 4. Deduplicate by provider message ID (if present).
///
/// On success returns the verified envelope and the dedup result.
/// On failure returns a rejection that transport layers can map to responses.
pub fn channel_webhook_gate(
    verifier: &dyn ChannelWebhookVerifier,
    dedup_store: &std::sync::RwLock<ChannelWebhookDedupeStore>,
    rate_limiter: &ChannelWebhookRateLimiter,
    account_id: &str,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<(VerifiedChannelWebhook, ChannelWebhookDedupeResult), ChannelWebhookRejection> {
    #[cfg(feature = "metrics")]
    let start = std::time::Instant::now();

    #[cfg(feature = "metrics")]
    {
        use moltis_metrics::{counter, labels};
        counter!(
            moltis_metrics::channel_webhook::REQUESTS_TOTAL,
            labels::CHANNEL => verifier.channel_type().as_str()
        )
        .increment(1);
    }

    // Step 1: Signature verification.
    let envelope = match verifier.verify(headers, body) {
        Ok(env) => {
            #[cfg(feature = "metrics")]
            {
                use moltis_metrics::{counter, histogram, labels};
                counter!(
                    moltis_metrics::channel_webhook::VERIFIED_TOTAL,
                    labels::CHANNEL => verifier.channel_type().as_str()
                )
                .increment(1);
                histogram!(
                    moltis_metrics::channel_webhook::VERIFY_DURATION_SECONDS,
                    labels::CHANNEL => verifier.channel_type().as_str()
                )
                .record(start.elapsed().as_secs_f64());
            }
            env
        },
        Err(rejection) => {
            #[cfg(feature = "metrics")]
            {
                use moltis_metrics::{counter, labels};
                counter!(
                    moltis_metrics::channel_webhook::REJECTED_TOTAL,
                    labels::CHANNEL => verifier.channel_type().as_str(),
                    labels::REJECTION_REASON => rejection.reason_label()
                )
                .increment(1);
            }
            return Err(rejection);
        },
    };

    // Step 2: Timestamp staleness check.
    if let Err(rejection) =
        TimestampGuard::check(envelope.timestamp_epoch, verifier.max_timestamp_age())
    {
        #[cfg(feature = "metrics")]
        {
            use moltis_metrics::{counter, labels};
            counter!(
                moltis_metrics::channel_webhook::REJECTED_TOTAL,
                labels::CHANNEL => verifier.channel_type().as_str(),
                labels::REJECTION_REASON => rejection.reason_label()
            )
            .increment(1);
        }
        return Err(rejection);
    }

    // Step 3: Per-(channel, account) rate limiting.
    if let Err(rejection) = rate_limiter.check(
        verifier.channel_type().as_str(),
        account_id,
        &verifier.rate_policy(),
    ) {
        #[cfg(feature = "metrics")]
        {
            use moltis_metrics::{counter, labels};
            counter!(
                moltis_metrics::channel_webhook::RATE_LIMITED_TOTAL,
                labels::CHANNEL => verifier.channel_type().as_str()
            )
            .increment(1);
        }
        return Err(rejection);
    }

    // Step 4: Idempotency deduplication.
    let dedup_result = if let Some(ref key) = envelope.idempotency_key {
        let mut store = dedup_store.write().unwrap_or_else(|e| e.into_inner());
        if store.check_and_insert(key) {
            #[cfg(feature = "metrics")]
            {
                use moltis_metrics::{counter, labels};
                counter!(
                    moltis_metrics::channel_webhook::DEDUPED_TOTAL,
                    labels::CHANNEL => verifier.channel_type().as_str()
                )
                .increment(1);
            }
            ChannelWebhookDedupeResult::Duplicate
        } else {
            ChannelWebhookDedupeResult::New
        }
    } else {
        ChannelWebhookDedupeResult::New
    };

    Ok((envelope, dedup_result))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use {super::*, bytes::Bytes, moltis_channels::plugin::ChannelType};

    /// Dummy verifier that always passes for testing the pipeline.
    struct PassVerifier;

    impl ChannelWebhookVerifier for PassVerifier {
        fn verify(
            &self,
            _headers: &HeaderMap,
            body: &[u8],
        ) -> Result<VerifiedChannelWebhook, ChannelWebhookRejection> {
            let idempotency_key = serde_json::from_slice::<serde_json::Value>(body)
                .ok()
                .and_then(|v| v.get("id").and_then(|e| e.as_str()).map(String::from));

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;

            Ok(VerifiedChannelWebhook {
                idempotency_key,
                body: Bytes::copy_from_slice(body),
                timestamp_epoch: Some(now),
            })
        }

        fn channel_type(&self) -> ChannelType {
            ChannelType::Slack
        }
    }

    /// Dummy verifier that always rejects.
    struct FailVerifier;

    impl ChannelWebhookVerifier for FailVerifier {
        fn verify(
            &self,
            _headers: &HeaderMap,
            _body: &[u8],
        ) -> Result<VerifiedChannelWebhook, ChannelWebhookRejection> {
            Err(ChannelWebhookRejection::BadSignature("test failure".into()))
        }

        fn channel_type(&self) -> ChannelType {
            ChannelType::Slack
        }
    }

    fn make_store() -> std::sync::RwLock<ChannelWebhookDedupeStore> {
        std::sync::RwLock::new(ChannelWebhookDedupeStore::new())
    }

    fn make_limiter() -> ChannelWebhookRateLimiter {
        ChannelWebhookRateLimiter::new()
    }

    #[test]
    fn gate_passes_valid_request() {
        let store = make_store();
        let limiter = make_limiter();
        let body = br#"{"id":"ev1","text":"hello"}"#;
        let result = channel_webhook_gate(
            &PassVerifier,
            &store,
            &limiter,
            "acct1",
            &HeaderMap::new(),
            body,
        );
        assert!(result.is_ok());
        let (env, dedup) = result.unwrap();
        assert_eq!(env.idempotency_key.as_deref(), Some("ev1"));
        assert_eq!(dedup, ChannelWebhookDedupeResult::New);
    }

    #[test]
    fn gate_rejects_bad_signature() {
        let store = make_store();
        let limiter = make_limiter();
        let result = channel_webhook_gate(
            &FailVerifier,
            &store,
            &limiter,
            "acct1",
            &HeaderMap::new(),
            b"{}",
        );
        assert!(matches!(
            result,
            Err(ChannelWebhookRejection::BadSignature(_))
        ));
    }

    #[test]
    fn gate_deduplicates_by_idempotency_key() {
        let store = make_store();
        let limiter = make_limiter();
        let body = br#"{"id":"ev-dup"}"#;
        let headers = HeaderMap::new();

        let (_, d1) =
            channel_webhook_gate(&PassVerifier, &store, &limiter, "acct1", &headers, body).unwrap();
        assert_eq!(d1, ChannelWebhookDedupeResult::New);

        let (_, d2) =
            channel_webhook_gate(&PassVerifier, &store, &limiter, "acct1", &headers, body).unwrap();
        assert_eq!(d2, ChannelWebhookDedupeResult::Duplicate);
    }

    #[test]
    fn gate_skips_dedup_without_idempotency_key() {
        let store = make_store();
        let limiter = make_limiter();
        let body = br#"{"text":"no id"}"#;
        let headers = HeaderMap::new();

        let (_, d1) =
            channel_webhook_gate(&PassVerifier, &store, &limiter, "acct1", &headers, body).unwrap();
        assert_eq!(d1, ChannelWebhookDedupeResult::New);

        // Same body without id — still New (no dedup key)
        let (_, d2) =
            channel_webhook_gate(&PassVerifier, &store, &limiter, "acct1", &headers, body).unwrap();
        assert_eq!(d2, ChannelWebhookDedupeResult::New);
    }

    #[test]
    fn gate_rate_limits_after_burst() {
        let store = make_store();
        let limiter = ChannelWebhookRateLimiter::new();
        let headers = HeaderMap::new();

        // Use a tiny rate policy: 6 req/min with 0 burst → capacity = 6.
        struct TinyRateVerifier;
        impl ChannelWebhookVerifier for TinyRateVerifier {
            fn verify(
                &self,
                _headers: &HeaderMap,
                body: &[u8],
            ) -> Result<VerifiedChannelWebhook, ChannelWebhookRejection> {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                Ok(VerifiedChannelWebhook {
                    idempotency_key: None,
                    body: Bytes::copy_from_slice(body),
                    timestamp_epoch: Some(now),
                })
            }

            fn rate_policy(&self) -> moltis_channels::ChannelWebhookRatePolicy {
                moltis_channels::ChannelWebhookRatePolicy {
                    max_requests_per_minute: 6,
                    burst: 0,
                }
            }

            fn channel_type(&self) -> ChannelType {
                ChannelType::Slack
            }
        }

        // Exhaust the bucket (capacity = 6).
        for _ in 0..6 {
            let result = channel_webhook_gate(
                &TinyRateVerifier,
                &store,
                &limiter,
                "rate-test",
                &headers,
                b"{}",
            );
            assert!(result.is_ok());
        }

        // Next request should be rate limited.
        let result = channel_webhook_gate(
            &TinyRateVerifier,
            &store,
            &limiter,
            "rate-test",
            &headers,
            b"{}",
        );
        assert!(matches!(
            result,
            Err(ChannelWebhookRejection::RateLimited { .. })
        ));
    }
}
