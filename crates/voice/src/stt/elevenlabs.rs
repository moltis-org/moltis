//! ElevenLabs Scribe STT provider implementation.
//!
//! ElevenLabs Scribe provides high-quality speech-to-text with support for
//! 90+ languages, word-level timestamps, and speaker diarization.

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

/// ElevenLabs API base URL.
const API_BASE: &str = "https://api.elevenlabs.io/v1";

/// Default model (Scribe v1 for best quality).
const DEFAULT_MODEL: &str = "scribe_v1";

/// ElevenLabs Scribe STT provider.
#[derive(Clone)]
pub struct ElevenLabsStt {
    client: Client,
    api_key: Option<Secret<String>>,
    model: String,
    language: Option<String>,
}

impl std::fmt::Debug for ElevenLabsStt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ElevenLabsStt")
            .field("api_key", &"[REDACTED]")
            .field("model", &self.model)
            .field("language", &self.language)
            .finish()
    }
}

impl Default for ElevenLabsStt {
    fn default() -> Self {
        Self::new(None)
    }
}

impl ElevenLabsStt {
    /// Create a new ElevenLabs Scribe STT provider.
    #[must_use]
    pub fn new(api_key: Option<Secret<String>>) -> Self {
        Self {
            client: Client::new(),
            api_key,
            model: DEFAULT_MODEL.into(),
            language: None,
        }
    }

    /// Create with custom options.
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
            .ok_or_else(|| anyhow!("ElevenLabs API key not configured"))
    }

    /// Get file extension for audio format.
    fn file_extension(format: AudioFormat) -> &'static str {
        match format {
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Opus => "opus",
            AudioFormat::Aac => "aac",
            AudioFormat::Pcm => "wav",
        }
    }
}

#[async_trait]
impl SttProvider for ElevenLabsStt {
    fn id(&self) -> &'static str {
        "elevenlabs"
    }

    fn name(&self) -> &'static str {
        "ElevenLabs Scribe"
    }

    fn is_configured(&self) -> bool {
        self.api_key.is_some()
    }

    async fn transcribe(&self, request: TranscribeRequest) -> Result<Transcript> {
        let api_key = self.get_api_key()?;

        // Build multipart form
        let file_ext = Self::file_extension(request.format);
        let file_part = Part::bytes(request.audio.to_vec())
            .file_name(format!("audio.{file_ext}"))
            .mime_str(request.format.mime_type())
            .context("invalid mime type")?;

        let mut form = Form::new()
            .part("file", file_part)
            .text("model", self.model.clone());

        // Use request language if provided, otherwise fall back to configured language
        if let Some(language) = request.language.as_ref().or(self.language.as_ref()) {
            form = form.text("language", language.clone());
        }

        let url = format!("{API_BASE}/speech-to-text");

        let response = self
            .client
            .post(&url)
            .header("xi-api-key", api_key.expose_secret())
            .multipart(form)
            .send()
            .await
            .context("failed to send ElevenLabs transcription request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "ElevenLabs transcription request failed: {} - {}",
                status,
                body
            ));
        }

        let el_response: ElevenLabsResponse = response
            .json()
            .await
            .context("failed to parse ElevenLabs response")?;

        Ok(Transcript {
            text: el_response.text,
            language: el_response.language_code,
            confidence: el_response.language_probability,
            duration_seconds: None, // Not provided by ElevenLabs API
            words: el_response.words.map(|words| {
                words
                    .into_iter()
                    .map(|w| Word {
                        word: w.text,
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
struct ElevenLabsResponse {
    text: String,
    #[serde(default)]
    language_code: Option<String>,
    #[serde(default)]
    language_probability: Option<f32>,
    #[serde(default)]
    words: Option<Vec<ElevenLabsWord>>,
}

#[derive(Debug, Deserialize)]
struct ElevenLabsWord {
    text: String,
    start: f32,
    end: f32,
}

#[cfg(test)]
mod tests {
    use {super::*, bytes::Bytes};

    #[test]
    fn test_provider_metadata() {
        let provider = ElevenLabsStt::new(None);
        assert_eq!(provider.id(), "elevenlabs");
        assert_eq!(provider.name(), "ElevenLabs Scribe");
        assert!(!provider.is_configured());

        let configured = ElevenLabsStt::new(Some(Secret::new("test-key".into())));
        assert!(configured.is_configured());
    }

    #[test]
    fn test_debug_redacts_api_key() {
        let provider = ElevenLabsStt::new(Some(Secret::new("super-secret-key".into())));
        let debug_output = format!("{:?}", provider);
        assert!(debug_output.contains("[REDACTED]"));
        assert!(!debug_output.contains("super-secret-key"));
    }

    #[test]
    fn test_with_options() {
        let provider = ElevenLabsStt::with_options(
            Some(Secret::new("key".into())),
            Some("scribe_v2".into()),
            Some("en".into()),
        );
        assert_eq!(provider.model, "scribe_v2");
        assert_eq!(provider.language, Some("en".into()));
    }

    #[tokio::test]
    async fn test_transcribe_without_api_key() {
        let provider = ElevenLabsStt::new(None);
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
    fn test_elevenlabs_response_parsing() {
        let json = r#"{
            "text": "Hello, how are you?",
            "language_code": "en",
            "language_probability": 0.95,
            "words": [
                {"text": "Hello", "start": 0.0, "end": 0.5, "type": "word"},
                {"text": ",", "start": 0.5, "end": 0.55, "type": "punctuation"},
                {"text": "how", "start": 0.6, "end": 0.8, "type": "word"}
            ]
        }"#;

        let response: ElevenLabsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.text, "Hello, how are you?");
        assert_eq!(response.language_code, Some("en".into()));
        assert_eq!(response.language_probability, Some(0.95));
        assert_eq!(response.words.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn test_elevenlabs_response_minimal() {
        let json = r#"{
            "text": "Hello"
        }"#;
        let response: ElevenLabsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.text, "Hello");
        assert!(response.language_code.is_none());
        assert!(response.language_probability.is_none());
        assert!(response.words.is_none());
    }

    #[test]
    fn test_file_extension() {
        assert_eq!(ElevenLabsStt::file_extension(AudioFormat::Mp3), "mp3");
        assert_eq!(ElevenLabsStt::file_extension(AudioFormat::Opus), "opus");
        assert_eq!(ElevenLabsStt::file_extension(AudioFormat::Aac), "aac");
        assert_eq!(ElevenLabsStt::file_extension(AudioFormat::Pcm), "wav");
    }
}
