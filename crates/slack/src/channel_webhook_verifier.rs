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

/// Read one field out of an `application/x-www-form-urlencoded` body.
///
/// Slack posts slash commands and interactions as form bodies, so `+` means
/// space and values are percent-encoded — decoding is delegated to
/// `form_urlencoded` rather than hand-rolled.
fn form_value(body: &[u8], name: &str) -> Option<String> {
    form_urlencoded::parse(body)
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

/// Whether a decoded value is usable as a Slack trigger id.
///
/// `form_urlencoded` decodes leniently, so a body that did not decode cleanly
/// still yields a value: a malformed escape like `%GG` survives verbatim, and
/// invalid UTF-8 becomes U+FFFD. Real trigger ids are dot-separated
/// alphanumerics (`13345224609.738474920.8088930838d88f008e0`) and contain
/// neither, so their presence means the value is decode garbage. Admitting
/// garbage as a dedupe key would let unrelated callbacks collide on it and be
/// dropped as duplicates; rejecting it only means this callback is never
/// deduplicated, which is the safe direction.
fn is_trigger_id(value: &str) -> bool {
    !value.is_empty() && !value.contains(['%', char::REPLACEMENT_CHARACTER])
}

/// Derive a dedupe key for a Slack callback.
///
/// Each endpoint carries its own unique id, in decreasing order of specificity:
/// Events API envelopes have `event_id` (JSON), slash commands have a top-level
/// `trigger_id` (form), and interactions nest `trigger_id` inside the JSON
/// `payload` field (form). Returning `None` means the callback cannot be
/// deduplicated and will be admitted on every delivery.
fn idempotency_key(body: &[u8]) -> Option<String> {
    if let Some(event_id) = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value["event_id"].as_str().map(ToOwned::to_owned))
    {
        return Some(event_id);
    }

    if let Some(trigger_id) = form_value(body, "trigger_id").filter(|value| is_trigger_id(value)) {
        return Some(trigger_id);
    }

    form_value(body, "payload")
        .and_then(|payload| serde_json::from_str::<serde_json::Value>(&payload).ok())
        .and_then(|value| value["trigger_id"].as_str().map(ToOwned::to_owned))
        .filter(|value| is_trigger_id(value))
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

        let timestamp_epoch = timestamp.parse::<i64>().map_err(|_| {
            ChannelWebhookRejection::BadSignature("invalid Slack request timestamp".into())
        })?;

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

        Ok(VerifiedChannelWebhook {
            idempotency_key: idempotency_key(body),
            body: Bytes::copy_from_slice(body),
            timestamp_epoch: Some(timestamp_epoch),
        })
    }

    fn rate_policy(&self) -> moltis_channels::ChannelWebhookRatePolicy {
        moltis_channels::ChannelWebhookRatePolicy {
            max_requests_per_minute: 600,
            burst: 200,
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
    fn malformed_timestamp_rejects_even_with_matching_signature() {
        let verifier = SlackChannelWebhookVerifier::new(Secret::new(TEST_SECRET.into()));
        let body = b"body";
        let headers = make_signed_headers(TEST_SECRET, "not-an-epoch", body);

        assert!(matches!(
            verifier.verify(&headers, body),
            Err(ChannelWebhookRejection::BadSignature(_))
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
    fn command_trigger_id_is_used_for_idempotency() {
        let verifier = SlackChannelWebhookVerifier::new(Secret::new(TEST_SECRET.into()));
        let body = b"command=%2Fmoltis&trigger_id=1337.42.command&text=hello+world";
        let headers = make_signed_headers(TEST_SECRET, "1700000000", body);

        let envelope = verifier.verify(&headers, body).unwrap();
        assert_eq!(envelope.idempotency_key.as_deref(), Some("1337.42.command"));
    }

    #[test]
    fn interaction_payload_trigger_id_is_used_for_idempotency() {
        let verifier = SlackChannelWebhookVerifier::new(Secret::new(TEST_SECRET.into()));
        let body = b"payload=%7B%22type%22%3A%22block_actions%22%2C%22trigger_id%22%3A%221337.42.interaction%22%7D";
        let headers = make_signed_headers(TEST_SECRET, "1700000000", body);

        let envelope = verifier.verify(&headers, body).unwrap();
        assert_eq!(
            envelope.idempotency_key.as_deref(),
            Some("1337.42.interaction")
        );
    }

    #[test]
    fn malformed_form_value_does_not_create_idempotency_key() {
        let verifier = SlackChannelWebhookVerifier::new(Secret::new(TEST_SECRET.into()));
        let body = b"trigger_id=%GG";
        let headers = make_signed_headers(TEST_SECRET, "1700000000", body);

        let envelope = verifier.verify(&headers, body).unwrap();
        assert!(envelope.idempotency_key.is_none());
    }

    #[test]
    fn form_values_decode_plus_as_space_and_percent_escapes() {
        // Slack posts commands as application/x-www-form-urlencoded, where a
        // space is `+` — not `%20`. Treating `+` literally would corrupt every
        // multi-word slash command argument.
        let body = b"command=%2Fmoltis&text=hello+world%21&trigger_id=1.2.abc";
        assert_eq!(form_value(body, "command").as_deref(), Some("/moltis"));
        assert_eq!(form_value(body, "text").as_deref(), Some("hello world!"));
        assert_eq!(form_value(body, "missing"), None);
    }

    #[test]
    fn channel_type_is_slack() {
        let verifier = SlackChannelWebhookVerifier::new(Secret::new(TEST_SECRET.into()));
        assert_eq!(verifier.channel_type(), ChannelType::Slack);
    }

    #[test]
    fn rate_policy_has_callback_headroom() {
        let verifier = SlackChannelWebhookVerifier::new(Secret::new(TEST_SECRET.into()));
        let policy = verifier.rate_policy();
        assert_eq!(policy.max_requests_per_minute, 600);
        assert_eq!(policy.burst, 200);
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
