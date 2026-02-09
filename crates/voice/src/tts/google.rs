//! Google Cloud Text-to-Speech provider.

use {
    crate::{
        config::GoogleTtsConfig,
        tts::{AudioFormat, AudioOutput, SynthesizeRequest, TtsProvider, Voice, contains_ssml},
    },
    anyhow::{Result, anyhow},
    async_trait::async_trait,
    bytes::Bytes,
    reqwest::Client,
    secrecy::{ExposeSecret, Secret},
    serde::{Deserialize, Serialize},
};

/// Google Cloud Text-to-Speech provider.
pub struct GoogleTts {
    api_key: Option<Secret<String>>,
    voice: Option<String>,
    language_code: String,
    speaking_rate: f32,
    pitch: f32,
    client: Client,
}

impl GoogleTts {
    /// Create a new Google Cloud TTS provider from config.
    #[must_use]
    pub fn new(config: &GoogleTtsConfig) -> Self {
        let api_key = config
            .api_key
            .clone()
            .or_else(|| std::env::var("GOOGLE_API_KEY").ok().map(Secret::new));

        Self {
            api_key,
            voice: config.voice.clone(),
            language_code: config
                .language_code
                .clone()
                .unwrap_or_else(|| "en-US".into()),
            speaking_rate: config.speaking_rate.unwrap_or(1.0),
            pitch: config.pitch.unwrap_or(0.0),
            client: Client::new(),
        }
    }
}

#[async_trait]
impl TtsProvider for GoogleTts {
    fn id(&self) -> &'static str {
        "google"
    }

    fn name(&self) -> &'static str {
        "Google Cloud TTS"
    }

    fn is_configured(&self) -> bool {
        self.api_key.is_some()
    }

    fn supports_ssml(&self) -> bool {
        true
    }

    async fn voices(&self) -> Result<Vec<Voice>> {
        let api_key = self
            .api_key
            .as_ref()
            .ok_or_else(|| anyhow!("Google Cloud TTS API key not configured"))?;

        let url = format!(
            "https://texttospeech.googleapis.com/v1/voices?key={}",
            api_key.expose_secret()
        );

        let resp = self.client.get(&url).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Google Cloud TTS API error {}: {}", status, body));
        }

        let voices_resp: VoicesResponse = resp.json().await?;

        // Filter to voices matching the configured language
        let voices = voices_resp
            .voices
            .unwrap_or_default()
            .into_iter()
            .filter(|v| {
                v.language_codes
                    .iter()
                    .any(|lc| lc.starts_with(&self.language_code[..2]))
            })
            .map(|v| Voice {
                id: v.name.clone(),
                name: v.name,
                description: Some(format!(
                    "{} - {}",
                    v.language_codes.join(", "),
                    v.ssml_gender.unwrap_or_default()
                )),
                preview_url: None,
            })
            .collect();

        Ok(voices)
    }

    async fn synthesize(&self, request: SynthesizeRequest) -> Result<AudioOutput> {
        let api_key = self
            .api_key
            .as_ref()
            .ok_or_else(|| anyhow!("Google Cloud TTS API key not configured"))?;

        let voice_name = request
            .voice_id
            .or_else(|| self.voice.clone())
            .unwrap_or_else(|| format!("{}-Neural2-A", self.language_code));

        // Map output format to Google's encoding
        let audio_encoding = match request.output_format {
            AudioFormat::Mp3 => "MP3",
            AudioFormat::Opus => "OGG_OPUS",
            AudioFormat::Aac => "MP3", // AAC not supported, fallback to MP3
            AudioFormat::Pcm => "LINEAR16",
        };

        let input = if contains_ssml(&request.text) {
            // Wrap in <speak> if not already wrapped, use native SSML field
            let ssml = if request.text.trim_start().starts_with("<speak") {
                request.text.clone()
            } else {
                format!("<speak>{}</speak>", request.text)
            };
            SynthesisInput {
                text: None,
                ssml: Some(ssml),
            }
        } else {
            SynthesisInput {
                text: Some(request.text.clone()),
                ssml: None,
            }
        };

        let req_body = SynthesizeRequestBody {
            input,
            voice: VoiceSelectionParams {
                language_code: self.language_code.clone(),
                name: voice_name,
                ssml_gender: None,
            },
            audio_config: AudioConfig {
                audio_encoding: audio_encoding.into(),
                speaking_rate: request.speed.unwrap_or(self.speaking_rate),
                pitch: self.pitch,
                sample_rate_hertz: None,
            },
        };

        let url = format!(
            "https://texttospeech.googleapis.com/v1/text:synthesize?key={}",
            api_key.expose_secret()
        );

        let resp = self.client.post(&url).json(&req_body).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Google Cloud TTS API error {}: {}", status, body));
        }

        let synth_resp: SynthesizeResponse = resp.json().await?;

        // Decode base64 audio content
        use base64::Engine;
        let audio_data =
            base64::engine::general_purpose::STANDARD.decode(&synth_resp.audio_content)?;

        Ok(AudioOutput {
            data: Bytes::from(audio_data),
            format: request.output_format,
            duration_ms: None,
        })
    }
}

// ── API request/response types ─────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SynthesizeRequestBody {
    input: SynthesisInput,
    voice: VoiceSelectionParams,
    audio_config: AudioConfig,
}

#[derive(Serialize)]
struct SynthesisInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ssml: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VoiceSelectionParams {
    language_code: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    ssml_gender: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AudioConfig {
    audio_encoding: String,
    speaking_rate: f32,
    pitch: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    sample_rate_hertz: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SynthesizeResponse {
    audio_content: String,
}

#[derive(Deserialize)]
struct VoicesResponse {
    voices: Option<Vec<GoogleVoice>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleVoice {
    language_codes: Vec<String>,
    name: String,
    ssml_gender: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_google_tts_not_configured_without_key() {
        let config = GoogleTtsConfig::default();
        let tts = GoogleTts::new(&config);
        // Without env var set, should not be configured
        if std::env::var("GOOGLE_API_KEY").is_err() {
            assert!(!tts.is_configured());
        }
    }

    #[test]
    fn test_google_tts_id_and_name() {
        let config = GoogleTtsConfig::default();
        let tts = GoogleTts::new(&config);
        assert_eq!(tts.id(), "google");
        assert_eq!(tts.name(), "Google Cloud TTS");
        assert!(tts.supports_ssml());
    }
}
