//! Slack webhook verifier implementing [`ChannelWebhookVerifier`].
//!
//! Uses HMAC-SHA256 with the app's signing secret, following the
//! [Slack verification protocol](https://api.slack.com/authentication/verifying-requests-from-slack).

use {
    bytes::Bytes,
    http::HeaderMap,
    moltis_channels::{
        channel_webhook_middleware::{
            ChannelWebhookRejection, ChannelWebhookVerifier, VerifiedChannelWebhook,
        },
        plugin::ChannelType,
    },
    secrecy::{ExposeSecret, Secret},
};

use crate::webhook::verify_signature;

/// Slack Events API / Interactions webhook verifier.
///
/// Wraps the existing HMAC-SHA256 [`verify_signature`] function behind the
/// shared [`ChannelWebhookVerifier`] trait.
pub struct SlackChannelWebhookVerifier {
    signing_secret: Secret<String>,
}

impl SlackChannelWebhookVerifier {
    pub fn new(signing_secret: Secret<String>) -> Self {
        Self { signing_secret }
    }
}

impl ChannelWebhookVerifier for SlackChannelWebhookVerifier {
    fn verify(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<VerifiedChannelWebhook, ChannelWebhookRejection> {
        let timestamp = headers
            .get("x-slack-request-timestamp")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                ChannelWebhookRejection::MissingHeaders("x-slack-request-timestamp".into())
            })?;

        let signature = headers
            .get("x-slack-signature")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| ChannelWebhookRejection::MissingHeaders("x-slack-signature".into()))?;

        if !verify_signature(
            self.signing_secret.expose_secret(),
            timestamp,
            body,
            signature,
        ) {
            return Err(ChannelWebhookRejection::BadSignature(
                "invalid Slack webhook signature".into(),
            ));
        }

        // Extract idempotency key from JSON `event_id` field (Events API).
        let idempotency_key = serde_json::from_slice::<serde_json::Value>(body)
            .ok()
            .and_then(|v| v.get("event_id").and_then(|e| e.as_str()).map(String::from));

        let timestamp_epoch = timestamp.parse::<i64>().ok();

        Ok(VerifiedChannelWebhook {
            idempotency_key,
            body: Bytes::copy_from_slice(body),
            timestamp_epoch,
        })
    }

    fn rate_policy(&self) -> moltis_channels::ChannelWebhookRatePolicy {
        moltis_channels::ChannelWebhookRatePolicy {
            max_requests_per_minute: 30,
            burst: 10,
        }
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Slack
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    use {
        hmac::{Hmac, Mac},
        sha2::Sha256,
    };

    const TEST_SECRET: &str = "test_signing_secret_123";

    /// Build valid Slack signature headers for a given body and timestamp.
    fn make_signed_headers(secret: &str, timestamp: &str, body: &[u8]) -> HeaderMap {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("valid HMAC key");
        mac.update(b"v0:");
        mac.update(timestamp.as_bytes());
        mac.update(b":");
        mac.update(body);
        let sig = mac.finalize().into_bytes();
        let hex = sig.iter().map(|b| format!("{b:02x}")).collect::<String>();

        let mut headers = HeaderMap::new();
        headers.insert("x-slack-request-timestamp", timestamp.parse().unwrap());
        headers.insert("x-slack-signature", format!("v0={hex}").parse().unwrap());
        headers
    }

    #[test]
    fn valid_signature_passes() {
        let verifier = SlackChannelWebhookVerifier::new(Secret::new(TEST_SECRET.into()));
        let body = br#"{"type":"event_callback","event_id":"Ev123"}"#;
        let ts = "1700000000";
        let headers = make_signed_headers(TEST_SECRET, ts, body);

        let result = verifier.verify(&headers, body);
        assert!(result.is_ok());

        let envelope = result.unwrap();
        assert_eq!(envelope.idempotency_key.as_deref(), Some("Ev123"));
        assert_eq!(envelope.timestamp_epoch, Some(1_700_000_000));
    }

    #[test]
    fn bad_signature_rejects() {
        let verifier = SlackChannelWebhookVerifier::new(Secret::new(TEST_SECRET.into()));
        let body = b"hello";
        let mut headers = HeaderMap::new();
        headers.insert("x-slack-request-timestamp", "1700000000".parse().unwrap());
        headers.insert(
            "x-slack-signature",
            "v0=deadbeef00000000000000000000000000000000000000000000000000000000"
                .parse()
                .unwrap(),
        );

        let result = verifier.verify(&headers, body);
        assert!(matches!(
            result,
            Err(ChannelWebhookRejection::BadSignature(_))
        ));
    }

    #[test]
    fn missing_timestamp_header_rejects() {
        let verifier = SlackChannelWebhookVerifier::new(Secret::new(TEST_SECRET.into()));
        let mut headers = HeaderMap::new();
        headers.insert("x-slack-signature", "v0=abc".parse().unwrap());

        let result = verifier.verify(&headers, b"body");
        assert!(matches!(
            result,
            Err(ChannelWebhookRejection::MissingHeaders(_))
        ));
    }

    #[test]
    fn missing_signature_header_rejects() {
        let verifier = SlackChannelWebhookVerifier::new(Secret::new(TEST_SECRET.into()));
        let mut headers = HeaderMap::new();
        headers.insert("x-slack-request-timestamp", "1700000000".parse().unwrap());

        let result = verifier.verify(&headers, b"body");
        assert!(matches!(
            result,
            Err(ChannelWebhookRejection::MissingHeaders(_))
        ));
    }

    #[test]
    fn no_event_id_yields_none_idempotency_key() {
        let verifier = SlackChannelWebhookVerifier::new(Secret::new(TEST_SECRET.into()));
        let body = br#"{"type":"url_verification","challenge":"abc"}"#;
        let ts = "1700000000";
        let headers = make_signed_headers(TEST_SECRET, ts, body);

        let envelope = verifier.verify(&headers, body).unwrap();
        assert!(envelope.idempotency_key.is_none());
    }

    #[test]
    fn channel_type_is_slack() {
        let verifier = SlackChannelWebhookVerifier::new(Secret::new(TEST_SECRET.into()));
        assert_eq!(verifier.channel_type(), ChannelType::Slack);
    }

    #[test]
    fn rate_policy_is_30_per_minute() {
        let verifier = SlackChannelWebhookVerifier::new(Secret::new(TEST_SECRET.into()));
        let policy = verifier.rate_policy();
        assert_eq!(policy.max_requests_per_minute, 30);
        assert_eq!(policy.burst, 10);
    }

    // ── Contract tests ──────────────────────────────────────────────────────

    #[test]
    fn contract_rejects_empty_signature() {
        let verifier = SlackChannelWebhookVerifier::new(Secret::new(TEST_SECRET.into()));
        moltis_channels::contract::channel_webhook_verifier_rejects_empty_signature(&verifier);
    }

    #[test]
    fn contract_rejects_bad_signature() {
        let verifier = SlackChannelWebhookVerifier::new(Secret::new(TEST_SECRET.into()));
        let mut headers = HeaderMap::new();
        headers.insert("x-slack-request-timestamp", "1700000000".parse().unwrap());
        headers.insert(
            "x-slack-signature",
            "v0=0000000000000000000000000000000000000000000000000000000000000000"
                .parse()
                .unwrap(),
        );
        moltis_channels::contract::channel_webhook_verifier_rejects_bad_signature(
            &verifier, &headers,
        );
    }

    #[test]
    fn contract_has_channel_type() {
        let verifier = SlackChannelWebhookVerifier::new(Secret::new(TEST_SECRET.into()));
        moltis_channels::contract::channel_webhook_verifier_has_channel_type(&verifier);
    }

    #[test]
    fn contract_has_positive_max_age() {
        let verifier = SlackChannelWebhookVerifier::new(Secret::new(TEST_SECRET.into()));
        moltis_channels::contract::channel_webhook_verifier_has_positive_max_age(&verifier);
    }

    #[test]
    fn contract_has_valid_rate_policy() {
        let verifier = SlackChannelWebhookVerifier::new(Secret::new(TEST_SECRET.into()));
        moltis_channels::contract::channel_webhook_verifier_has_valid_rate_policy(&verifier);
    }
}
