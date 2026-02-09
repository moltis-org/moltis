//! Local Voxtral STT provider via vLLM server.
//!
//! Connects to a locally running vLLM server serving the Voxtral model.
//! The server exposes an OpenAI-compatible transcription endpoint.
//!
//! Setup:
//! ```bash
//! pip install "vllm[audio]"
//! vllm serve mistralai/Voxtral-Mini-3B-2507 \
//!   --tokenizer_mode mistral --config_format mistral --load_format mistral
//! ```

use {
    anyhow::{Context, Result, anyhow},
    async_trait::async_trait,
    reqwest::{
        Client,
        multipart::{Form, Part},
    },
    serde::Deserialize,
};

use {
    super::{SttProvider, TranscribeRequest, Transcript, Word},
    crate::tts::AudioFormat,
};

/// Default vLLM server endpoint.
const DEFAULT_ENDPOINT: &str = "http://localhost:8000";

/// Local Voxtral STT provider via vLLM.
#[derive(Clone, Debug)]
pub struct VoxtralLocalStt {
    client: Client,
    endpoint: String,
    model: Option<String>,
    language: Option<String>,
}

impl Default for VoxtralLocalStt {
    fn default() -> Self {
        Self::new()
    }
}

impl VoxtralLocalStt {
    /// Create a new local Voxtral STT provider with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            endpoint: DEFAULT_ENDPOINT.into(),
            model: None,
            language: None,
        }
    }

    /// Create with custom options.
    #[must_use]
    pub fn with_options(
        endpoint: Option<String>,
        model: Option<String>,
        language: Option<String>,
    ) -> Self {
        Self {
            client: Client::new(),
            endpoint: endpoint.unwrap_or_else(|| DEFAULT_ENDPOINT.into()),
            model,
            language,
        }
    }

    /// Check if the vLLM server is reachable.
    async fn check_server(&self) -> bool {
        let health_url = format!("{}/health", self.endpoint);
        self.client
            .get(&health_url)
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
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
impl SttProvider for VoxtralLocalStt {
    fn id(&self) -> &'static str {
        "voxtral-local"
    }

    fn name(&self) -> &'static str {
        "Voxtral (Local)"
    }

    fn is_configured(&self) -> bool {
        // We can't do async check in is_configured, so we require explicit configuration.
        // The user must either set a non-default endpoint or specify a model.
        // The actual server check happens at transcription time.
        self.model.is_some() || self.endpoint != DEFAULT_ENDPOINT
    }

    async fn transcribe(&self, request: TranscribeRequest) -> Result<Transcript> {
        // Check server availability
        if !self.check_server().await {
            return Err(anyhow!(
                "vLLM server not reachable at {}. Start it with: vllm serve mistralai/Voxtral-Mini-3B-2507 --tokenizer_mode mistral --config_format mistral --load_format mistral",
                self.endpoint
            ));
        }

        let filename = format!("audio.{}", Self::file_extension(request.format));
        let mime_type = Self::mime_type(request.format);

        // Build multipart form (OpenAI-compatible format)
        let file_part = Part::bytes(request.audio.to_vec())
            .file_name(filename)
            .mime_str(mime_type)
            .context("failed to create file part")?;

        let mut form = Form::new()
            .part("file", file_part)
            .text("response_format", "verbose_json");

        // Add model if specified
        if let Some(ref model) = self.model {
            form = form.text("model", model.clone());
        }

        // Use request language if provided, otherwise fall back to configured language
        if let Some(language) = request.language.or_else(|| self.language.clone()) {
            form = form.text("language", language);
        }

        let url = format!("{}/v1/audio/transcriptions", self.endpoint);
        let response = self
            .client
            .post(&url)
            .multipart(form)
            .send()
            .await
            .context("failed to send request to vLLM server")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "vLLM transcription request failed: {} - {}",
                status,
                body
            ));
        }

        let vllm_response: VllmResponse = response
            .json()
            .await
            .context("failed to parse vLLM response")?;

        Ok(Transcript {
            text: vllm_response.text,
            language: vllm_response.language,
            confidence: None,
            duration_seconds: vllm_response.duration,
            words: vllm_response.words.map(|words| {
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

// ── API Types (OpenAI-compatible) ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct VllmResponse {
    text: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    duration: Option<f32>,
    #[serde(default)]
    words: Option<Vec<VllmWord>>,
}

#[derive(Debug, Deserialize)]
struct VllmWord {
    word: String,
    start: f32,
    end: f32,
}

#[cfg(test)]
mod tests {
    use {super::*, bytes::Bytes};

    #[test]
    fn test_provider_metadata() {
        let provider = VoxtralLocalStt::new();
        assert_eq!(provider.id(), "voxtral-local");
        assert_eq!(provider.name(), "Voxtral (Local)");
        // Not configured by default (requires explicit model or non-default endpoint)
        assert!(!provider.is_configured());
    }

    #[test]
    fn test_is_configured_with_model() {
        let provider = VoxtralLocalStt::with_options(None, Some("my-model".into()), None);
        assert!(provider.is_configured());
    }

    #[test]
    fn test_is_configured_with_custom_endpoint() {
        let provider =
            VoxtralLocalStt::with_options(Some("http://localhost:9000".into()), None, None);
        assert!(provider.is_configured());
    }

    #[test]
    fn test_with_options() {
        let provider = VoxtralLocalStt::with_options(
            Some("http://localhost:9000".into()),
            Some("mistralai/Voxtral-Mini-3B-2507".into()),
            Some("en".into()),
        );
        assert_eq!(provider.endpoint, "http://localhost:9000");
        assert_eq!(
            provider.model,
            Some("mistralai/Voxtral-Mini-3B-2507".into())
        );
        assert_eq!(provider.language, Some("en".into()));
    }

    #[test]
    fn test_default_endpoint() {
        let provider = VoxtralLocalStt::new();
        assert_eq!(provider.endpoint, "http://localhost:8000");
    }

    #[tokio::test]
    async fn test_transcribe_server_not_running() {
        let provider = VoxtralLocalStt::with_options(
            Some("http://localhost:59999".into()), // Unlikely to be in use
            None,
            None,
        );
        let request = TranscribeRequest {
            audio: Bytes::from_static(b"fake audio"),
            format: AudioFormat::Mp3,
            language: None,
            prompt: None,
        };

        let result = provider.transcribe(request).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not reachable"));
    }

    #[test]
    fn test_vllm_response_parsing() {
        let json = r#"{
            "text": "Hello, how are you?",
            "language": "en",
            "duration": 2.5,
            "words": [
                {"word": "Hello", "start": 0.0, "end": 0.5},
                {"word": "how", "start": 0.6, "end": 0.8}
            ]
        }"#;

        let response: VllmResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.text, "Hello, how are you?");
        assert_eq!(response.language, Some("en".into()));
        assert_eq!(response.duration, Some(2.5));
        assert_eq!(response.words.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_vllm_response_minimal() {
        let json = r#"{"text": "Hello"}"#;
        let response: VllmResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.text, "Hello");
        assert!(response.language.is_none());
        assert!(response.words.is_none());
    }
}
