//! Microsoft Teams webhook verifier implementing [`ChannelWebhookVerifier`].
//!
//! Uses a shared-secret query parameter (injected as the
//! `x-moltis-webhook-secret` header by the gateway route handler) with
//! constant-time comparison.

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
    subtle::ConstantTimeEq,
};

/// Teams webhook verifier using shared-secret comparison.
///
/// The gateway route handler extracts `?secret=<value>` from the query string
/// and places it in the `x-moltis-webhook-secret` header before calling this
/// verifier.
pub struct TeamsChannelWebhookVerifier {
    webhook_secret: Option<Secret<String>>,
    require_secret: bool,
}

impl TeamsChannelWebhookVerifier {
    pub fn new(webhook_secret: Option<Secret<String>>, require_secret: bool) -> Self {
        Self {
            webhook_secret,
            require_secret,
        }
    }
}

impl ChannelWebhookVerifier for TeamsChannelWebhookVerifier {
    fn verify(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<VerifiedChannelWebhook, ChannelWebhookRejection> {
        if let Some(expected) = self
            .webhook_secret
            .as_ref()
            .map(ExposeSecret::expose_secret)
            .filter(|s| !s.is_empty())
        {
            let provided = headers
                .get("x-moltis-webhook-secret")
                .and_then(|v| v.to_str().ok());

            let matches = provided.is_some_and(|p| p.as_bytes().ct_eq(expected.as_bytes()).into());

            if !matches {
                return Err(ChannelWebhookRejection::BadSignature(
                    "invalid Teams webhook secret".into(),
                ));
            }
        } else if self.require_secret {
            return Err(ChannelWebhookRejection::BadSignature(
                "Teams webhook_secret is required but not configured".into(),
            ));
        }

        // Extract idempotency key from the JSON payload's `id` field.
        let idempotency_key = serde_json::from_slice::<serde_json::Value>(body)
            .ok()
            .and_then(|v| v.get("id").and_then(|e| e.as_str()).map(String::from));

        Ok(VerifiedChannelWebhook {
            idempotency_key,
            body: Bytes::copy_from_slice(body),
            timestamp_epoch: None, // Teams does not provide a timestamp header
        })
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::MsTeams
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn headers_with_secret(secret: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-moltis-webhook-secret", secret.parse().unwrap());
        headers
    }

    #[test]
    fn valid_secret_passes() {
        let verifier =
            TeamsChannelWebhookVerifier::new(Some(Secret::new("my-secret".into())), true);
        let body = br#"{"id":"act-123","type":"message","text":"hello"}"#;
        let headers = headers_with_secret("my-secret");

        let result = verifier.verify(&headers, body);
        assert!(result.is_ok());

        let envelope = result.unwrap();
        assert_eq!(envelope.idempotency_key.as_deref(), Some("act-123"));
        assert!(envelope.timestamp_epoch.is_none());
    }

    #[test]
    fn wrong_secret_rejects() {
        let verifier =
            TeamsChannelWebhookVerifier::new(Some(Secret::new("correct-secret".into())), true);
        let headers = headers_with_secret("wrong-secret");

        let result = verifier.verify(&headers, b"{}");
        assert!(matches!(
            result,
            Err(ChannelWebhookRejection::BadSignature(_))
        ));
    }

    #[test]
    fn missing_secret_header_rejects() {
        let verifier =
            TeamsChannelWebhookVerifier::new(Some(Secret::new("my-secret".into())), true);
        let headers = HeaderMap::new();

        let result = verifier.verify(&headers, b"{}");
        assert!(matches!(
            result,
            Err(ChannelWebhookRejection::BadSignature(_))
        ));
    }

    #[test]
    fn require_secret_but_not_configured_rejects() {
        let verifier = TeamsChannelWebhookVerifier::new(None, true);
        let headers = HeaderMap::new();

        let result = verifier.verify(&headers, b"{}");
        assert!(matches!(
            result,
            Err(ChannelWebhookRejection::BadSignature(_))
        ));
    }

    #[test]
    fn no_secret_configured_and_not_required_passes() {
        let verifier = TeamsChannelWebhookVerifier::new(None, false);
        let body = br#"{"id":"act-456","type":"message"}"#;
        let headers = HeaderMap::new();

        let result = verifier.verify(&headers, body);
        assert!(result.is_ok());

        let envelope = result.unwrap();
        assert_eq!(envelope.idempotency_key.as_deref(), Some("act-456"));
    }

    #[test]
    fn empty_secret_configured_and_not_required_passes() {
        let verifier = TeamsChannelWebhookVerifier::new(Some(Secret::new(String::new())), false);
        let headers = HeaderMap::new();

        let result = verifier.verify(&headers, b"{}");
        assert!(result.is_ok());
    }

    #[test]
    fn no_id_field_yields_none_idempotency_key() {
        let verifier = TeamsChannelWebhookVerifier::new(None, false);
        let body = br#"{"type":"message","text":"hi"}"#;

        let envelope = verifier.verify(&HeaderMap::new(), body).unwrap();
        assert!(envelope.idempotency_key.is_none());
    }

    #[test]
    fn channel_type_is_msteams() {
        let verifier = TeamsChannelWebhookVerifier::new(None, false);
        assert_eq!(verifier.channel_type(), ChannelType::MsTeams);
    }

    // ── Contract tests ──────────────────────────────────────────────────────

    #[test]
    fn contract_rejects_empty_signature() {
        // With require_secret=true, missing headers → BadSignature.
        let verifier = TeamsChannelWebhookVerifier::new(Some(Secret::new("secret".into())), true);
        moltis_channels::contract::channel_webhook_verifier_rejects_empty_signature(&verifier);
    }

    #[test]
    fn contract_rejects_bad_signature() {
        let verifier =
            TeamsChannelWebhookVerifier::new(Some(Secret::new("correct-secret".into())), true);
        let headers = headers_with_secret("wrong-secret");
        moltis_channels::contract::channel_webhook_verifier_rejects_bad_signature(
            &verifier, &headers,
        );
    }

    #[test]
    fn contract_has_channel_type() {
        let verifier = TeamsChannelWebhookVerifier::new(None, false);
        moltis_channels::contract::channel_webhook_verifier_has_channel_type(&verifier);
    }

    #[test]
    fn contract_has_positive_max_age() {
        let verifier = TeamsChannelWebhookVerifier::new(None, false);
        moltis_channels::contract::channel_webhook_verifier_has_positive_max_age(&verifier);
    }

    #[test]
    fn contract_has_valid_rate_policy() {
        let verifier = TeamsChannelWebhookVerifier::new(None, false);
        moltis_channels::contract::channel_webhook_verifier_has_valid_rate_policy(&verifier);
    }
}
