//! Channel webhook middleware pipeline.
//!
//! The [`channel_webhook_gate`] function is the single entry point for all
//! webhook-based channel routes. It runs:
//!
//! 1. Signature verification (via the channel's [`ChannelWebhookVerifier`])
//! 2. Timestamp staleness rejection
//! 3. Idempotency deduplication
//!
//! Rate limiting (Phase 3) will be added between steps 1 and 3.

use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};

use moltis_channels::channel_webhook_middleware::{
    ChannelWebhookDedupeResult, ChannelWebhookRejection, ChannelWebhookVerifier, TimestampGuard,
    VerifiedChannelWebhook,
};

use crate::channel_webhook_dedup::ChannelWebhookDedupeStore;

/// Run the full channel webhook verification pipeline.
///
/// Steps:
/// 1. Verify the cryptographic signature via the channel's verifier.
/// 2. Check the timestamp is within the acceptable staleness window.
/// 3. Deduplicate by provider message ID (if present).
///
/// On success returns the verified envelope and the dedup result.
/// On failure returns a rejection that maps to an HTTP response via [`IntoResponse`].
pub fn channel_webhook_gate(
    verifier: &dyn ChannelWebhookVerifier,
    dedup_store: &std::sync::RwLock<ChannelWebhookDedupeStore>,
    headers: &axum::http::HeaderMap,
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

    // Step 3: Idempotency deduplication.
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

/// Convert a [`ChannelWebhookRejection`] into an axum HTTP [`Response`].
pub fn rejection_into_response(rejection: ChannelWebhookRejection) -> Response {
    match rejection {
        ChannelWebhookRejection::BadSignature(ref msg) => (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({ "ok": false, "error": msg })),
        )
            .into_response(),
        ChannelWebhookRejection::StaleTimestamp {
            age_seconds,
            max_seconds,
        } => (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({
                "ok": false,
                "error": format!(
                    "request timestamp too old ({age_seconds}s > {max_seconds}s max)"
                )
            })),
        )
            .into_response(),
        ChannelWebhookRejection::MissingHeaders(ref header) => (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "ok": false,
                "error": format!("missing required header: {header}")
            })),
        )
            .into_response(),
        ChannelWebhookRejection::Duplicate => (
            StatusCode::OK,
            axum::Json(serde_json::json!({ "ok": true, "deduplicated": true })),
        )
            .into_response(),
        ChannelWebhookRejection::RateLimited { retry_after } => {
            let secs = retry_after.as_secs().max(1);
            let mut resp = (
                StatusCode::TOO_MANY_REQUESTS,
                axum::Json(serde_json::json!({
                    "ok": false,
                    "error": "rate limited",
                    "retry_after_seconds": secs
                })),
            )
                .into_response();
            if let Ok(val) = secs.to_string().parse() {
                resp.headers_mut()
                    .insert(axum::http::header::RETRY_AFTER, val);
            }
            resp
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use moltis_channels::plugin::ChannelType;

    /// Dummy verifier that always passes for testing the pipeline.
    struct PassVerifier;

    impl ChannelWebhookVerifier for PassVerifier {
        fn verify(
            &self,
            _headers: &axum::http::HeaderMap,
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
            _headers: &axum::http::HeaderMap,
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

    #[test]
    fn gate_passes_valid_request() {
        let store = make_store();
        let body = br#"{"id":"ev1","text":"hello"}"#;
        let result = channel_webhook_gate(&PassVerifier, &store, &axum::http::HeaderMap::new(), body);
        assert!(result.is_ok());
        let (env, dedup) = result.unwrap();
        assert_eq!(env.idempotency_key.as_deref(), Some("ev1"));
        assert_eq!(dedup, ChannelWebhookDedupeResult::New);
    }

    #[test]
    fn gate_rejects_bad_signature() {
        let store = make_store();
        let result =
            channel_webhook_gate(&FailVerifier, &store, &axum::http::HeaderMap::new(), b"{}");
        assert!(matches!(
            result,
            Err(ChannelWebhookRejection::BadSignature(_))
        ));
    }

    #[test]
    fn gate_deduplicates_by_idempotency_key() {
        let store = make_store();
        let body = br#"{"id":"ev-dup"}"#;
        let headers = axum::http::HeaderMap::new();

        let (_, d1) = channel_webhook_gate(&PassVerifier, &store, &headers, body).unwrap();
        assert_eq!(d1, ChannelWebhookDedupeResult::New);

        let (_, d2) = channel_webhook_gate(&PassVerifier, &store, &headers, body).unwrap();
        assert_eq!(d2, ChannelWebhookDedupeResult::Duplicate);
    }

    #[test]
    fn gate_skips_dedup_without_idempotency_key() {
        let store = make_store();
        let body = br#"{"text":"no id"}"#;
        let headers = axum::http::HeaderMap::new();

        let (_, d1) = channel_webhook_gate(&PassVerifier, &store, &headers, body).unwrap();
        assert_eq!(d1, ChannelWebhookDedupeResult::New);

        // Same body without id — still New (no dedup key)
        let (_, d2) = channel_webhook_gate(&PassVerifier, &store, &headers, body).unwrap();
        assert_eq!(d2, ChannelWebhookDedupeResult::New);
    }

    #[test]
    fn rejection_into_response_status_codes() {
        let bad_sig =
            rejection_into_response(ChannelWebhookRejection::BadSignature("test".into()));
        assert_eq!(bad_sig.status(), StatusCode::UNAUTHORIZED);

        let stale = rejection_into_response(ChannelWebhookRejection::StaleTimestamp {
            age_seconds: 400,
            max_seconds: 300,
        });
        assert_eq!(stale.status(), StatusCode::UNAUTHORIZED);

        let missing =
            rejection_into_response(ChannelWebhookRejection::MissingHeaders("x-sig".into()));
        assert_eq!(missing.status(), StatusCode::BAD_REQUEST);

        let dup = rejection_into_response(ChannelWebhookRejection::Duplicate);
        assert_eq!(dup.status(), StatusCode::OK);

        let rate = rejection_into_response(ChannelWebhookRejection::RateLimited {
            retry_after: std::time::Duration::from_secs(30),
        });
        assert_eq!(rate.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            rate.headers()
                .get(axum::http::header::RETRY_AFTER)
                .unwrap(),
            "30"
        );
    }
}
