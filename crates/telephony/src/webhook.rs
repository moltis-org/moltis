//! Webhook verification for telephony providers.
//!
//! Integrates with the shared `ChannelWebhookVerifier` infrastructure.

use {
    bytes::Bytes,
    http::HeaderMap,
    moltis_channels::{
        ChannelType,
        channel_webhook_middleware::{
            ChannelWebhookRatePolicy, ChannelWebhookRejection, ChannelWebhookVerifier,
            VerifiedChannelWebhook,
        },
    },
    std::sync::Arc,
};

use crate::provider::TelephonyProvider;

/// Webhook verifier that delegates to the active telephony provider.
pub struct TelephonyWebhookVerifier {
    provider: Arc<dyn TelephonyProvider>,
    webhook_url: String,
}

impl TelephonyWebhookVerifier {
    pub fn new(provider: Arc<dyn TelephonyProvider>, webhook_url: String) -> Self {
        Self {
            provider,
            webhook_url,
        }
    }
}

impl ChannelWebhookVerifier for TelephonyWebhookVerifier {
    fn verify(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<VerifiedChannelWebhook, ChannelWebhookRejection> {
        self.provider
            .verify_webhook(&self.webhook_url, headers, body)
            .map_err(|e| ChannelWebhookRejection::BadSignature(e.to_string()))?;

        Ok(VerifiedChannelWebhook {
            idempotency_key: None,
            body: Bytes::copy_from_slice(body),
            timestamp_epoch: None,
        })
    }

    fn rate_policy(&self) -> ChannelWebhookRatePolicy {
        ChannelWebhookRatePolicy {
            max_requests_per_minute: 120,
            burst: 20,
        }
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Telephony
    }
}
