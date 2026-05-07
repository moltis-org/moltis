//! Local Whisper STT provider via an OpenAI-compatible server.
//!
//! Works with any server that implements the `/v1/audio/transcriptions`
//! endpoint: faster-whisper-server, whisper.cpp server, LocalAI, etc.
//!
//! Example with faster-whisper-server:
//! ```bash
//! pip install faster-whisper-server
//! faster-whisper-server --model Systran/faster-whisper-large-v3 --port 8080
//! ```

use {
    anyhow::{Result, anyhow},
    async_trait::async_trait,
    reqwest::Client,
};

use super::{SttProvider, TranscribeRequest, Transcript, openai_compat};

/// Default local server endpoint.
const DEFAULT_ENDPOINT: &str = "http://localhost:8080";

/// Local Whisper STT provider via an OpenAI-compatible server.
#[derive(Clone, Debug)]
pub struct WhisperLocalStt {
    client: Client,
    endpoint: String,
    model: Option<String>,
    language: Option<String>,
}

impl Default for WhisperLocalStt {
    fn default() -> Self {
        Self::new()
    }
}

impl WhisperLocalStt {
    /// Create a new local Whisper STT provider with default settings.
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
impl SttProvider for WhisperLocalStt {
    fn id(&self) -> &'static str {
        "whisper-local"
    }

    fn name(&self) -> &'static str {
        "Whisper (Local)"
    }

    fn is_configured(&self) -> bool {
        // Always report as configured — the actual server reachability check
        // happens at transcription time. The `enabled` field in the config
        // gates whether this provider is offered to users.
        true
    }

    async fn transcribe(&self, request: TranscribeRequest) -> Result<Transcript> {
        if !openai_compat::check_server_health(&self.client, &self.endpoint).await {
            return Err(anyhow!(
                "Whisper server not reachable at {}. Start an OpenAI-compatible Whisper server (e.g. faster-whisper-server, whisper.cpp server, or LocalAI).",
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
            "whisper-local",
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
        let provider = WhisperLocalStt::new();
        assert_eq!(provider.id(), "whisper-local");
        assert_eq!(provider.name(), "Whisper (Local)");
        // Always configured — runtime health check gates actual use
        assert!(provider.is_configured());
    }

    #[test]
    fn test_with_options() {
        let provider = WhisperLocalStt::with_options(
            Some("http://localhost:9000".into()),
            Some("large-v3".into()),
            Some("en".into()),
        );
        assert_eq!(provider.endpoint, "http://localhost:9000");
        assert_eq!(provider.model, Some("large-v3".into()));
        assert_eq!(provider.language, Some("en".into()));
    }

    #[test]
    fn test_default_endpoint() {
        let provider = WhisperLocalStt::new();
        assert_eq!(provider.endpoint, "http://localhost:8080");
    }

    #[tokio::test]
    async fn test_transcribe_server_not_running() {
        let provider =
            WhisperLocalStt::with_options(Some("http://localhost:59998".into()), None, None);
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
