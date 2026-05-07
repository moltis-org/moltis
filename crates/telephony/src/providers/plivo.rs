//! Plivo telephony provider.
//!
//! Uses the Plivo Voice API with XML-based call control.
//! Webhook verification uses HMAC-SHA256.
//! <https://www.plivo.com/docs/voice/>

use {
    async_trait::async_trait,
    base64::Engine,
    bytes::Bytes,
    hmac::{Hmac, Mac},
    http::HeaderMap,
    reqwest::Client,
    secrecy::{ExposeSecret, Secret},
    sha2::Sha256,
    tracing::{debug, warn},
};

use crate::{
    provider::{InitiateCallParams, InitiateCallResult, ProviderCallStatus, TelephonyProvider},
    types::{CallEndReason, CallEvent},
};

type HmacSha256 = Hmac<Sha256>;

/// Plivo provider implementation.
pub struct PlivoProvider {
    auth_id: String,
    auth_token: Secret<String>,
    client: Client,
    base_url: String,
}

impl PlivoProvider {
    pub fn new(auth_id: String, auth_token: Secret<String>) -> Self {
        let base_url = format!("https://api.plivo.com/v1/Account/{auth_id}");
        Self {
            auth_id,
            auth_token,
            client: Client::new(),
            base_url,
        }
    }

    #[cfg(test)]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    fn basic_auth(&self) -> (String, String) {
        (
            self.auth_id.clone(),
            self.auth_token.expose_secret().clone(),
        )
    }
}

impl std::fmt::Debug for PlivoProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlivoProvider")
            .field("auth_id", &self.auth_id)
            .field("auth_token", &"[REDACTED]")
            .finish()
    }
}

#[async_trait]
impl TelephonyProvider for PlivoProvider {
    fn id(&self) -> &'static str {
        "plivo"
    }

    fn name(&self) -> &'static str {
        "Plivo"
    }

    fn is_configured(&self) -> bool {
        !self.auth_id.is_empty()
    }

    async fn initiate_call(
        &self,
        params: InitiateCallParams,
    ) -> anyhow::Result<InitiateCallResult> {
        let url = format!("{}/Call/", self.base_url);
        let (user, pass) = self.basic_auth();

        let body = serde_json::json!({
            "from": normalize_number(&params.from),
            "to": normalize_number(&params.to),
            "answer_url": params.answer_url,
            "answer_method": "POST",
            "hangup_url": params.status_callback_url,
            "hangup_method": "POST",
            "hangup_on_ring": 30,
        });

        debug!(url = %url, "initiating Plivo call");

        let resp = self
            .client
            .post(&url)
            .basic_auth(&user, Some(&pass))
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Plivo API error {status}: {text}");
        }

        let json: serde_json::Value = resp.json().await?;
        let request_uuid = json["request_uuid"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|v| v.as_str())
            .or_else(|| json["request_uuid"].as_str())
            .ok_or_else(|| anyhow::anyhow!("missing request_uuid in Plivo response"))?;

        Ok(InitiateCallResult {
            provider_call_id: request_uuid.to_string(),
        })
    }

    async fn hangup_call(&self, provider_call_id: &str) -> anyhow::Result<()> {
        let url = format!("{}/Call/{provider_call_id}/", self.base_url);
        let (user, pass) = self.basic_auth();

        let resp = self
            .client
            .delete(&url)
            .basic_auth(&user, Some(&pass))
            .send()
            .await?;

        if !resp.status().is_success() && resp.status().as_u16() != 404 {
            let text = resp.text().await.unwrap_or_default();
            warn!(call_id = %provider_call_id, "Plivo hangup failed: {text}");
        }
        Ok(())
    }

    async fn play_tts(
        &self,
        provider_call_id: &str,
        text: &str,
        _voice: Option<&str>,
        _gather_url: Option<&str>,
    ) -> anyhow::Result<()> {
        // Plivo uses XML transfer for TTS. We update the call with a Speak XML URL.
        let url = format!("{}/Call/{provider_call_id}/", self.base_url);
        let (user, pass) = self.basic_auth();

        // Plivo expects a URL that returns XML; for inline TTS we use the
        // Speak API endpoint instead.
        let speak_url = format!("{}/Call/{provider_call_id}/Speak/", self.base_url);
        let resp = self
            .client
            .post(&speak_url)
            .basic_auth(&user, Some(&pass))
            .json(&serde_json::json!({
                "text": text,
                "language": "en-US",
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Plivo speak failed: {text}");
        }
        let _ = url;
        Ok(())
    }

    async fn send_dtmf(&self, provider_call_id: &str, digits: &str) -> anyhow::Result<()> {
        let url = format!("{}/Call/{provider_call_id}/DTMF/", self.base_url);
        let (user, pass) = self.basic_auth();

        let resp = self
            .client
            .post(&url)
            .basic_auth(&user, Some(&pass))
            .json(&serde_json::json!({ "digits": digits }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Plivo send_dtmf failed: {text}");
        }
        Ok(())
    }

    async fn get_call_status(&self, provider_call_id: &str) -> anyhow::Result<ProviderCallStatus> {
        let url = format!("{}/Call/{provider_call_id}/", self.base_url);
        let (user, pass) = self.basic_auth();

        let resp = self
            .client
            .get(&url)
            .basic_auth(&user, Some(&pass))
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Plivo API error {status}: {text}");
        }

        let json: serde_json::Value = resp.json().await?;
        Ok(ProviderCallStatus {
            provider_call_id: provider_call_id.to_string(),
            status: json["call_status"]
                .as_str()
                .unwrap_or("unknown")
                .to_string(),
            duration_seconds: json["duration"].as_str().and_then(|s| s.parse().ok()),
        })
    }

    fn verify_webhook(&self, url: &str, headers: &HeaderMap, body: &[u8]) -> anyhow::Result<()> {
        // Plivo v3 signature verification (HMAC-SHA256 with nonce).
        let signature = headers
            .get("X-Plivo-Signature-V3")
            .and_then(|v| v.to_str().ok());
        let nonce = headers
            .get("X-Plivo-Signature-V3-Nonce")
            .and_then(|v| v.to_str().ok());

        let (Some(signature), Some(nonce)) = (signature, nonce) else {
            // Fail closed: if auth_token is configured, require signatures.
            anyhow::bail!(
                "missing X-Plivo-Signature-V3 or nonce header — \
                 webhook signature verification failed"
            );
        };

        let body_str = std::str::from_utf8(body).unwrap_or("");
        let signed_payload = format!("{url}.{nonce}.{body_str}");

        let mut mac = HmacSha256::new_from_slice(self.auth_token.expose_secret().as_bytes())
            .map_err(|e| anyhow::anyhow!("HMAC init failed: {e}"))?;
        mac.update(signed_payload.as_bytes());

        let expected =
            base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        if expected != signature {
            anyhow::bail!("Plivo signature mismatch");
        }
        Ok(())
    }

    fn parse_webhook_event(&self, _headers: &HeaderMap, body: &[u8]) -> anyhow::Result<CallEvent> {
        // Plivo webhooks are form-urlencoded.
        let body_str = std::str::from_utf8(body)?;
        let params: std::collections::HashMap<String, String> =
            url::form_urlencoded::parse(body_str.as_bytes())
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();

        let call_uuid = params.get("CallUUID").cloned().unwrap_or_default();
        let call_status = params.get("CallStatus").map(String::as_str).unwrap_or("");

        // Check for speech recognition result first.
        if let Some(speech) = params.get("Speech")
            && !speech.is_empty()
        {
            return Ok(CallEvent::Speech {
                provider_call_id: call_uuid,
                text: speech.clone(),
                confidence: None,
            });
        }

        // Check for DTMF digits.
        if let Some(digits) = params.get("Digits")
            && !digits.is_empty()
        {
            return Ok(CallEvent::Dtmf {
                provider_call_id: call_uuid,
                digits: digits.clone(),
            });
        }

        match call_status {
            "ringing" => Ok(CallEvent::Ringing {
                provider_call_id: call_uuid,
            }),
            "in-progress" | "answered" => Ok(CallEvent::Answered {
                provider_call_id: call_uuid,
            }),
            "completed" => Ok(CallEvent::Ended {
                provider_call_id: call_uuid,
                reason: CallEndReason::Completed,
            }),
            "busy" => Ok(CallEvent::Ended {
                provider_call_id: call_uuid,
                reason: CallEndReason::Busy,
            }),
            "no-answer" => Ok(CallEvent::Ended {
                provider_call_id: call_uuid,
                reason: CallEndReason::NoAnswer,
            }),
            "failed" | "cancel" => Ok(CallEvent::Ended {
                provider_call_id: call_uuid,
                reason: CallEndReason::Error,
            }),
            other => {
                if !other.is_empty() {
                    debug!(status = %other, "unrecognized Plivo status");
                }
                // Treat as answered if we have a call UUID (Plivo answer_url callback).
                if !call_uuid.is_empty() {
                    Ok(CallEvent::Answered {
                        provider_call_id: call_uuid,
                    })
                } else {
                    anyhow::bail!("cannot parse Plivo webhook: no CallUUID or status")
                }
            },
        }
    }

    fn build_answer_response(&self, message: Option<&str>, gather_url: Option<&str>) -> Bytes {
        let mut xml = String::from(r#"<?xml version="1.0" encoding="UTF-8"?><Response>"#);

        if let Some(url) = gather_url {
            let url = xml_escape(url);
            xml.push_str(&format!(
                r#"<GetInput action="{url}" method="POST" inputType="speech" speechEndTimeout="auto">"#
            ));
            if let Some(msg) = message {
                xml.push_str(&format!(
                    r#"<Speak language="en-US">{}</Speak>"#,
                    xml_escape(msg)
                ));
            }
            xml.push_str("</GetInput>");
        } else if let Some(msg) = message {
            // Notify mode: speak, then hang up.
            xml.push_str(&format!(
                r#"<Speak language="en-US">{}</Speak><Wait length="2"/><Hangup/>"#,
                xml_escape(msg)
            ));
        }

        xml.push_str("</Response>");
        Bytes::from(xml)
    }

    fn build_gather_response(&self, prompt: Option<&str>, action_url: &str) -> Bytes {
        let mut xml = String::from(r#"<?xml version="1.0" encoding="UTF-8"?><Response>"#);
        let action_url = xml_escape(action_url);
        xml.push_str(&format!(
            r#"<GetInput action="{action_url}" method="POST" inputType="speech" speechEndTimeout="auto">"#
        ));
        if let Some(p) = prompt {
            xml.push_str(&format!(
                r#"<Speak language="en-US">{}</Speak>"#,
                xml_escape(p)
            ));
        }
        xml.push_str("</GetInput></Response>");
        Bytes::from(xml)
    }

    fn build_play_response(&self, audio_url: &str) -> Bytes {
        Bytes::from(format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><Response><Play>{audio_url}</Play></Response>"#
        ))
    }

    fn build_hangup_response(&self) -> Bytes {
        Bytes::from(r#"<?xml version="1.0" encoding="UTF-8"?><Response><Hangup/></Response>"#)
    }
}

/// Minimal XML escaping.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Normalize a phone number: strip non-digit characters except leading +.
fn normalize_number(n: &str) -> String {
    let trimmed = n.trim();
    if trimmed.starts_with("sip:") {
        return trimmed.to_string();
    }
    let mut result = String::with_capacity(trimmed.len());
    for (i, ch) in trimmed.chars().enumerate() {
        if (ch == '+' && i == 0) || ch.is_ascii_digit() {
            result.push(ch);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn serve_plivo_response(status: &str, body: &str) -> String {
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

        format!("http://{addr}/v1/Account/MA")
    }

    #[test]
    fn plivo_debug_redacts_token() {
        let provider = PlivoProvider::new("MATEST".into(), Secret::new("secret_tok".into()));
        let debug_str = format!("{provider:?}");
        assert!(!debug_str.contains("secret_tok"));
        assert!(debug_str.contains("[REDACTED]"));
    }

    #[test]
    fn parse_webhook_ringing() {
        let provider = PlivoProvider::new("MA".into(), Secret::new("tok".into()));
        let body = b"CallUUID=abc123&CallStatus=ringing";
        let event = provider
            .parse_webhook_event(&HeaderMap::new(), body)
            .unwrap_or_else(|e| panic!("{e}"));
        match event {
            CallEvent::Ringing { provider_call_id } => assert_eq!(provider_call_id, "abc123"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parse_webhook_completed() {
        let provider = PlivoProvider::new("MA".into(), Secret::new("tok".into()));
        let body = b"CallUUID=xyz&CallStatus=completed";
        let event = provider
            .parse_webhook_event(&HeaderMap::new(), body)
            .unwrap_or_else(|e| panic!("{e}"));
        match event {
            CallEvent::Ended {
                provider_call_id,
                reason,
            } => {
                assert_eq!(provider_call_id, "xyz");
                assert_eq!(reason, CallEndReason::Completed);
            },
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parse_webhook_speech() {
        let provider = PlivoProvider::new("MA".into(), Secret::new("tok".into()));
        let body = b"CallUUID=s1&Speech=hello%20world";
        let event = provider
            .parse_webhook_event(&HeaderMap::new(), body)
            .unwrap_or_else(|e| panic!("{e}"));
        match event {
            CallEvent::Speech { text, .. } => assert_eq!(text, "hello world"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parse_webhook_dtmf() {
        let provider = PlivoProvider::new("MA".into(), Secret::new("tok".into()));
        let body = b"CallUUID=d1&Digits=42";
        let event = provider
            .parse_webhook_event(&HeaderMap::new(), body)
            .unwrap_or_else(|e| panic!("{e}"));
        match event {
            CallEvent::Dtmf { digits, .. } => assert_eq!(digits, "42"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn normalize_number_strips_non_digits() {
        assert_eq!(normalize_number("+1 (555) 123-4567"), "+15551234567");
        assert_eq!(normalize_number("5551234567"), "5551234567");
        assert_eq!(normalize_number("sip:user@domain"), "sip:user@domain");
    }

    #[test]
    fn build_answer_with_gather() {
        let provider = PlivoProvider::new("MA".into(), Secret::new("tok".into()));
        let resp =
            provider.build_answer_response(Some("Hello"), Some("https://example.com/gather"));
        let xml = std::str::from_utf8(&resp).unwrap_or("");
        assert!(xml.contains("<GetInput"));
        assert!(xml.contains("Hello"));
        assert!(xml.contains("https://example.com/gather"));
    }

    #[test]
    fn build_gather_responses_escape_action_url_attributes() {
        let provider = PlivoProvider::new("MA".into(), Secret::new("tok".into()));
        let url = "https://example.com/gather?foo=1&bar=\"two\"";

        let answer = provider.build_answer_response(Some("Hello"), Some(url));
        let answer_xml = std::str::from_utf8(&answer).unwrap_or("");
        assert!(answer_xml.contains("foo=1&amp;bar=&quot;two&quot;"));
        assert!(!answer_xml.contains("foo=1&bar=\"two\""));

        let gather = provider.build_gather_response(None, url);
        let gather_xml = std::str::from_utf8(&gather).unwrap_or("");
        assert!(gather_xml.contains("foo=1&amp;bar=&quot;two&quot;"));
        assert!(!gather_xml.contains("foo=1&bar=\"two\""));
    }

    #[test]
    fn build_answer_notify_includes_hangup() {
        let provider = PlivoProvider::new("MA".into(), Secret::new("tok".into()));
        let resp = provider.build_answer_response(Some("Reminder call"), None);
        let xml = std::str::from_utf8(&resp).unwrap_or("");
        assert!(xml.contains("Reminder call"));
        assert!(xml.contains("<Hangup/>"));
        assert!(!xml.contains("<GetInput"));
    }

    #[test]
    fn verify_webhook_rejects_missing_headers() {
        let provider = PlivoProvider::new("MA".into(), Secret::new("tok".into()));
        let result = provider.verify_webhook("https://example.com", &HeaderMap::new(), b"body");
        assert!(result.is_err());
    }

    #[test]
    fn parse_webhook_busy() {
        let provider = PlivoProvider::new("MA".into(), Secret::new("tok".into()));
        let body = b"CallUUID=busy1&CallStatus=busy";
        let event = provider
            .parse_webhook_event(&HeaderMap::new(), body)
            .unwrap_or_else(|e| panic!("{e}"));
        match event {
            CallEvent::Ended { reason, .. } => assert_eq!(reason, CallEndReason::Busy),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn get_call_status_rejects_non_success_response() {
        let base_url = serve_plivo_response("401 Unauthorized", r#"{"error":"auth failed"}"#).await;
        let provider =
            PlivoProvider::new("MA".into(), Secret::new("tok".into())).with_base_url(base_url);

        let error = provider
            .get_call_status("call-123")
            .await
            .err()
            .unwrap_or_else(|| panic!("non-success response should fail"));

        let message = error.to_string();
        assert!(message.contains("Plivo API error 401"));
        assert!(message.contains("auth failed"));
    }
}
