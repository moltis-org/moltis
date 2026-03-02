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
    model: String,
}

impl std::fmt::Debug for WhisperStt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WhisperStt")
            .field("api_key", &"[REDACTED]")
            .field("model", &self.model)
            .finish()
    }
}

impl Default for WhisperStt {
    fn default() -> Self {
        Self::new(None)
    }
}

impl WhisperStt {
    /// Create a new Whisper STT provider.
    #[must_use]
    pub fn new(api_key: Option<Secret<String>>) -> Self {
        Self {
            client: Client::new(),
            api_key,
            model: DEFAULT_MODEL.into(),
        }
    }

    /// Create with custom model.
    #[must_use]
    pub fn with_model(api_key: Option<Secret<String>>, model: Option<String>) -> Self {
        Self {
            client: Client::new(),
            api_key,
            model: model.unwrap_or_else(|| DEFAULT_MODEL.into()),
        }
    }

    /// Get the API key, returning an error if not configured.
    fn get_api_key(&self) -> Result<&Secret<String>> {
        self.api_key
            .as_ref()
            .ok_or_else(|| anyhow!("OpenAI API key not configured for Whisper"))
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
            .text("response_format", "verbose_json");

        if let Some(language) = request.language {
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

        let result = provider.transcribe(request).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not configured"));
    }

    #[test]
    fn test_with_model() {
        let provider = WhisperStt::with_model(
            Some(Secret::new("key".into())),
            Some("whisper-large-v3".into()),
        );
        assert_eq!(provider.model, "whisper-large-v3");
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
