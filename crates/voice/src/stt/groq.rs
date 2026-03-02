//! Groq STT provider implementation.
//!
//! Groq provides a Whisper-compatible API endpoint with various
//! Whisper models optimized for their LPU inference hardware.

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

/// Groq API base URL.
const API_BASE: &str = "https://api.groq.com/openai/v1";

/// Default Groq Whisper model.
const DEFAULT_MODEL: &str = "whisper-large-v3-turbo";

/// Groq STT provider using Whisper-compatible API.
#[derive(Clone)]
pub struct GroqStt {
    client: Client,
    api_key: Option<Secret<String>>,
    model: String,
    language: Option<String>,
}

impl std::fmt::Debug for GroqStt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroqStt")
            .field("api_key", &"[REDACTED]")
            .field("model", &self.model)
            .field("language", &self.language)
            .finish()
    }
}

impl Default for GroqStt {
    fn default() -> Self {
        Self::new(None)
    }
}

impl GroqStt {
    /// Create a new Groq STT provider.
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
            .ok_or_else(|| anyhow!("Groq API key not configured"))
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
impl SttProvider for GroqStt {
    fn id(&self) -> &'static str {
        "groq"
    }

    fn name(&self) -> &'static str {
        "Groq"
    }

    fn is_configured(&self) -> bool {
        self.api_key.is_some()
    }

    async fn transcribe(&self, request: TranscribeRequest) -> Result<Transcript> {
        let api_key = self.get_api_key()?;

        let filename = format!("audio.{}", Self::file_extension(request.format));
        let mime_type = Self::mime_type(request.format);

        // Build multipart form (OpenAI-compatible)
        let file_part = Part::bytes(request.audio.to_vec())
            .file_name(filename)
            .mime_str(mime_type)
            .context("failed to create file part")?;

        let mut form = Form::new()
            .part("file", file_part)
            .text("model", self.model.clone())
            .text("response_format", "verbose_json");

        // Use request language if provided, otherwise fall back to configured language
        if let Some(language) = request.language.or_else(|| self.language.clone()) {
            form = form.text("language", language);
        }

        if let Some(prompt) = request.prompt {
            form = form.text("prompt", prompt);
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
            .context("failed to send Groq transcription request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Groq transcription request failed: {} - {}",
                status,
                body
            ));
        }

        let groq_response: GroqResponse = response
            .json()
            .await
            .context("failed to parse Groq response")?;

        Ok(Transcript {
            text: groq_response.text,
            language: groq_response.language,
            confidence: None,
            duration_seconds: groq_response.duration,
            words: groq_response.words.map(|words| {
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
struct GroqResponse {
    text: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    duration: Option<f32>,
    #[serde(default)]
    words: Option<Vec<GroqWord>>,
}

#[derive(Debug, Deserialize)]
struct GroqWord {
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
        let provider = GroqStt::new(None);
        assert_eq!(provider.id(), "groq");
        assert_eq!(provider.name(), "Groq");
        assert!(!provider.is_configured());

        let configured = GroqStt::new(Some(Secret::new("test-key".into())));
        assert!(configured.is_configured());
    }

    #[test]
    fn test_debug_redacts_api_key() {
        let provider = GroqStt::new(Some(Secret::new("super-secret-key".into())));
        let debug_output = format!("{:?}", provider);
        assert!(debug_output.contains("[REDACTED]"));
        assert!(!debug_output.contains("super-secret-key"));
    }

    #[test]
    fn test_with_options() {
        let provider = GroqStt::with_options(
            Some(Secret::new("key".into())),
            Some("whisper-large-v3".into()),
            Some("en".into()),
        );
        assert_eq!(provider.model, "whisper-large-v3");
        assert_eq!(provider.language, Some("en".into()));
    }

    #[tokio::test]
    async fn test_transcribe_without_api_key() {
        let provider = GroqStt::new(None);
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
    fn test_groq_response_parsing() {
        let json = r#"{
            "text": "Hello, how are you?",
            "language": "en",
            "duration": 2.5,
            "words": [
                {"word": "Hello", "start": 0.0, "end": 0.5},
                {"word": "how", "start": 0.6, "end": 0.8}
            ]
        }"#;

        let response: GroqResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.text, "Hello, how are you?");
        assert_eq!(response.language, Some("en".into()));
        assert_eq!(response.duration, Some(2.5));
        assert_eq!(response.words.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_groq_response_minimal() {
        let json = r#"{"text": "Hello"}"#;
        let response: GroqResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.text, "Hello");
        assert!(response.language.is_none());
        assert!(response.words.is_none());
    }
}
