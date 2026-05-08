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
    anyhow::{Result, anyhow},
    async_trait::async_trait,
    reqwest::Client,
};

use super::{SttProvider, TranscribeRequest, Transcript, openai_compat};

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
        if !openai_compat::check_server_health(&self.client, &self.endpoint).await {
            return Err(anyhow!(
                "vLLM server not reachable at {}. Start it with: vllm serve mistralai/Voxtral-Mini-3B-2507 --tokenizer_mode mistral --config_format mistral --load_format mistral",
                self.endpoint
            ));
        }

        let language = request.language.or_else(|| self.language.clone());

        openai_compat::transcribe_openai_compat(
            &self.client,
            &self.endpoint,
            &request.audio,
            request.format,
            self.model.as_deref(),
            language.as_deref(),
            "vLLM",
        )
        .await
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, crate::tts::AudioFormat, bytes::Bytes};

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
}
