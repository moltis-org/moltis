//! Telephony provider trait — the abstraction all backends implement.

use {async_trait::async_trait, bytes::Bytes, http::HeaderMap, serde_json::Value};

use crate::types::{CallEvent, CallMode};

/// Parameters for initiating an outbound call.
#[derive(Debug, Clone)]
pub struct InitiateCallParams {
    /// Caller ID (E.164).
    pub from: String,
    /// Destination number (E.164).
    pub to: String,
    /// Call mode: notify or conversation.
    pub mode: CallMode,
    /// Initial message to speak when the call connects.
    pub message: Option<String>,
    /// Webhook URL for status callbacks.
    pub status_callback_url: String,
    /// Webhook URL for the call to fetch TwiML / instructions from.
    pub answer_url: String,
}

/// Result of initiating a call.
#[derive(Debug, Clone)]
pub struct InitiateCallResult {
    /// Provider-assigned call identifier.
    pub provider_call_id: String,
}

/// Provider-level call status.
#[derive(Debug, Clone)]
pub struct ProviderCallStatus {
    pub provider_call_id: String,
    pub status: String,
    pub duration_seconds: Option<u64>,
}

/// Pluggable telephony backend.
///
/// Implementations translate between our normalized types and each provider's
/// REST API, webhook format, and audio streaming protocol.
#[async_trait]
pub trait TelephonyProvider: Send + Sync {
    /// Short identifier (e.g. "twilio").
    fn id(&self) -> &'static str;

    /// Human-readable name.
    fn name(&self) -> &'static str;

    /// Whether the provider has valid credentials configured.
    fn is_configured(&self) -> bool;

    /// Place an outbound call.
    async fn initiate_call(&self, params: InitiateCallParams)
    -> anyhow::Result<InitiateCallResult>;

    /// Hang up an active call.
    async fn hangup_call(&self, provider_call_id: &str) -> anyhow::Result<()>;

    /// Play TTS audio to the caller. Returns when playback is queued.
    async fn play_tts(
        &self,
        provider_call_id: &str,
        text: &str,
        voice: Option<&str>,
        gather_url: Option<&str>,
    ) -> anyhow::Result<()>;

    /// Send DTMF digits to the remote party.
    async fn send_dtmf(&self, provider_call_id: &str, digits: &str) -> anyhow::Result<()>;

    /// Query the provider for current call status.
    async fn get_call_status(&self, provider_call_id: &str) -> anyhow::Result<ProviderCallStatus>;

    /// Verify a webhook signature. Returns the raw body on success.
    fn verify_webhook(&self, url: &str, headers: &HeaderMap, body: &[u8]) -> anyhow::Result<()>;

    /// Parse a provider webhook payload into a normalized `CallEvent`.
    fn parse_webhook_event(&self, headers: &HeaderMap, body: &[u8]) -> anyhow::Result<CallEvent>;

    /// Generate the initial response document (e.g. TwiML) for an answered call.
    fn build_answer_response(&self, message: Option<&str>, gather_url: Option<&str>) -> Bytes;

    /// Generate a gather/listen response for collecting speech input.
    fn build_gather_response(&self, prompt: Option<&str>, action_url: &str) -> Bytes;

    /// Generate TwiML or equivalent for playing a raw audio payload.
    fn build_play_response(&self, audio_url: &str) -> Bytes;

    /// Build a response that hangs up the call.
    fn build_hangup_response(&self) -> Bytes;

    /// Provider-specific status page data for health probes.
    async fn health_check(&self) -> anyhow::Result<Value> {
        Ok(Value::Null)
    }
}
