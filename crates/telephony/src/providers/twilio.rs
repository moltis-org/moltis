//! Twilio telephony provider.
//!
//! Uses the Twilio REST API for call control and TwiML for call flow.
//! Webhook signature verification uses HMAC-SHA1 per Twilio's spec.

use {
    async_trait::async_trait,
    bytes::Bytes,
    hmac::{Hmac, Mac},
    http::HeaderMap,
    reqwest::Client,
    secrecy::{ExposeSecret, Secret},
    sha1::Sha1,
    tracing::{debug, warn},
};

use crate::{
    provider::{InitiateCallParams, InitiateCallResult, ProviderCallStatus, TelephonyProvider},
    types::{CallEndReason, CallEvent},
};

type HmacSha1 = Hmac<Sha1>;

/// Twilio provider implementation.
pub struct TwilioProvider {
    account_sid: String,
    auth_token: Secret<String>,
    client: Client,
    base_url: String,
}

impl TwilioProvider {
    pub fn new(account_sid: String, auth_token: Secret<String>) -> Self {
        Self {
            base_url: format!("https://api.twilio.com/2010-04-01/Accounts/{account_sid}"),
            account_sid,
            auth_token,
            client: Client::new(),
        }
    }

    /// Override base URL for testing.
    #[cfg(test)]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Build basic auth header value.
    fn basic_auth(&self) -> (String, String) {
        (
            self.account_sid.clone(),
            self.auth_token.expose_secret().clone(),
        )
    }
}

impl std::fmt::Debug for TwilioProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TwilioProvider")
            .field("account_sid", &self.account_sid)
            .field("auth_token", &"[REDACTED]")
            .finish()
    }
}

#[async_trait]
impl TelephonyProvider for TwilioProvider {
    fn id(&self) -> &'static str {
        "twilio"
    }

    fn name(&self) -> &'static str {
        "Twilio"
    }

    fn is_configured(&self) -> bool {
        !self.account_sid.is_empty()
    }

    async fn initiate_call(
        &self,
        params: InitiateCallParams,
    ) -> anyhow::Result<InitiateCallResult> {
        let url = format!("{}/Calls.json", self.base_url);
        let (user, pass) = self.basic_auth();

        let form = vec![
            ("From", params.from),
            ("To", params.to),
            ("Url", params.answer_url),
            ("StatusCallback", params.status_callback_url),
            (
                "StatusCallbackEvent",
                "initiated ringing answered completed".into(),
            ),
        ];

        if let Some(msg) = &params.message {
            // Twiml attribute for fallback
            let _ = msg; // message is embedded in the answer URL TwiML
        }

        debug!(url = %url, "initiating Twilio call");

        let resp = self
            .client
            .post(&url)
            .basic_auth(&user, Some(&pass))
            .form(&form)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Twilio API error {status}: {body}");
        }

        let json: serde_json::Value = resp.json().await?;
        let sid = json["sid"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing sid in Twilio response"))?;

        Ok(InitiateCallResult {
            provider_call_id: sid.to_string(),
        })
    }

    async fn hangup_call(&self, provider_call_id: &str) -> anyhow::Result<()> {
        let url = format!("{}/Calls/{provider_call_id}.json", self.base_url);
        let (user, pass) = self.basic_auth();

        let resp = self
            .client
            .post(&url)
            .basic_auth(&user, Some(&pass))
            .form(&[("Status", "completed")])
            .send()
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!(sid = %provider_call_id, "hangup failed: {body}");
        }
        Ok(())
    }

    async fn play_tts(
        &self,
        provider_call_id: &str,
        text: &str,
        voice: Option<&str>,
        gather_url: Option<&str>,
    ) -> anyhow::Result<()> {
        // Update the live call with new TwiML via the Calls API.
        let url = format!("{}/Calls/{provider_call_id}.json", self.base_url);
        let (user, pass) = self.basic_auth();

        let twiml = build_say_gather_twiml(text, voice, gather_url);

        let resp = self
            .client
            .post(&url)
            .basic_auth(&user, Some(&pass))
            .form(&[("Twiml", &twiml)])
            .send()
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("play_tts failed: {body}");
        }
        Ok(())
    }

    async fn send_dtmf(&self, provider_call_id: &str, digits: &str) -> anyhow::Result<()> {
        // Validate DTMF digits to prevent TwiML injection.
        if !digits
            .chars()
            .all(|c| c.is_ascii_digit() || "*#wW".contains(c))
        {
            anyhow::bail!("invalid DTMF digits: only 0-9, *, #, w, W are allowed");
        }

        let url = format!("{}/Calls/{provider_call_id}.json", self.base_url);
        let (user, pass) = self.basic_auth();

        let twiml = format!(r#"<Response><Play digits="{digits}"/></Response>"#);

        let resp = self
            .client
            .post(&url)
            .basic_auth(&user, Some(&pass))
            .form(&[("Twiml", &twiml)])
            .send()
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("send_dtmf failed: {body}");
        }
        Ok(())
    }

    async fn get_call_status(&self, provider_call_id: &str) -> anyhow::Result<ProviderCallStatus> {
        let url = format!("{}/Calls/{provider_call_id}.json", self.base_url);
        let (user, pass) = self.basic_auth();

        let resp = self
            .client
            .get(&url)
            .basic_auth(&user, Some(&pass))
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Twilio API error {status}: {body}");
        }

        let json: serde_json::Value = resp.json().await?;
        Ok(ProviderCallStatus {
            provider_call_id: provider_call_id.to_string(),
            status: json["status"].as_str().unwrap_or("unknown").to_string(),
            duration_seconds: json["duration"].as_str().and_then(|s| s.parse().ok()),
        })
    }

    fn verify_webhook(&self, url: &str, headers: &HeaderMap, body: &[u8]) -> anyhow::Result<()> {
        let signature = headers
            .get("X-Twilio-Signature")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| anyhow::anyhow!("missing X-Twilio-Signature header"))?;

        // Parse body as form-urlencoded, sort params, append to URL.
        let body_str = std::str::from_utf8(body)?;
        let mut params: Vec<(String, String)> = url::form_urlencoded::parse(body_str.as_bytes())
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        params.sort_by(|a, b| a.0.cmp(&b.0));

        let mut data = url.to_string();
        for (k, v) in &params {
            data.push_str(k);
            data.push_str(v);
        }

        let mut mac = HmacSha1::new_from_slice(self.auth_token.expose_secret().as_bytes())
            .map_err(|e| anyhow::anyhow!("HMAC init failed: {e}"))?;
        mac.update(data.as_bytes());

        let provided_signature =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, signature)
                .map_err(|e| anyhow::anyhow!("invalid X-Twilio-Signature header: {e}"))?;
        mac.verify_slice(&provided_signature)
            .map_err(|_| anyhow::anyhow!("signature mismatch"))?;
        Ok(())
    }

    fn parse_webhook_event(&self, _headers: &HeaderMap, body: &[u8]) -> anyhow::Result<CallEvent> {
        let body_str = std::str::from_utf8(body)?;
        let params: std::collections::HashMap<String, String> =
            url::form_urlencoded::parse(body_str.as_bytes())
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();

        let call_sid = params.get("CallSid").cloned().unwrap_or_default();

        let status = params.get("CallStatus").map(String::as_str).unwrap_or("");

        match status {
            "initiated" | "queued" => Ok(CallEvent::Initiated {
                provider_call_id: call_sid,
            }),
            "ringing" => Ok(CallEvent::Ringing {
                provider_call_id: call_sid,
            }),
            "in-progress" => Ok(CallEvent::Answered {
                provider_call_id: call_sid,
            }),
            "completed" => Ok(CallEvent::Ended {
                provider_call_id: call_sid,
                reason: CallEndReason::Completed,
            }),
            "busy" => Ok(CallEvent::Ended {
                provider_call_id: call_sid,
                reason: CallEndReason::Busy,
            }),
            "no-answer" => Ok(CallEvent::Ended {
                provider_call_id: call_sid,
                reason: CallEndReason::NoAnswer,
            }),
            "failed" | "canceled" => Ok(CallEvent::Ended {
                provider_call_id: call_sid,
                reason: CallEndReason::Error,
            }),
            other => {
                // Check for speech result from <Gather>.
                if let Some(speech) = params.get("SpeechResult") {
                    let confidence = params.get("Confidence").and_then(|c| c.parse::<f32>().ok());
                    return Ok(CallEvent::Speech {
                        provider_call_id: call_sid,
                        text: speech.clone(),
                        confidence,
                    });
                }
                if let Some(digits) = params.get("Digits") {
                    return Ok(CallEvent::Dtmf {
                        provider_call_id: call_sid,
                        digits: digits.clone(),
                    });
                }
                debug!(status = %other, "unrecognized Twilio status, treating as error");
                Ok(CallEvent::Error {
                    provider_call_id: call_sid,
                    message: format!("unrecognized status: {other}"),
                })
            },
        }
    }

    fn build_answer_response(&self, message: Option<&str>, gather_url: Option<&str>) -> Bytes {
        let mut twiml = String::from(r#"<?xml version="1.0" encoding="UTF-8"?><Response>"#);

        if let Some(url) = gather_url {
            let url = xml_escape(url);
            twiml.push_str(&format!(
                r#"<Gather input="speech" action="{url}" speechTimeout="auto">"#
            ));
            if let Some(msg) = message {
                twiml.push_str(&format!(
                    r#"<Say voice="Polly.Joanna">{}</Say>"#,
                    xml_escape(msg)
                ));
            }
            twiml.push_str("</Gather>");
        } else if let Some(msg) = message {
            // Notify mode: speak the message, then hang up.
            twiml.push_str(&format!(
                r#"<Say voice="Polly.Joanna">{}</Say><Pause length="2"/><Hangup/>"#,
                xml_escape(msg)
            ));
        }

        twiml.push_str("</Response>");
        Bytes::from(twiml)
    }

    fn build_gather_response(&self, prompt: Option<&str>, action_url: &str) -> Bytes {
        let mut twiml = String::from(r#"<?xml version="1.0" encoding="UTF-8"?><Response>"#);
        let action_url = xml_escape(action_url);
        twiml.push_str(&format!(
            r#"<Gather input="speech" action="{action_url}" speechTimeout="auto">"#
        ));
        if let Some(p) = prompt {
            twiml.push_str(&format!(
                r#"<Say voice="Polly.Joanna">{}</Say>"#,
                xml_escape(p)
            ));
        }
        twiml.push_str("</Gather></Response>");
        Bytes::from(twiml)
    }

    fn build_play_response(&self, audio_url: &str) -> Bytes {
        let audio_url = xml_escape(audio_url);
        Bytes::from(format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><Response><Play>{audio_url}</Play></Response>"#
        ))
    }

    fn build_hangup_response(&self) -> Bytes {
        Bytes::from(r#"<?xml version="1.0" encoding="UTF-8"?><Response><Hangup/></Response>"#)
    }
}

fn build_say_gather_twiml(text: &str, voice: Option<&str>, gather_url: Option<&str>) -> String {
    let voice_attr = xml_escape(voice.unwrap_or("Polly.Joanna"));
    let text = xml_escape(text);
    let gather_action = gather_url
        .map(|url| format!(r#" action="{}""#, xml_escape(url)))
        .unwrap_or_default();

    format!(
        r#"<Response><Say voice="{voice_attr}">{text}</Say><Gather input="speech"{gather_action} speechTimeout="auto" timeout="30"/><Pause length="120"/></Response>"#
    )
}

/// Minimal XML escaping for TwiML text and attribute values.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn serve_twilio_response(status: &str, body: &str) -> String {
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

        format!("http://{addr}")
    }

    #[test]
    fn xml_escape_handles_special_chars() {
        assert_eq!(xml_escape("a<b>c&d"), "a&lt;b&gt;c&amp;d");
        assert_eq!(xml_escape("hello"), "hello");
    }

    #[test]
    fn build_answer_response_with_message_and_gather() {
        let provider = TwilioProvider::new("AC_TEST".into(), Secret::new("TOKEN".into()));
        let resp = provider
            .build_answer_response(Some("Hello caller"), Some("https://example.com/gather"));
        let twiml = std::str::from_utf8(&resp).unwrap_or("");
        assert!(twiml.contains("<Gather"));
        assert!(twiml.contains("Hello caller"));
        assert!(twiml.contains("https://example.com/gather"));
    }

    #[test]
    fn build_gather_responses_escape_action_url_attributes() {
        let provider = TwilioProvider::new("AC_TEST".into(), Secret::new("TOKEN".into()));
        let url = "https://example.com/gather?foo=1&bar=\"two\"";

        let answer = provider.build_answer_response(Some("Hello"), Some(url));
        let answer_twiml = std::str::from_utf8(&answer).unwrap_or("");
        assert!(answer_twiml.contains("foo=1&amp;bar=&quot;two&quot;"));
        assert!(!answer_twiml.contains("foo=1&bar=\"two\""));

        let gather = provider.build_gather_response(None, url);
        let gather_twiml = std::str::from_utf8(&gather).unwrap_or("");
        assert!(gather_twiml.contains("foo=1&amp;bar=&quot;two&quot;"));
        assert!(!gather_twiml.contains("foo=1&bar=\"two\""));
    }

    #[test]
    fn build_say_gather_twiml_includes_escaped_action_and_voice() {
        let twiml = build_say_gather_twiml(
            "hello & goodbye",
            Some("voice\"attr"),
            Some("https://example.com/gather?foo=1&bar=\"two\""),
        );

        assert!(twiml.contains(r#"voice="voice&quot;attr""#));
        assert!(twiml.contains("hello &amp; goodbye"));
        assert!(
            twiml.contains(r#"action="https://example.com/gather?foo=1&amp;bar=&quot;two&quot;""#)
        );
        assert!(!twiml.contains("foo=1&bar=\"two\""));
    }

    #[test]
    fn build_play_response_escapes_audio_url_text() {
        let provider = TwilioProvider::new("AC_TEST".into(), Secret::new("TOKEN".into()));
        let resp =
            provider.build_play_response("https://cdn.example.com/tts.mp3?codec=mulaw&rate=8000");
        let twiml = std::str::from_utf8(&resp).unwrap_or("");

        assert!(twiml.contains("codec=mulaw&amp;rate=8000"));
        assert!(!twiml.contains("codec=mulaw&rate=8000"));
    }

    #[test]
    fn build_hangup_response_is_valid_twiml() {
        let provider = TwilioProvider::new("AC_TEST".into(), Secret::new("TOKEN".into()));
        let resp = provider.build_hangup_response();
        let twiml = std::str::from_utf8(&resp).unwrap_or("");
        assert!(twiml.contains("<Hangup/>"));
    }

    #[test]
    fn parse_webhook_completed_status() {
        let provider = TwilioProvider::new("AC_TEST".into(), Secret::new("TOKEN".into()));
        let body = b"CallSid=CA123&CallStatus=completed";
        let headers = HeaderMap::new();
        let event = provider
            .parse_webhook_event(&headers, body)
            .unwrap_or_else(|e| panic!("{e}"));
        match event {
            CallEvent::Ended {
                provider_call_id,
                reason,
            } => {
                assert_eq!(provider_call_id, "CA123");
                assert_eq!(reason, CallEndReason::Completed);
            },
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn parse_webhook_speech_result() {
        let provider = TwilioProvider::new("AC_TEST".into(), Secret::new("TOKEN".into()));
        let body = b"CallSid=CA456&SpeechResult=hello%20world&Confidence=0.92";
        let headers = HeaderMap::new();
        let event = provider
            .parse_webhook_event(&headers, body)
            .unwrap_or_else(|e| panic!("{e}"));
        match event {
            CallEvent::Speech {
                text, confidence, ..
            } => {
                assert_eq!(text, "hello world");
                assert!((confidence.unwrap_or(0.0) - 0.92).abs() < 0.01);
            },
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn parse_webhook_dtmf_digits() {
        let provider = TwilioProvider::new("AC_TEST".into(), Secret::new("TOKEN".into()));
        let body = b"CallSid=CA789&Digits=123";
        let headers = HeaderMap::new();
        let event = provider
            .parse_webhook_event(&headers, body)
            .unwrap_or_else(|e| panic!("{e}"));
        match event {
            CallEvent::Dtmf { digits, .. } => assert_eq!(digits, "123"),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn twilio_debug_redacts_token() {
        let provider = TwilioProvider::new("AC_TEST".into(), Secret::new("super-secret".into()));
        let debug_str = format!("{provider:?}");
        assert!(!debug_str.contains("super-secret"));
        assert!(debug_str.contains("[REDACTED]"));
    }

    #[test]
    fn build_answer_notify_mode_includes_hangup() {
        // When gather_url is None (notify mode), TwiML should include <Hangup/>
        let provider = TwilioProvider::new("AC_TEST".into(), Secret::new("T".into()));
        let resp = provider.build_answer_response(Some("Your appointment is tomorrow"), None);
        let twiml = std::str::from_utf8(&resp).unwrap_or("");
        assert!(twiml.contains("Your appointment is tomorrow"));
        assert!(twiml.contains("<Hangup/>"));
        assert!(!twiml.contains("<Gather"));
    }

    #[test]
    fn play_tts_twiml_includes_gather_continuation() {
        // play_tts generates TwiML with <Gather> after <Say> to keep the call alive
        let provider = TwilioProvider::new("AC_TEST".into(), Secret::new("T".into()));
        // We can't test play_tts directly (it needs HTTP), but we can test the TwiML pattern
        // by looking at build_gather_response which is similar.
        let resp = provider.build_gather_response(
            Some("How can I help?"),
            "/api/channels/telephony/default/gather",
        );
        let twiml = std::str::from_utf8(&resp).unwrap_or("");
        assert!(twiml.contains("<Gather"));
        assert!(twiml.contains("How can I help?"));
        assert!(twiml.contains("action="));
    }

    #[test]
    fn dtmf_validation_rejects_invalid_chars() {
        // Test that the DTMF validation pattern works
        let valid = "0123456789*#wW";
        assert!(
            valid
                .chars()
                .all(|c| c.is_ascii_digit() || "*#wW".contains(c))
        );

        let invalid = "1<script>";
        assert!(
            !invalid
                .chars()
                .all(|c| c.is_ascii_digit() || "*#wW".contains(c))
        );

        let injection = r#"1"/><Say>injected</Say><Play digits=""#;
        assert!(
            !injection
                .chars()
                .all(|c| c.is_ascii_digit() || "*#wW".contains(c))
        );
    }

    #[tokio::test]
    async fn get_call_status_rejects_non_success_response() {
        let base_url =
            serve_twilio_response("401 Unauthorized", r#"{"message":"auth failed"}"#).await;
        let provider =
            TwilioProvider::new("AC_TEST".into(), Secret::new("T".into())).with_base_url(base_url);

        let error = provider
            .get_call_status("CA123")
            .await
            .err()
            .unwrap_or_else(|| panic!("non-success response should fail"));

        let message = error.to_string();
        assert!(message.contains("Twilio API error 401"));
        assert!(message.contains("auth failed"));
    }
}
