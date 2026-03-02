//! Mistral AI STT provider implementation.
//!
//! Mistral's Voxtral Transcribe provides fast and accurate speech-to-text
//! with support for 13 languages and optional speaker diarization.

use {
    anyhow::{Context, Result, anyhow},
    async_trait::async_trait,
    reqwest::{
        Client,
        multipart::{Form, Part},
    },
    secrecy::{ExposeSecret, Secret},
    serde::Deserialize,
};

use {
    super::{SttProvider, TranscribeRequest, Transcript, Word},
    crate::tts::AudioFormat,
};

/// Mistral API base URL.
const API_BASE: &str = "https://api.mistral.ai/v1";

/// Default Mistral transcription model.
const DEFAULT_MODEL: &str = "voxtral-mini-latest";

/// Mistral AI STT provider using Voxtral Transcribe.
#[derive(Clone)]
pub struct MistralStt {
    client: Client,
    api_key: Option<Secret<String>>,
    model: String,
    language: Option<String>,
}

impl std::fmt::Debug for MistralStt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MistralStt")
            .field("api_key", &"[REDACTED]")
            .field("model", &self.model)
            .field("language", &self.language)
            .finish()
    }
}

impl Default for MistralStt {
    fn default() -> Self {
        Self::new(None)
    }
}

impl MistralStt {
    /// Create a new Mistral STT provider.
    #[must_use]
    pub fn new(api_key: Option<Secret<String>>) -> Self {
        Self {
            client: Client::new(),
            api_key,
            model: DEFAULT_MODEL.into(),
            language: None,
        }
    }

    /// Create with custom model and language.
    #[must_use]
    pub fn with_options(
        api_key: Option<Secret<String>>,
        model: Option<String>,
        language: Option<String>,
    ) -> Self {
        Self {
            client: Client::new(),
            api_key,
            model: model.unwrap_or_else(|| DEFAULT_MODEL.into()),
            language,
        }
    }

    /// Get the API key, returning an error if not configured.
    fn get_api_key(&self) -> Result<&Secret<String>> {
        self.api_key
            .as_ref()
            .ok_or_else(|| anyhow!("Mistral API key not configured"))
    }

    /// Get file extension for audio format.
    fn file_extension(format: AudioFormat) -> &'static str {
        format.extension()
    }

    /// Get MIME type for audio format.
    fn mime_type(format: AudioFormat) -> &'static str {
        format.mime_type()
    }
}

#[async_trait]
impl SttProvider for MistralStt {
    fn id(&self) -> &'static str {
        "mistral"
    }

    fn name(&self) -> &'static str {
        "Mistral AI"
    }

    fn is_configured(&self) -> bool {
        self.api_key.is_some()
    }

    async fn transcribe(&self, request: TranscribeRequest) -> Result<Transcript> {
        let api_key = self.get_api_key()?;

        let filename = format!("audio.{}", Self::file_extension(request.format));
        let mime_type = Self::mime_type(request.format);

        // Build multipart form
        let file_part = Part::bytes(request.audio.to_vec())
            .file_name(filename)
            .mime_str(mime_type)
            .context("failed to create file part")?;

        let mut form = Form::new()
            .part("file", file_part)
            .text("model", self.model.clone())
            .text("response_format", "verbose_json")
            .text("timestamp_granularities", "word");

        // Use request language if provided, otherwise fall back to configured language
        if let Some(language) = request.language.or_else(|| self.language.clone()) {
            form = form.text("language", language);
        }

        let response = self
            .client
            .post(format!("{API_BASE}/audio/transcriptions"))
            .header(
                "Authorization",
                format!("Bearer {}", api_key.expose_secret()),
            )
            .multipart(form)
            .send()
            .await
            .context("failed to send Mistral transcription request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Mistral transcription request failed: {} - {}",
                status,
                body
            ));
        }

        let mistral_response: MistralResponse = response
            .json()
            .await
            .context("failed to parse Mistral response")?;

        Ok(Transcript {
            text: mistral_response.text,
            language: mistral_response.language,
            confidence: None,
            duration_seconds: mistral_response.duration,
            words: mistral_response.words.map(|words| {
                words
                    .into_iter()
                    .map(|w| Word {
                        word: w.word,
                        start: w.start,
                        end: w.end,
                    })
                    .collect()
            }),
        })
    }
}

// ── API Types ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct MistralResponse {
    text: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    duration: Option<f32>,
    #[serde(default)]
    words: Option<Vec<MistralWord>>,
}

#[derive(Debug, Deserialize)]
struct MistralWord {
    word: String,
    start: f32,
    end: f32,
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, bytes::Bytes};

    #[test]
    fn test_provider_metadata() {
        let provider = MistralStt::new(None);
        assert_eq!(provider.id(), "mistral");
        assert_eq!(provider.name(), "Mistral AI");
        assert!(!provider.is_configured());

        let configured = MistralStt::new(Some(Secret::new("test-key".into())));
        assert!(configured.is_configured());
    }

    #[test]
    fn test_debug_redacts_api_key() {
        let provider = MistralStt::new(Some(Secret::new("super-secret-key".into())));
        let debug_output = format!("{:?}", provider);
        assert!(debug_output.contains("[REDACTED]"));
        assert!(!debug_output.contains("super-secret-key"));
    }

    #[test]
    fn test_with_options() {
        let provider = MistralStt::with_options(
            Some(Secret::new("key".into())),
            Some("voxtral-mini-2602".into()),
            Some("en".into()),
        );
        assert_eq!(provider.model, "voxtral-mini-2602");
        assert_eq!(provider.language, Some("en".into()));
    }

    #[tokio::test]
    async fn test_transcribe_without_api_key() {
        let provider = MistralStt::new(None);
        let request = TranscribeRequest {
            audio: Bytes::from_static(b"fake audio"),
            format: AudioFormat::Mp3,
            language: None,
            prompt: None,
        };

        let result = provider.transcribe(request).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not configured"));
    }

    #[test]
    fn test_mistral_response_parsing() {
        let json = r#"{
            "text": "Hello, how are you?",
            "language": "en",
            "duration": 2.5,
            "words": [
                {"word": "Hello", "start": 0.0, "end": 0.5},
                {"word": "how", "start": 0.6, "end": 0.8}
            ]
        }"#;

        let response: MistralResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.text, "Hello, how are you?");
        assert_eq!(response.language, Some("en".into()));
        assert_eq!(response.duration, Some(2.5));
        assert_eq!(response.words.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_mistral_response_minimal() {
        let json = r#"{"text": "Hello"}"#;
        let response: MistralResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.text, "Hello");
        assert!(response.language.is_none());
        assert!(response.words.is_none());
    }
}
