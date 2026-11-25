//! Google Gemini STT provider implementation.
//!
//! Uses Gemini's dedicated transcription model (`gemini-3.5-transcribe`)
//! through the Generative Language API with a Google AI Studio key. This is
//! distinct from [`super::GoogleStt`], which targets Google Cloud Speech.

use {
    anyhow::{Context, Result, anyhow},
    async_trait::async_trait,
    base64::Engine,
    reqwest::Client,
    secrecy::{ExposeSecret, Secret},
    serde::{Deserialize, Serialize},
    std::time::Duration,
    tracing::warn,
};

use super::{SttProvider, TranscribeRequest, Transcript};

/// Generative Language API base URL.
const API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";

/// Default Gemini transcription model.
const DEFAULT_MODEL: &str = "gemini-3.5-transcribe";

const REQUEST_TIMEOUT_SECS: u64 = 120;
const CONNECT_TIMEOUT_SECS: u64 = 10;

/// Upper bound on API error bodies echoed into errors and logs.
const MAX_ERROR_BODY_CHARS: usize = 300;

/// Google Gemini STT provider.
#[derive(Clone)]
pub struct GeminiStt {
    client: Client,
    api_key: Option<Secret<String>>,
    model: String,
    language: Option<String>,
    base_url: String,
}

impl std::fmt::Debug for GeminiStt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeminiStt")
            .field("api_key", &"[REDACTED]")
            .field("model", &self.model)
            .field("language", &self.language)
            .finish()
    }
}

impl Default for GeminiStt {
    fn default() -> Self {
        Self::new(None)
    }
}

impl GeminiStt {
    fn build_client() -> Client {
        Client::builder()
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .unwrap_or_else(|error| {
                warn!(
                    error = %error,
                    "failed to build Gemini STT HTTP client with timeouts, falling back to default client"
                );
                Client::new()
            })
    }

    /// Create a new Gemini STT provider.
    #[must_use]
    pub fn new(api_key: Option<Secret<String>>) -> Self {
        Self::with_options(api_key, None, None)
    }

    /// Create with custom model and language.
    #[must_use]
    pub fn with_options(
        api_key: Option<Secret<String>>,
        model: Option<String>,
        language: Option<String>,
    ) -> Self {
        Self {
            client: Self::build_client(),
            api_key,
            model: model
                .filter(|model| !model.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_MODEL.into()),
            language,
            base_url: API_BASE.into(),
        }
    }

    #[cfg(test)]
    #[must_use]
    fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    fn get_api_key(&self) -> Result<&Secret<String>> {
        self.api_key
            .as_ref()
            .ok_or_else(|| anyhow!("Gemini API key not configured"))
    }

    fn build_body(&self, request: &TranscribeRequest) -> GenerateContentRequest {
        let language = request
            .language
            .as_deref()
            .or(self.language.as_deref())
            .map(str::trim)
            .filter(|language| !language.is_empty());

        GenerateContentRequest {
            contents: vec![Content {
                role: "user",
                parts: vec![Part {
                    inline_data: InlineData {
                        mime_type: request.format.mime_type(),
                        data: base64::engine::general_purpose::STANDARD.encode(&request.audio),
                    },
                }],
            }],
            generation_config: language.map(|language| GenerationConfig {
                audio_transcription_config: AudioTranscriptionConfig {
                    language_codes: vec![language.to_string()],
                },
            }),
        }
    }
}

#[async_trait]
impl SttProvider for GeminiStt {
    fn id(&self) -> &'static str {
        "gemini"
    }

    fn name(&self) -> &'static str {
        "Google Gemini"
    }

    fn is_configured(&self) -> bool {
        self.api_key.is_some()
    }

    /// The request prompt is ignored: the transcription model takes audio only.
    #[tracing::instrument(
        name = "gemini_stt.transcribe",
        skip_all,
        fields(model = %self.model, format = ?request.format, audio_bytes = request.audio.len())
    )]
    async fn transcribe(&self, request: TranscribeRequest) -> Result<Transcript> {
        let api_key = self.get_api_key()?;
        let body = self.build_body(&request);

        let response = self
            .client
            .post(format!(
                "{}/models/{}:generateContent",
                self.base_url, self.model
            ))
            .header("x-goog-api-key", api_key.expose_secret())
            .json(&body)
            .send()
            .await
            .context("failed to send Gemini transcription request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let body: String = body.chars().take(MAX_ERROR_BODY_CHARS).collect();
            return Err(anyhow!(
                "Gemini transcription request failed: {status} - {body}"
            ));
        }

        let gemini_response: GenerateContentResponse = response
            .json()
            .await
            .context("failed to parse Gemini transcription response")?;

        Ok(Transcript {
            text: gemini_response.transcript(),
            language: None,
            confidence: None,
            duration_seconds: None,
            words: None,
        })
    }
}

// ── API Types ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerateContentRequest {
    contents: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GenerationConfig>,
}

#[derive(Debug, Serialize)]
struct Content {
    role: &'static str,
    parts: Vec<Part>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Part {
    inline_data: InlineData,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InlineData {
    mime_type: &'static str,
    data: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationConfig {
    audio_transcription_config: AudioTranscriptionConfig,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AudioTranscriptionConfig {
    language_codes: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GenerateContentResponse {
    #[serde(default)]
    candidates: Vec<Candidate>,
}

#[derive(Debug, Deserialize)]
struct Candidate {
    #[serde(default)]
    content: Option<CandidateContent>,
}

#[derive(Debug, Deserialize)]
struct CandidateContent {
    #[serde(default)]
    parts: Vec<ResponsePart>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponsePart {
    #[serde(default)]
    audio_transcription: Option<AudioTranscription>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AudioTranscription {
    #[serde(default)]
    text: Option<String>,
}

impl GenerateContentResponse {
    /// The transcription model answers in `audioTranscription`; plain `text`
    /// parts are accepted as a fallback for general-purpose Gemini models.
    fn transcript(self) -> String {
        self.candidates
            .into_iter()
            .next()
            .and_then(|candidate| candidate.content)
            .map(|content| {
                content
                    .parts
                    .into_iter()
                    .filter_map(|part| {
                        part.audio_transcription
                            .and_then(|transcription| transcription.text)
                            .or(part.text)
                    })
                    .collect::<String>()
            })
            .unwrap_or_default()
            .trim()
            .to_string()
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, crate::tts::AudioFormat, bytes::Bytes};

    fn request(language: Option<&str>) -> TranscribeRequest {
        TranscribeRequest {
            audio: Bytes::from_static(b"fake audio"),
            format: AudioFormat::Opus,
            language: language.map(Into::into),
            prompt: Some("ignored prompt".into()),
        }
    }

    #[test]
    fn test_provider_metadata() {
        let provider = GeminiStt::new(None);
        assert_eq!(provider.id(), "gemini");
        assert_eq!(provider.name(), "Google Gemini");
        assert!(!provider.is_configured());
        assert_eq!(provider.model, DEFAULT_MODEL);

        let configured = GeminiStt::new(Some(Secret::new("test-key".into())));
        assert!(configured.is_configured());
    }

    #[test]
    fn test_debug_redacts_api_key() {
        let provider = GeminiStt::new(Some(Secret::new("super-secret-key".into())));
        let debug_output = format!("{provider:?}");
        assert!(debug_output.contains("[REDACTED]"));
        assert!(!debug_output.contains("super-secret-key"));
    }

    #[test]
    fn test_with_options() {
        let provider = GeminiStt::with_options(
            Some(Secret::new("key".into())),
            Some("gemini-3.5-transcribe-preview".into()),
            Some("fr-FR".into()),
        );
        assert_eq!(provider.model, "gemini-3.5-transcribe-preview");
        assert_eq!(provider.language.as_deref(), Some("fr-FR"));

        let blank_model = GeminiStt::with_options(None, Some("  ".into()), None);
        assert_eq!(blank_model.model, DEFAULT_MODEL);
    }

    #[tokio::test]
    async fn test_transcribe_without_api_key() {
        let result = GeminiStt::new(None).transcribe(request(None)).await;
        assert!(result.unwrap_err().to_string().contains("not configured"));
    }

    #[test]
    fn test_body_without_language_auto_detects() {
        let body = serde_json::to_value(GeminiStt::new(None).build_body(&request(None))).unwrap();
        assert_eq!(
            body,
            serde_json::json!({
                "contents": [{
                    "role": "user",
                    "parts": [{
                        "inlineData": {
                            "mimeType": "audio/ogg",
                            "data": "ZmFrZSBhdWRpbw==",
                        }
                    }]
                }]
            })
        );
    }

    #[test]
    fn test_body_language_request_overrides_config() {
        let provider = GeminiStt::with_options(None, None, Some("en-US".into()));

        let configured = serde_json::to_value(provider.build_body(&request(None))).unwrap();
        assert_eq!(
            configured["generationConfig"]["audioTranscriptionConfig"]["languageCodes"],
            serde_json::json!(["en-US"])
        );

        let overridden = serde_json::to_value(provider.build_body(&request(Some("fr")))).unwrap();
        assert_eq!(
            overridden["generationConfig"]["audioTranscriptionConfig"]["languageCodes"],
            serde_json::json!(["fr"])
        );

        let blank = serde_json::to_value(
            GeminiStt::with_options(None, None, Some(" ".into())).build_body(&request(None)),
        )
        .unwrap();
        assert!(blank.get("generationConfig").is_none());
    }

    #[test]
    fn test_response_audio_transcription_parts_are_joined() {
        let json = r#"{
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [
                        {"audioTranscription": {"text": " Bonjour, "}},
                        {"audioTranscription": {"text": "ça va ? "}}
                    ]
                },
                "finishReason": "STOP"
            }]
        }"#;
        let response: GenerateContentResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.transcript(), "Bonjour, ça va ?");
    }

    #[test]
    fn test_response_text_part_fallback() {
        let json = r#"{"candidates": [{"content": {"parts": [{"text": "Hello world"}]}}]}"#;
        let response: GenerateContentResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.transcript(), "Hello world");
    }

    #[test]
    fn test_response_without_transcript_is_empty() {
        for json in [
            r#"{}"#,
            r#"{"candidates": []}"#,
            r#"{"candidates": [{"finishReason": "SAFETY"}]}"#,
            r#"{"candidates": [{"content": {"parts": [{}]}}]}"#,
        ] {
            let response: GenerateContentResponse = serde_json::from_str(json).unwrap();
            assert_eq!(response.transcript(), "", "{json}");
        }
    }

    mod integration {
        use {
            super::*,
            wiremock::{
                Mock, MockServer, ResponseTemplate,
                matchers::{body_partial_json, header, method, path},
            },
        };

        #[tokio::test]
        async fn test_transcribe_success() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/models/gemini-3.5-transcribe:generateContent"))
                .and(header("x-goog-api-key", "test-api-key"))
                .and(body_partial_json(serde_json::json!({
                    "contents": [{"parts": [{"inlineData": {"mimeType": "audio/ogg"}}]}]
                })))
                .respond_with(ResponseTemplate::new(200).set_body_string(
                    r#"{"candidates": [{"content": {"parts": [{"audioTranscription": {"text": "Salut, on se voit demain."}}]}}]}"#,
                ))
                .expect(1)
                .mount(&mock_server)
                .await;

            let provider = GeminiStt::new(Some(Secret::new("test-api-key".into())))
                .with_base_url(mock_server.uri());

            let result = provider.transcribe(request(None)).await.unwrap();
            assert_eq!(result.text, "Salut, on se voit demain.");
            assert!(result.language.is_none());

            let requests = mock_server.received_requests().await.unwrap();
            assert!(!requests[0].url.as_str().contains("test-api-key"));
            let body = String::from_utf8_lossy(&requests[0].body);
            assert!(!body.contains("ignored prompt"));
            assert!(!body.contains("generationConfig"));
        }

        #[tokio::test]
        async fn test_transcribe_with_language_and_custom_model() {
            let mock_server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/models/custom-transcribe:generateContent"))
                .and(body_partial_json(serde_json::json!({
                    "generationConfig": {"audioTranscriptionConfig": {"languageCodes": ["en-US"]}}
                })))
                .respond_with(ResponseTemplate::new(200).set_body_string(
                    r#"{"candidates": [{"content": {"parts": [{"audioTranscription": {"text": "See you tomorrow."}}]}}]}"#,
                ))
                .expect(1)
                .mount(&mock_server)
                .await;

            let provider = GeminiStt::with_options(
                Some(Secret::new("key".into())),
                Some("custom-transcribe".into()),
                Some("en-US".into()),
            )
            .with_base_url(mock_server.uri());

            let result = provider.transcribe(request(None)).await.unwrap();
            assert_eq!(result.text, "See you tomorrow.");
        }

        #[tokio::test]
        async fn test_transcribe_api_error_is_truncated() {
            let mock_server = MockServer::start().await;
            let long_body = format!(r#"{{"error": {{"message": "{}"}}}}"#, "x".repeat(1000));

            Mock::given(method("POST"))
                .and(path("/models/gemini-3.5-transcribe:generateContent"))
                .respond_with(ResponseTemplate::new(400).set_body_string(long_body))
                .mount(&mock_server)
                .await;

            let provider =
                GeminiStt::new(Some(Secret::new("key".into()))).with_base_url(mock_server.uri());

            let error = provider
                .transcribe(request(None))
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains("400"));
            assert!(error.len() < 400, "{error}");
        }
    }
}
