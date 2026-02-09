//! OpenAI TTS provider implementation.
//!
//! OpenAI offers text-to-speech with multiple voices and two quality tiers:
//! - tts-1: Optimized for real-time, lower latency
//! - tts-1-hd: Higher quality, slightly higher latency

use {
    anyhow::{Context, Result, anyhow},
    async_trait::async_trait,
    reqwest::Client,
    secrecy::{ExposeSecret, Secret},
    serde::Serialize,
};

use super::{AudioFormat, AudioOutput, SynthesizeRequest, TtsProvider, Voice};

/// OpenAI API base URL.
const API_BASE: &str = "https://api.openai.com/v1";

/// Default voice.
const DEFAULT_VOICE: &str = "alloy";

/// Default model (real-time optimized).
const DEFAULT_MODEL: &str = "tts-1";

/// Available OpenAI TTS voices.
const VOICES: &[(&str, &str)] = &[
    ("alloy", "Alloy - Neutral, balanced"),
    ("echo", "Echo - Warm, conversational"),
    ("fable", "Fable - Expressive, storytelling"),
    ("onyx", "Onyx - Deep, authoritative"),
    ("nova", "Nova - Friendly, upbeat"),
    ("shimmer", "Shimmer - Soft, gentle"),
];

/// OpenAI TTS provider.
#[derive(Clone)]
pub struct OpenAiTts {
    client: Client,
    api_key: Option<Secret<String>>,
    default_voice: String,
    default_model: String,
}

impl std::fmt::Debug for OpenAiTts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiTts")
            .field("api_key", &"[REDACTED]")
            .field("default_voice", &self.default_voice)
            .field("default_model", &self.default_model)
            .finish()
    }
}

impl Default for OpenAiTts {
    fn default() -> Self {
        Self::new(None)
    }
}

impl OpenAiTts {
    /// Create a new OpenAI TTS provider.
    #[must_use]
    pub fn new(api_key: Option<Secret<String>>) -> Self {
        Self {
            client: Client::new(),
            api_key,
            default_voice: DEFAULT_VOICE.into(),
            default_model: DEFAULT_MODEL.into(),
        }
    }

    /// Create with custom default voice and model.
    #[must_use]
    pub fn with_defaults(
        api_key: Option<Secret<String>>,
        voice: Option<String>,
        model: Option<String>,
    ) -> Self {
        Self {
            client: Client::new(),
            api_key,
            default_voice: voice.unwrap_or_else(|| DEFAULT_VOICE.into()),
            default_model: model.unwrap_or_else(|| DEFAULT_MODEL.into()),
        }
    }

    /// Get the API key, returning an error if not configured.
    fn get_api_key(&self) -> Result<&Secret<String>> {
        self.api_key
            .as_ref()
            .ok_or_else(|| anyhow!("OpenAI API key not configured"))
    }

    /// Map audio format to OpenAI response format.
    fn response_format(format: AudioFormat) -> &'static str {
        match format {
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Opus => "opus",
            AudioFormat::Aac => "aac",
            AudioFormat::Pcm => "pcm",
        }
    }
}

#[async_trait]
impl TtsProvider for OpenAiTts {
    fn id(&self) -> &'static str {
        "openai"
    }

    fn name(&self) -> &'static str {
        "OpenAI"
    }

    fn is_configured(&self) -> bool {
        self.api_key.is_some()
    }

    async fn voices(&self) -> Result<Vec<Voice>> {
        // OpenAI has a fixed set of voices, no API call needed
        Ok(VOICES
            .iter()
            .map(|(id, desc)| Voice {
                id: (*id).to_string(),
                name: (*id).to_string(),
                description: Some((*desc).to_string()),
                preview_url: None,
            })
            .collect())
    }

    async fn synthesize(&self, request: SynthesizeRequest) -> Result<AudioOutput> {
        let api_key = self.get_api_key()?;
        let voice = request.voice_id.as_deref().unwrap_or(&self.default_voice);
        let model = request.model.as_deref().unwrap_or(&self.default_model);
        let body = TtsRequest {
            model,
            input: &request.text,
            voice,
            response_format: Some(Self::response_format(request.output_format)),
            speed: request.speed,
        };

        let response = self
            .client
            .post(format!("{API_BASE}/audio/speech"))
            .header(
                "Authorization",
                format!("Bearer {}", api_key.expose_secret()),
            )
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("failed to send OpenAI TTS request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!("OpenAI TTS request failed: {} - {}", status, body));
        }

        let data = response
            .bytes()
            .await
            .context("failed to read OpenAI TTS response")?;

        Ok(AudioOutput {
            data,
            format: request.output_format,
            duration_ms: None,
        })
    }
}

// ── API Types ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct TtsRequest<'a> {
    model: &'a str,
    input: &'a str,
    voice: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed: Option<f32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_metadata() {
        let provider = OpenAiTts::new(None);
        assert_eq!(provider.id(), "openai");
        assert_eq!(provider.name(), "OpenAI");
        assert!(!provider.is_configured());
        assert!(!provider.supports_ssml());

        let configured = OpenAiTts::new(Some(Secret::new("test-key".into())));
        assert!(configured.is_configured());
    }

    #[test]
    fn test_response_format() {
        assert_eq!(OpenAiTts::response_format(AudioFormat::Mp3), "mp3");
        assert_eq!(OpenAiTts::response_format(AudioFormat::Opus), "opus");
        assert_eq!(OpenAiTts::response_format(AudioFormat::Aac), "aac");
    }

    #[test]
    fn test_debug_redacts_api_key() {
        let provider = OpenAiTts::new(Some(Secret::new("super-secret-key".into())));
        let debug_output = format!("{:?}", provider);
        assert!(debug_output.contains("[REDACTED]"));
        assert!(!debug_output.contains("super-secret-key"));
    }

    #[tokio::test]
    async fn test_voices_returns_all_voices() {
        let provider = OpenAiTts::new(None);
        let voices = provider.voices().await.unwrap();

        assert_eq!(voices.len(), VOICES.len());
        assert!(voices.iter().any(|v| v.id == "alloy"));
        assert!(voices.iter().any(|v| v.id == "nova"));
    }

    #[tokio::test]
    async fn test_synthesize_without_api_key() {
        let provider = OpenAiTts::new(None);
        let request = SynthesizeRequest {
            text: "Hello".into(),
            ..Default::default()
        };

        let result = provider.synthesize(request).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not configured"));
    }

    #[test]
    fn test_with_defaults() {
        let provider = OpenAiTts::with_defaults(
            Some(Secret::new("key".into())),
            Some("nova".into()),
            Some("tts-1-hd".into()),
        );
        assert_eq!(provider.default_voice, "nova");
        assert_eq!(provider.default_model, "tts-1-hd");
    }

    #[test]
    fn test_tts_request_serialization() {
        let request = TtsRequest {
            model: "tts-1",
            input: "Hello world",
            voice: "alloy",
            response_format: Some("mp3"),
            speed: Some(1.5),
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"model\":\"tts-1\""));
        assert!(json.contains("\"input\":\"Hello world\""));
        assert!(json.contains("\"voice\":\"alloy\""));
        assert!(json.contains("\"speed\":1.5"));
    }
}
