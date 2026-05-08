//! Mock telephony provider for testing.

use {
    async_trait::async_trait,
    bytes::Bytes,
    http::HeaderMap,
    std::sync::atomic::{AtomicU64, Ordering},
};

use crate::{
    provider::{InitiateCallParams, InitiateCallResult, ProviderCallStatus, TelephonyProvider},
    types::CallEvent,
};

/// Mock provider that records calls without making real API requests.
pub struct MockProvider {
    call_counter: AtomicU64,
}

impl MockProvider {
    pub fn new() -> Self {
        Self {
            call_counter: AtomicU64::new(0),
        }
    }
}

impl Default for MockProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TelephonyProvider for MockProvider {
    fn id(&self) -> &'static str {
        "mock"
    }

    fn name(&self) -> &'static str {
        "Mock"
    }

    fn is_configured(&self) -> bool {
        true
    }

    async fn initiate_call(
        &self,
        _params: InitiateCallParams,
    ) -> anyhow::Result<InitiateCallResult> {
        let n = self.call_counter.fetch_add(1, Ordering::Relaxed);
        Ok(InitiateCallResult {
            provider_call_id: format!("MOCK-{n}"),
        })
    }

    async fn hangup_call(&self, _provider_call_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn play_tts(
        &self,
        _provider_call_id: &str,
        _text: &str,
        _voice: Option<&str>,
        _gather_url: Option<&str>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn send_dtmf(&self, _provider_call_id: &str, _digits: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn get_call_status(&self, provider_call_id: &str) -> anyhow::Result<ProviderCallStatus> {
        Ok(ProviderCallStatus {
            provider_call_id: provider_call_id.to_string(),
            status: "in-progress".to_string(),
            duration_seconds: Some(0),
        })
    }

    fn verify_webhook(&self, _url: &str, _headers: &HeaderMap, _body: &[u8]) -> anyhow::Result<()> {
        Ok(())
    }

    fn parse_webhook_event(&self, _headers: &HeaderMap, _body: &[u8]) -> anyhow::Result<CallEvent> {
        anyhow::bail!("mock provider does not handle real webhooks")
    }

    fn build_answer_response(&self, message: Option<&str>, _gather_url: Option<&str>) -> Bytes {
        let msg = message.unwrap_or("Mock call connected.");
        Bytes::from(format!("<Response><Say>{msg}</Say></Response>"))
    }

    fn build_gather_response(&self, prompt: Option<&str>, _action_url: &str) -> Bytes {
        let msg = prompt.unwrap_or("Listening...");
        Bytes::from(format!("<Response><Say>{msg}</Say></Response>"))
    }

    fn build_play_response(&self, audio_url: &str) -> Bytes {
        Bytes::from(format!("<Response><Play>{audio_url}</Play></Response>"))
    }

    fn build_hangup_response(&self) -> Bytes {
        Bytes::from("<Response><Hangup/></Response>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_initiate_returns_sequential_ids() {
        let provider = MockProvider::new();
        let r1 = provider
            .initiate_call(InitiateCallParams {
                from: "+1".into(),
                to: "+2".into(),
                mode: crate::types::CallMode::Conversation,
                message: None,
                status_callback_url: "http://localhost/status".into(),
                answer_url: "http://localhost/answer".into(),
            })
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r1.provider_call_id, "MOCK-0");

        let r2 = provider
            .initiate_call(InitiateCallParams {
                from: "+1".into(),
                to: "+2".into(),
                mode: crate::types::CallMode::Notify,
                message: Some("hi".into()),
                status_callback_url: "http://localhost/status".into(),
                answer_url: "http://localhost/answer".into(),
            })
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r2.provider_call_id, "MOCK-1");
    }

    #[test]
    fn mock_is_configured() {
        assert!(MockProvider::new().is_configured());
    }
}
