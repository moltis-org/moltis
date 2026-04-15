//! OpenAI Whisper STT provider implementation.
//!
//! Whisper is a general-purpose speech recognition model that handles
//! accents, background noise, and technical language well.

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

/// OpenAI API base URL.
const API_BASE: &str = "https://api.openai.com/v1";

/// Default Whisper model.
const DEFAULT_MODEL: &str = "whisper-1";

/// OpenAI Whisper STT provider.
#[derive(Clone)]
pub struct WhisperStt {
    client: Client,
    api_key: Option<Secret<String>>,
    base_url: String,
    model: String,
    language: Option<String>,
}

impl std::fmt::Debug for WhisperStt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WhisperStt")
            .field("api_key", &"[REDACTED]")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("language", &self.language)
            .finish()
    }
}

impl Default for WhisperStt {
    fn default() -> Self {
        Self::new(None)
    }
}

impl WhisperStt {
    fn normalize_base_url(base_url: Option<String>) -> String {
        base_url
            .map(|url| url.trim_end_matches('/').to_string())
            .unwrap_or_else(|| API_BASE.into())
    }

    /// Create a new Whisper STT provider.
    #[must_use]
    pub fn new(api_key: Option<Secret<String>>) -> Self {
        Self::with_options(api_key, None, None, None)
    }

    /// Create with custom model (no base URL or language override).
    #[must_use]
    pub fn with_model(api_key: Option<Secret<String>>, model: Option<String>) -> Self {
        Self::with_options(api_key, None, model, None)
    }

    /// Create with custom base URL, model, and language.
    #[must_use]
    pub fn with_options(
        api_key: Option<Secret<String>>,
        base_url: Option<String>,
        model: Option<String>,
        language: Option<String>,
    ) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: Self::normalize_base_url(base_url),
            model: model.unwrap_or_else(|| DEFAULT_MODEL.into()),
            language,
        }
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
impl SttProvider for WhisperStt {
    fn id(&self) -> &'static str {
        "whisper"
    }

    fn name(&self) -> &'static str {
        "OpenAI Whisper"
    }

    fn is_configured(&self) -> bool {
        // Configured if API key is set, or if using a custom base URL (local servers
        // like faster-whisper-server don't require auth).
        self.api_key.is_some() || self.base_url != API_BASE
    }

    async fn transcribe(&self, request: TranscribeRequest) -> Result<Transcript> {
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
            .text("response_format", "verbose_json");

        // Request language overrides the configured language, otherwise fall back.
        if let Some(language) = request.language.or_else(|| self.language.clone()) {
            form = form.text("language", language);
        }

        if let Some(prompt) = request.prompt {
            form = form.text("prompt", prompt);
        }

        let mut req = self
            .client
            .post(format!("{}/audio/transcriptions", self.base_url))
            .multipart(form);

        // Only add auth header if an API key is configured (local servers skip auth).
        if let Some(api_key) = &self.api_key {
            req = req.header(
                "Authorization",
                format!("Bearer {}", api_key.expose_secret()),
            );
        }

        let response = req
            .send()
            .await
            .context("failed to send Whisper transcription request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Whisper transcription request failed: {} - {}",
                status,
                body
            ));
        }

        let whisper_response: WhisperResponse = response
            .json()
            .await
            .context("failed to parse Whisper response")?;

        Ok(Transcript {
            text: whisper_response.text,
            language: whisper_response.language,
            confidence: None, // Whisper doesn't return overall confidence
            duration_seconds: whisper_response.duration,
            words: whisper_response.words.map(|words| {
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
struct WhisperResponse {
    text: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    duration: Option<f32>,
    #[serde(default)]
    words: Option<Vec<WhisperWord>>,
}

#[derive(Debug, Deserialize)]
struct WhisperWord {
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
        let provider = WhisperStt::new(None);
        assert_eq!(provider.id(), "whisper");
        assert_eq!(provider.name(), "OpenAI Whisper");
        assert!(!provider.is_configured());

        let configured = WhisperStt::new(Some(Secret::new("test-key".into())));
        assert!(configured.is_configured());
    }

    #[test]
    fn test_debug_redacts_api_key() {
        let provider = WhisperStt::new(Some(Secret::new("super-secret-key".into())));
        let debug_output = format!("{:?}", provider);
        assert!(debug_output.contains("[REDACTED]"));
        assert!(!debug_output.contains("super-secret-key"));
    }

    #[test]
    fn test_file_extension() {
        assert_eq!(WhisperStt::file_extension(AudioFormat::Mp3), "mp3");
        assert_eq!(WhisperStt::file_extension(AudioFormat::Opus), "ogg");
    }

    #[tokio::test]
    async fn test_transcribe_without_api_key() {
        let provider = WhisperStt::new(None);
        let request = TranscribeRequest {
            audio: Bytes::from_static(b"fake audio"),
            format: AudioFormat::Mp3,
            language: None,
            prompt: None,
        };

        // Without API key and default base URL, the request will fail
        // (either connection refused to api.openai.com or auth error).
        let result = provider.transcribe(request).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_with_options() {
        let provider = WhisperStt::with_options(
            Some(Secret::new("key".into())),
            None,
            Some("whisper-large-v3".into()),
            None,
        );
        assert_eq!(provider.model, "whisper-large-v3");
        assert_eq!(provider.base_url, API_BASE);
        assert!(provider.language.is_none());
    }

    #[test]
    fn test_with_custom_base_url() {
        let provider =
            WhisperStt::with_options(None, Some("http://10.1.2.30:8001".into()), None, None);
        assert!(provider.is_configured());
        assert_eq!(provider.base_url, "http://10.1.2.30:8001");
    }

    #[test]
    fn test_with_custom_base_url_trims_trailing_slash() {
        let provider =
            WhisperStt::with_options(None, Some("http://10.1.2.30:8001/".into()), None, None);
        assert_eq!(provider.base_url, "http://10.1.2.30:8001");
    }

    #[test]
    fn test_with_options_sets_model_and_language() {
        let provider = WhisperStt::with_options(
            Some(Secret::new("key".into())),
            None,
            Some("whisper-large-v3".into()),
            Some("ru".into()),
        );
        assert_eq!(provider.model, "whisper-large-v3");
        assert_eq!(provider.language, Some("ru".into()));
    }

    #[test]
    fn test_with_options_defaults() {
        let provider = WhisperStt::with_options(Some(Secret::new("key".into())), None, None, None);
        assert_eq!(provider.model, DEFAULT_MODEL);
        assert!(provider.language.is_none());
    }

    #[test]
    fn test_new_delegates_to_with_options() {
        let provider = WhisperStt::new(Some(Secret::new("key".into())));
        assert_eq!(provider.model, DEFAULT_MODEL);
        assert!(provider.language.is_none());
    }

    #[test]
    fn test_debug_includes_language() {
        let provider = WhisperStt::with_options(
            Some(Secret::new("super-secret-key".into())),
            None,
            Some("whisper-large-v3".into()),
            Some("ru".into()),
        );
        let debug_output = format!("{:?}", provider);
        assert!(debug_output.contains("[REDACTED]"));
        assert!(!debug_output.contains("super-secret-key"));
        assert!(debug_output.contains("whisper-large-v3"));
        assert!(debug_output.contains("ru"));
    }

    #[test]
    fn test_whisper_response_parsing() {
        let json = r#"{
            "text": "Hello, how are you?",
            "language": "en",
            "duration": 2.5,
            "words": [
                {"word": "Hello", "start": 0.0, "end": 0.5},
                {"word": "how", "start": 0.6, "end": 0.8},
                {"word": "are", "start": 0.9, "end": 1.0},
                {"word": "you", "start": 1.1, "end": 1.3}
            ]
        }"#;

        let response: WhisperResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.text, "Hello, how are you?");
        assert_eq!(response.language, Some("en".into()));
        assert_eq!(response.duration, Some(2.5));
        assert_eq!(response.words.as_ref().unwrap().len(), 4);
    }

    #[test]
    fn test_whisper_response_minimal() {
        // Test with minimal response (only text)
        let json = r#"{"text": "Hello"}"#;
        let response: WhisperResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.text, "Hello");
        assert!(response.language.is_none());
        assert!(response.duration.is_none());
        assert!(response.words.is_none());
    }
}
