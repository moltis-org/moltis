//! Telnyx telephony provider.
//!
//! Uses the Telnyx Call Control API v2 for call management.
//! Webhook signature verification uses Ed25519.
//! <https://developers.telnyx.com/docs/api/v2/call-control>

use {
    async_trait::async_trait,
    base64::Engine,
    bytes::Bytes,
    http::HeaderMap,
    reqwest::Client,
    secrecy::{ExposeSecret, Secret},
    tracing::{debug, warn},
};

use crate::{
    provider::{InitiateCallParams, InitiateCallResult, ProviderCallStatus, TelephonyProvider},
    types::{CallEndReason, CallEvent},
};

/// Telnyx provider implementation.
pub struct TelnyxProvider {
    api_key: Secret<String>,
    connection_id: String,
    /// Ed25519 public key for webhook verification (optional).
    public_key: Option<String>,
    client: Client,
    base_url: String,
}

impl TelnyxProvider {
    pub fn new(api_key: Secret<String>, connection_id: String) -> Self {
        Self {
            api_key,
            connection_id,
            public_key: None,
            client: Client::new(),
            base_url: "https://api.telnyx.com/v2".to_string(),
        }
    }

    pub fn with_public_key(mut self, key: impl Into<String>) -> Self {
        self.public_key = Some(key.into());
        self
    }

    #[cfg(test)]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.api_key.expose_secret())
    }
}

impl std::fmt::Debug for TelnyxProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelnyxProvider")
            .field("connection_id", &self.connection_id)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

#[async_trait]
impl TelephonyProvider for TelnyxProvider {
    fn id(&self) -> &'static str {
        "telnyx"
    }

    fn name(&self) -> &'static str {
        "Telnyx"
    }

    fn is_configured(&self) -> bool {
        !self.connection_id.is_empty()
    }

    async fn initiate_call(
        &self,
        params: InitiateCallParams,
    ) -> anyhow::Result<InitiateCallResult> {
        let url = format!("{}/calls", self.base_url);

        // Encode internal state as base64 client_state so we can map
        // webhook events back to our call ID.
        let client_state =
            base64::engine::general_purpose::STANDARD.encode(params.status_callback_url.as_bytes());

        let body = serde_json::json!({
            "connection_id": self.connection_id,
            "to": params.to,
            "from": params.from,
            "webhook_url": params.answer_url,
            "webhook_url_method": "POST",
            "client_state": client_state,
            "timeout_secs": 30,
        });

        debug!(url = %url, "initiating Telnyx call");

        let resp = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Telnyx API error {status}: {text}");
        }

        let json: serde_json::Value = resp.json().await?;
        let call_control_id = json["data"]["call_control_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing call_control_id in Telnyx response"))?;

        Ok(InitiateCallResult {
            provider_call_id: call_control_id.to_string(),
        })
    }

    async fn hangup_call(&self, provider_call_id: &str) -> anyhow::Result<()> {
        let url = format!("{}/calls/{provider_call_id}/actions/hangup", self.base_url);

        let resp = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({ "command_id": uuid::Uuid::new_v4().to_string() }))
            .send()
            .await?;

        if !resp.status().is_success() && resp.status().as_u16() != 404 {
            let text = resp.text().await.unwrap_or_default();
            warn!(sid = %provider_call_id, "Telnyx hangup failed: {text}");
        }
        Ok(())
    }

    async fn play_tts(
        &self,
        provider_call_id: &str,
        text: &str,
        voice: Option<&str>,
        _gather_url: Option<&str>,
    ) -> anyhow::Result<()> {
        let url = format!("{}/calls/{provider_call_id}/actions/speak", self.base_url);

        let resp = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "command_id": uuid::Uuid::new_v4().to_string(),
                "payload": text,
                "voice": voice.unwrap_or("female"),
                "language": "en-US",
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Telnyx speak failed: {text}");
        }
        Ok(())
    }

    async fn send_dtmf(&self, provider_call_id: &str, digits: &str) -> anyhow::Result<()> {
        let url = format!(
            "{}/calls/{provider_call_id}/actions/send_dtmf",
            self.base_url
        );

        let resp = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "command_id": uuid::Uuid::new_v4().to_string(),
                "digits": digits,
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Telnyx send_dtmf failed: {text}");
        }
        Ok(())
    }

    async fn get_call_status(&self, provider_call_id: &str) -> anyhow::Result<ProviderCallStatus> {
        let url = format!("{}/calls/{provider_call_id}", self.base_url);

        let resp = self
            .client
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Telnyx API error {status}: {text}");
        }

        let json: serde_json::Value = resp.json().await?;
        Ok(ProviderCallStatus {
            provider_call_id: provider_call_id.to_string(),
            status: json["data"]["state"]
                .as_str()
                .unwrap_or("unknown")
                .to_string(),
            duration_seconds: None,
        })
    }

    fn verify_webhook(&self, _url: &str, headers: &HeaderMap, body: &[u8]) -> anyhow::Result<()> {
        let Some(ref public_key) = self.public_key else {
            tracing::debug!("telnyx: no public_key configured, skipping webhook verification");
            return Ok(());
        };

        let signature = headers
            .get("telnyx-signature-ed25519")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| anyhow::anyhow!("missing telnyx-signature-ed25519 header"))?;

        let timestamp = headers
            .get("telnyx-timestamp")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| anyhow::anyhow!("missing telnyx-timestamp header"))?;

        // Reject stale timestamps (5 minute window).
        if let Ok(ts) = timestamp.parse::<i64>() {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            if (now - ts).unsigned_abs() > 300 {
                anyhow::bail!("telnyx webhook timestamp too old");
            }
        }

        // Verify Ed25519 signature: signed_payload = "{timestamp}|{body}"
        let body_str = std::str::from_utf8(body).unwrap_or("");
        let signed_payload = format!("{timestamp}|{body_str}");

        let sig_bytes = base64::engine::general_purpose::STANDARD
            .decode(signature)
            .map_err(|e| anyhow::anyhow!("invalid base64 signature: {e}"))?;

        let key_bytes = hex::decode(public_key)
            .or_else(|_| base64::engine::general_purpose::STANDARD.decode(public_key))
            .map_err(|e| anyhow::anyhow!("invalid public key encoding: {e}"))?;

        if key_bytes.len() != 32 {
            anyhow::bail!(
                "invalid Ed25519 public key length: {} (expected 32)",
                key_bytes.len()
            );
        }
        if sig_bytes.len() != 64 {
            anyhow::bail!(
                "invalid Ed25519 signature length: {} (expected 64)",
                sig_bytes.len()
            );
        }

        // Verify using ring (already a transitive dep via rustls).
        use ring::signature;
        let peer_key = signature::UnparsedPublicKey::new(&signature::ED25519, &key_bytes);
        peer_key
            .verify(signed_payload.as_bytes(), &sig_bytes)
            .map_err(|_| anyhow::anyhow!("Ed25519 signature verification failed"))?;

        Ok(())
    }

    fn parse_webhook_event(&self, _headers: &HeaderMap, body: &[u8]) -> anyhow::Result<CallEvent> {
        let payload: serde_json::Value = serde_json::from_slice(body)?;
        let data = &payload["data"];

        let event_type = data["event_type"].as_str().unwrap_or("");

        let call_control_id = data["payload"]["call_control_id"]
            .as_str()
            .unwrap_or("")
            .to_string();

        match event_type {
            "call.initiated" => Ok(CallEvent::Initiated {
                provider_call_id: call_control_id,
            }),
            "call.ringing" => Ok(CallEvent::Ringing {
                provider_call_id: call_control_id,
            }),
            "call.answered" | "call.bridged" => Ok(CallEvent::Answered {
                provider_call_id: call_control_id,
            }),
            "call.speak.started" => Ok(CallEvent::Speaking {
                provider_call_id: call_control_id,
            }),
            "call.transcription" => {
                let transcript = data["payload"]["transcription_data"]["transcript"]
                    .as_str()
                    .or_else(|| data["payload"]["transcription"].as_str())
                    .unwrap_or("")
                    .to_string();
                let confidence = data["payload"]["transcription_data"]["confidence"]
                    .as_f64()
                    .or_else(|| data["payload"]["confidence"].as_f64())
                    .map(|c| c as f32);

                Ok(CallEvent::Speech {
                    provider_call_id: call_control_id,
                    text: transcript,
                    confidence,
                })
            },
            "call.dtmf.received" => {
                let digit = data["payload"]["digit"].as_str().unwrap_or("").to_string();
                Ok(CallEvent::Dtmf {
                    provider_call_id: call_control_id,
                    digits: digit,
                })
            },
            "call.hangup" => {
                let cause = data["payload"]["hangup_cause"].as_str().unwrap_or("");
                let reason = map_hangup_cause(cause);
                Ok(CallEvent::Ended {
                    provider_call_id: call_control_id,
                    reason,
                })
            },
            other => {
                debug!(event_type = %other, "unrecognized Telnyx event");
                Ok(CallEvent::Error {
                    provider_call_id: call_control_id,
                    message: format!("unrecognized event: {other}"),
                })
            },
        }
    }

    fn build_answer_response(&self, _message: Option<&str>, _gather_url: Option<&str>) -> Bytes {
        // Telnyx uses Call Control commands, not response documents like TwiML.
        // The webhook handler sends commands directly via the API.
        Bytes::from("{}")
    }

    fn build_gather_response(&self, _prompt: Option<&str>, _action_url: &str) -> Bytes {
        Bytes::from("{}")
    }

    fn build_play_response(&self, _audio_url: &str) -> Bytes {
        Bytes::from("{}")
    }

    fn build_hangup_response(&self) -> Bytes {
        Bytes::from("{}")
    }
}

/// Map Telnyx hangup cause to normalized CallEndReason.
fn map_hangup_cause(cause: &str) -> CallEndReason {
    match cause {
        "normal_clearing" | "normal_unspecified" => CallEndReason::Completed,
        "originator_cancel" => CallEndReason::HangupBot,
        "user_hangup" | "subscriber_absent" => CallEndReason::HangupUser,
        "call_rejected" | "user_busy" => CallEndReason::Busy,
        "no_answer" | "no_user_response" => CallEndReason::NoAnswer,
        "destination_out_of_order"
        | "network_out_of_order"
        | "service_unavailable"
        | "recovery_on_timer_expire" => CallEndReason::Error,
        _ => {
            if !cause.is_empty() {
                warn!(cause = %cause, "unknown Telnyx hangup cause");
            }
            CallEndReason::Completed
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn serve_telnyx_response(status: &str, body: &str) -> String {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap_or_else(|error| panic!("test server should bind: {error}"));
        let addr = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("test server address should be available: {error}"));
        let status = status.to_string();
        let body = body.to_string();
        tokio::spawn(async move {
            let (mut stream, _) = listener
                .accept()
                .await
                .unwrap_or_else(|error| panic!("test server should accept request: {error}"));
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request).await;
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .unwrap_or_else(|error| panic!("test server should write response: {error}"));
        });

        format!("http://{addr}/v2")
    }

    #[test]
    fn telnyx_debug_redacts_key() {
        let provider = TelnyxProvider::new(Secret::new("KEY_LIVE_xxx".into()), "conn_123".into());
        let debug_str = format!("{provider:?}");
        assert!(!debug_str.contains("KEY_LIVE_xxx"));
        assert!(debug_str.contains("[REDACTED]"));
    }

    #[test]
    fn parse_webhook_call_initiated() {
        let provider = TelnyxProvider::new(Secret::new("key".into()), "conn".into());
        let body = serde_json::to_vec(&serde_json::json!({
            "data": {
                "event_type": "call.initiated",
                "payload": { "call_control_id": "v2:abc123" }
            }
        }))
        .unwrap_or_default();
        let headers = HeaderMap::new();
        let event = provider
            .parse_webhook_event(&headers, &body)
            .unwrap_or_else(|e| panic!("{e}"));
        match event {
            CallEvent::Initiated { provider_call_id } => {
                assert_eq!(provider_call_id, "v2:abc123");
            },
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parse_webhook_call_hangup() {
        let provider = TelnyxProvider::new(Secret::new("key".into()), "conn".into());
        let body = serde_json::to_vec(&serde_json::json!({
            "data": {
                "event_type": "call.hangup",
                "payload": {
                    "call_control_id": "v2:xyz",
                    "hangup_cause": "user_hangup"
                }
            }
        }))
        .unwrap_or_default();
        let headers = HeaderMap::new();
        let event = provider
            .parse_webhook_event(&headers, &body)
            .unwrap_or_else(|e| panic!("{e}"));
        match event {
            CallEvent::Ended {
                provider_call_id,
                reason,
            } => {
                assert_eq!(provider_call_id, "v2:xyz");
                assert_eq!(reason, CallEndReason::HangupUser);
            },
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parse_webhook_transcription() {
        let provider = TelnyxProvider::new(Secret::new("key".into()), "conn".into());
        let body = serde_json::to_vec(&serde_json::json!({
            "data": {
                "event_type": "call.transcription",
                "payload": {
                    "call_control_id": "v2:t1",
                    "transcription_data": {
                        "transcript": "hello world",
                        "confidence": 0.95,
                        "is_final": true
                    }
                }
            }
        }))
        .unwrap_or_default();
        let headers = HeaderMap::new();
        let event = provider
            .parse_webhook_event(&headers, &body)
            .unwrap_or_else(|e| panic!("{e}"));
        match event {
            CallEvent::Speech {
                text, confidence, ..
            } => {
                assert_eq!(text, "hello world");
                assert!((confidence.unwrap_or(0.0) - 0.95).abs() < 0.01);
            },
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parse_webhook_dtmf() {
        let provider = TelnyxProvider::new(Secret::new("key".into()), "conn".into());
        let body = serde_json::to_vec(&serde_json::json!({
            "data": {
                "event_type": "call.dtmf.received",
                "payload": {
                    "call_control_id": "v2:d1",
                    "digit": "5"
                }
            }
        }))
        .unwrap_or_default();
        let headers = HeaderMap::new();
        let event = provider
            .parse_webhook_event(&headers, &body)
            .unwrap_or_else(|e| panic!("{e}"));
        match event {
            CallEvent::Dtmf { digits, .. } => assert_eq!(digits, "5"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn hangup_cause_mapping() {
        assert_eq!(
            map_hangup_cause("normal_clearing"),
            CallEndReason::Completed
        );
        assert_eq!(map_hangup_cause("user_hangup"), CallEndReason::HangupUser);
        assert_eq!(map_hangup_cause("user_busy"), CallEndReason::Busy);
        assert_eq!(map_hangup_cause("no_answer"), CallEndReason::NoAnswer);
        assert_eq!(
            map_hangup_cause("service_unavailable"),
            CallEndReason::Error
        );
        assert_eq!(map_hangup_cause(""), CallEndReason::Completed);
    }

    #[test]
    fn verify_webhook_rejects_missing_headers_when_key_configured() {
        let provider = TelnyxProvider::new(Secret::new("key".into()), "conn".into())
            .with_public_key("aabbccdd00112233445566778899aabb00112233445566778899aabbccddeeff");
        let result = provider.verify_webhook("https://example.com", &HeaderMap::new(), b"body");
        match result {
            Ok(()) => panic!("missing headers should fail verification"),
            Err(error) => assert!(error.to_string().contains("missing")),
        }
    }

    #[test]
    fn verify_webhook_skips_when_no_key() {
        let provider = TelnyxProvider::new(Secret::new("key".into()), "conn".into());
        // No public key configured — verification skipped
        let result = provider.verify_webhook("https://example.com", &HeaderMap::new(), b"body");
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn get_call_status_rejects_non_success_response() {
        let base_url =
            serve_telnyx_response("404 Not Found", r#"{"errors":[{"detail":"not found"}]}"#).await;
        let provider =
            TelnyxProvider::new(Secret::new("key".into()), "conn".into()).with_base_url(base_url);

        let error = provider
            .get_call_status("call-123")
            .await
            .err()
            .unwrap_or_else(|| panic!("non-success response should fail"));

        let message = error.to_string();
        assert!(message.contains("Telnyx API error 404"));
        assert!(message.contains("not found"));
    }
}
