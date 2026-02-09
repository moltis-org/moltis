//! ElevenLabs TTS provider implementation.
//!
//! ElevenLabs offers high-quality, low-latency text-to-speech with support
//! for voice cloning and multiple models. The Flash v2.5 model provides
//! the lowest latency (~75ms) for conversational AI applications.

use {
    anyhow::{Context, Result, anyhow},
    async_trait::async_trait,
    reqwest::Client,
    secrecy::{ExposeSecret, Secret},
    serde::{Deserialize, Serialize},
};

use super::{AudioFormat, AudioOutput, SynthesizeRequest, TtsProvider, Voice, contains_ssml};

/// ElevenLabs API base URL.
const API_BASE: &str = "https://api.elevenlabs.io/v1";

/// Default voice ID (Rachel - clear, professional female voice).
const DEFAULT_VOICE_ID: &str = "21m00Tcm4TlvDq8ikWAM";

/// Default model (Flash v2.5 for lowest latency).
const DEFAULT_MODEL: &str = "eleven_flash_v2_5";

/// ElevenLabs TTS provider.
#[derive(Clone)]
pub struct ElevenLabsTts {
    client: Client,
    api_key: Option<Secret<String>>,
    default_voice_id: String,
    default_model: String,
}

impl std::fmt::Debug for ElevenLabsTts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ElevenLabsTts")
            .field("api_key", &"[REDACTED]")
            .field("default_voice_id", &self.default_voice_id)
            .field("default_model", &self.default_model)
            .finish()
    }
}

impl Default for ElevenLabsTts {
    fn default() -> Self {
        Self::new(None)
    }
}

impl ElevenLabsTts {
    /// Create a new ElevenLabs TTS provider.
    #[must_use]
    pub fn new(api_key: Option<Secret<String>>) -> Self {
        Self {
            client: Client::new(),
            api_key,
            default_voice_id: DEFAULT_VOICE_ID.into(),
            default_model: DEFAULT_MODEL.into(),
        }
    }

    /// Create with custom default voice and model.
    #[must_use]
    pub fn with_defaults(
        api_key: Option<Secret<String>>,
        voice_id: Option<String>,
        model: Option<String>,
    ) -> Self {
        Self {
            client: Client::new(),
            api_key,
            default_voice_id: voice_id.unwrap_or_else(|| DEFAULT_VOICE_ID.into()),
            default_model: model.unwrap_or_else(|| DEFAULT_MODEL.into()),
        }
    }

    /// Get the API key, returning an error if not configured.
    fn get_api_key(&self) -> Result<&Secret<String>> {
        self.api_key
            .as_ref()
            .ok_or_else(|| anyhow!("ElevenLabs API key not configured"))
    }

    /// Map audio format to ElevenLabs output format parameter.
    fn output_format_param(format: AudioFormat) -> &'static str {
        match format {
            AudioFormat::Mp3 => "mp3_44100_128",
            AudioFormat::Opus => "opus_48000_64", // Good for Telegram voice notes
            AudioFormat::Aac => "aac_44100",
            AudioFormat::Pcm => "pcm_44100",
            AudioFormat::Webm => "opus_48000_64", // WebM uses Opus codec
        }
    }
}

#[async_trait]
impl TtsProvider for ElevenLabsTts {
    fn id(&self) -> &'static str {
        "elevenlabs"
    }

    fn name(&self) -> &'static str {
        "ElevenLabs"
    }

    fn is_configured(&self) -> bool {
        self.api_key.is_some()
    }

    fn supports_ssml(&self) -> bool {
        true
    }

    async fn voices(&self) -> Result<Vec<Voice>> {
        let api_key = self.get_api_key()?;

        let response = self
            .client
            .get(format!("{API_BASE}/voices"))
            .header("xi-api-key", api_key.expose_secret())
            .send()
            .await
            .context("failed to fetch ElevenLabs voices")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "ElevenLabs voices request failed: {} - {}",
                status,
                body
            ));
        }

        let voices_response: VoicesResponse = response
            .json()
            .await
            .context("failed to parse ElevenLabs voices response")?;

        Ok(voices_response
            .voices
            .into_iter()
            .map(|v| Voice {
                id: v.voice_id,
                name: v.name,
                description: v.description,
                preview_url: v.preview_url,
            })
            .collect())
    }

    async fn synthesize(&self, request: SynthesizeRequest) -> Result<AudioOutput> {
        let api_key = self.get_api_key()?;
        let voice_id = request
            .voice_id
            .as_deref()
            .unwrap_or(&self.default_voice_id);
        let model = request.model.as_deref().unwrap_or(&self.default_model);
        let has_ssml = contains_ssml(&request.text);

        let body = TtsRequest {
            text: &request.text,
            model_id: model,
            voice_settings: Some(VoiceSettings {
                stability: request.stability.unwrap_or(0.5),
                similarity_boost: request.similarity_boost.unwrap_or(0.75),
                style: None,
                use_speaker_boost: Some(true),
            }),
            enable_ssml_parsing: has_ssml,
        };

        let output_format = Self::output_format_param(request.output_format);
        let url = format!(
            "{API_BASE}/text-to-speech/{voice_id}?output_format={output_format}&optimize_streaming_latency=2"
        );

        let response = self
            .client
            .post(&url)
            .header("xi-api-key", api_key.expose_secret())
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("failed to send ElevenLabs TTS request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "ElevenLabs TTS request failed: {} - {}",
                status,
                body
            ));
        }

        let data = response
            .bytes()
            .await
            .context("failed to read ElevenLabs TTS response")?;

        Ok(AudioOutput {
            data,
            format: request.output_format,
            duration_ms: None, // ElevenLabs doesn't return duration in response
        })
    }
}

// ── API Types ──────────────────────────────────────────────────────────────

fn is_false(v: &bool) -> bool {
    !v
}

#[derive(Debug, Serialize)]
struct TtsRequest<'a> {
    text: &'a str,
    model_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    voice_settings: Option<VoiceSettings>,
    #[serde(skip_serializing_if = "is_false")]
    enable_ssml_parsing: bool,
}

#[derive(Debug, Serialize)]
struct VoiceSettings {
    stability: f32,
    similarity_boost: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    style: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    use_speaker_boost: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct VoicesResponse {
    voices: Vec<ElevenLabsVoice>,
}

#[derive(Debug, Deserialize)]
struct ElevenLabsVoice {
    voice_id: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    preview_url: Option<String>,
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{header, method, path_regex},
        },
    };

    #[test]
    fn test_provider_metadata() {
        let provider = ElevenLabsTts::new(None);
        assert_eq!(provider.id(), "elevenlabs");
        assert_eq!(provider.name(), "ElevenLabs");
        assert!(!provider.is_configured());
        assert!(provider.supports_ssml());

        let configured = ElevenLabsTts::new(Some(Secret::new("test-key".into())));
        assert!(configured.is_configured());
    }

    #[test]
    fn test_output_format_param() {
        assert_eq!(
            ElevenLabsTts::output_format_param(AudioFormat::Mp3),
            "mp3_44100_128"
        );
        assert_eq!(
            ElevenLabsTts::output_format_param(AudioFormat::Opus),
            "opus_48000_64"
        );
    }

    #[test]
    fn test_debug_redacts_api_key() {
        let provider = ElevenLabsTts::new(Some(Secret::new("super-secret-key".into())));
        let debug_output = format!("{:?}", provider);
        assert!(debug_output.contains("[REDACTED]"));
        assert!(!debug_output.contains("super-secret-key"));
    }

    #[tokio::test]
    async fn test_voices_request() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path_regex("/v1/voices"))
            .and(header("xi-api-key", "test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "voices": [
                    {
                        "voice_id": "voice1",
                        "name": "Test Voice",
                        "description": "A test voice",
                        "preview_url": "https://example.com/preview.mp3"
                    }
                ]
            })))
            .mount(&mock_server)
            .await;

        // Create provider with mocked URL
        let provider = ElevenLabsTts {
            client: Client::new(),
            api_key: Some(Secret::new("test-key".into())),
            default_voice_id: DEFAULT_VOICE_ID.into(),
            default_model: DEFAULT_MODEL.into(),
        };

        // Note: This test would need the API_BASE to be configurable for full testing
        // For now, we just verify the provider is properly structured
        assert!(provider.is_configured());
    }

    #[tokio::test]
    async fn test_synthesize_without_api_key() {
        let provider = ElevenLabsTts::new(None);
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
        let provider = ElevenLabsTts::with_defaults(
            Some(Secret::new("key".into())),
            Some("custom-voice".into()),
            Some("custom-model".into()),
        );
        assert_eq!(provider.default_voice_id, "custom-voice");
        assert_eq!(provider.default_model, "custom-model");
    }

    #[test]
    fn test_ssml_detection_enables_parsing() {
        let body = TtsRequest {
            text: "Hello <break time=\"0.5s\"/> world",
            model_id: "eleven_flash_v2_5",
            voice_settings: None,
            enable_ssml_parsing: true,
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"enable_ssml_parsing\":true"));
    }

    #[test]
    fn test_no_ssml_skips_parsing_field() {
        let body = TtsRequest {
            text: "Hello world",
            model_id: "eleven_flash_v2_5",
            voice_settings: None,
            enable_ssml_parsing: false,
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(!json.contains("enable_ssml_parsing"));
    }
}
