//! Coqui TTS (local) provider.
//!
//! Coqui TTS is an open-source deep learning toolkit for Text-to-Speech.
//! Run the server: tts-server --model_name tts_models/en/ljspeech/tacotron2-DDC
//! Or use Docker: docker run -p 5002:5002 ghcr.io/coqui-ai/tts

use {
    crate::{
        config::CoquiTtsConfig,
        tts::{AudioFormat, AudioOutput, SynthesizeRequest, TtsProvider, Voice},
    },
    anyhow::{Result, anyhow},
    async_trait::async_trait,
    bytes::Bytes,
    reqwest::Client,
    serde::Deserialize,
};

const DEFAULT_ENDPOINT: &str = "http://localhost:5002";

/// Coqui TTS (local server) provider.
pub struct CoquiTts {
    endpoint: String,
    model: Option<String>,
    speaker: Option<String>,
    language: Option<String>,
    client: Client,
}

impl CoquiTts {
    /// Create a new Coqui TTS provider from config.
    #[must_use]
    pub fn new(config: &CoquiTtsConfig) -> Self {
        Self {
            endpoint: config.endpoint.clone(),
            model: config.model.clone(),
            speaker: config.speaker.clone(),
            language: config.language.clone(),
            client: Client::new(),
        }
    }
}

#[async_trait]
impl TtsProvider for CoquiTts {
    fn id(&self) -> &'static str {
        "coqui"
    }

    fn name(&self) -> &'static str {
        "Coqui TTS"
    }

    fn is_configured(&self) -> bool {
        // Configured if endpoint is non-default or model is specified
        self.endpoint != DEFAULT_ENDPOINT || self.model.is_some()
    }

    async fn voices(&self) -> Result<Vec<Voice>> {
        // Try to get available speakers from the server
        let url = format!("{}/api/speakers", self.endpoint);

        let resp = self
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                // Server returns a list of speaker names/IDs
                if let Ok(speakers) = r.json::<SpeakersResponse>().await {
                    let voices = speakers
                        .0
                        .into_iter()
                        .map(|s| Voice {
                            id: s.clone(),
                            name: s,
                            description: None,
                            preview_url: None,
                        })
                        .collect();
                    return Ok(voices);
                }
            },
            _ => {},
        }

        // If server doesn't support speakers endpoint, return default voice
        Ok(vec![Voice {
            id: "default".into(),
            name: "Default".into(),
            description: Some("Coqui TTS default voice".into()),
            preview_url: None,
        }])
    }

    async fn synthesize(&self, request: SynthesizeRequest) -> Result<AudioOutput> {
        // Build query parameters
        let mut url = format!(
            "{}/api/tts?text={}",
            self.endpoint,
            urlencoding::encode(&request.text)
        );

        // Add speaker if specified
        if let Some(speaker) = request.voice_id.as_ref().or(self.speaker.as_ref()) {
            url.push_str(&format!("&speaker_id={}", urlencoding::encode(speaker)));
        }

        // Add language for multilingual models
        if let Some(language) = &self.language {
            url.push_str(&format!("&language_id={}", urlencoding::encode(language)));
        }

        let resp = self
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(60)) // TTS can be slow
            .send()
            .await
            .map_err(|e| {
                anyhow!(
                    "Failed to connect to Coqui TTS server at {}: {}. Start with: tts-server",
                    self.endpoint,
                    e
                )
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Coqui TTS error {}: {}", status, body));
        }

        // Coqui TTS returns WAV audio by default
        let audio_data = resp.bytes().await?;

        // The response is WAV format
        Ok(AudioOutput {
            data: Bytes::from(audio_data.to_vec()),
            format: AudioFormat::Pcm, // WAV is essentially PCM with headers
            duration_ms: None,
        })
    }
}

// Response type for speakers endpoint
#[derive(Deserialize)]
struct SpeakersResponse(Vec<String>);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coqui_not_configured_with_defaults() {
        let config = CoquiTtsConfig::default();
        let tts = CoquiTts::new(&config);
        // Default endpoint without model is not considered configured
        assert!(!tts.is_configured());
    }

    #[test]
    fn test_coqui_configured_with_model() {
        let config = CoquiTtsConfig {
            model: Some("tts_models/en/ljspeech/tacotron2-DDC".into()),
            ..Default::default()
        };
        let tts = CoquiTts::new(&config);
        assert!(tts.is_configured());
    }

    #[test]
    fn test_coqui_configured_with_custom_endpoint() {
        let config = CoquiTtsConfig {
            endpoint: "http://192.168.1.100:5002".into(),
            ..Default::default()
        };
        let tts = CoquiTts::new(&config);
        assert!(tts.is_configured());
    }

    #[test]
    fn test_coqui_id_and_name() {
        let config = CoquiTtsConfig::default();
        let tts = CoquiTts::new(&config);
        assert_eq!(tts.id(), "coqui");
        assert_eq!(tts.name(), "Coqui TTS");
        assert!(!tts.supports_ssml());
    }
}
