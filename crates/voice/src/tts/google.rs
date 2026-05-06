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
///
/// Supports two API modes:
/// - **Cloud TTS v1** (default): `texttospeech.googleapis.com/v1` — standard
///   Neural2/WaveNet voices, SSML support, no instructions.
/// - **Gemini TTS**: `generativelanguage.googleapis.com` — when a `gemini-*`
///   model is configured or requested. Supports free-form voice direction via
///   the `instructions` field on [`SynthesizeRequest`].
pub struct GoogleTts {
    api_key: Option<Secret<String>>,
    voice: Option<String>,
    /// Default model. When set to a `gemini-*` value, uses the Gemini TTS API.
    model: Option<String>,
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
            model: None,
            language_code: config
                .language_code
                .clone()
                .unwrap_or_else(|| "en-US".into()),
            speaking_rate: config.speaking_rate.unwrap_or(1.0),
            pitch: config.pitch.unwrap_or(0.0),
            client: Client::new(),
        }
    }

    /// Create with an explicit model override.
    #[must_use]
    pub fn with_model(mut self, model: Option<String>) -> Self {
        self.model = model;
        self
    }

    /// Whether to use the Gemini TTS API path for a given model.
    fn is_gemini_model(model: Option<&str>) -> bool {
        model.is_some_and(|m| m.starts_with("gemini-"))
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
        let language_prefix = self
            .language_code
            .split('-')
            .next()
            .unwrap_or(self.language_code.as_str());

        // Filter to voices matching the configured language
        let voices = voices_resp
            .voices
            .unwrap_or_default()
            .into_iter()
            .filter(|v| {
                v.language_codes
                    .iter()
                    .filter_map(|lc| lc.split('-').next())
                    .any(|prefix| prefix.eq_ignore_ascii_case(language_prefix))
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

        let model_str = request
            .model
            .as_deref()
            .or(self.model.as_deref())
            .unwrap_or_default()
            .to_string();

        if Self::is_gemini_model(Some(&model_str)) {
            return self.synthesize_gemini(api_key, &model_str, request).await;
        }

        self.synthesize_cloud_v1(api_key, request).await
    }
}

impl GoogleTts {
    /// Standard Google Cloud TTS v1 synthesis.
    async fn synthesize_cloud_v1(
        &self,
        api_key: &Secret<String>,
        request: SynthesizeRequest,
    ) -> Result<AudioOutput> {
        let voice_name = request
            .voice_id
            .or_else(|| self.voice.clone())
            .unwrap_or_else(|| format!("{}-Neural2-A", self.language_code));

        let audio_encoding = match request.output_format {
            AudioFormat::Mp3 => "MP3",
            AudioFormat::Opus | AudioFormat::Webm => "OGG_OPUS",
            AudioFormat::Aac => "MP3",
            AudioFormat::Pcm => "LINEAR16",
        };

        let input = if contains_ssml(&request.text) {
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

        use base64::Engine;
        let audio_data =
            base64::engine::general_purpose::STANDARD.decode(&synth_resp.audio_content)?;

        Ok(AudioOutput {
            data: Bytes::from(audio_data),
            format: request.output_format,
            duration_ms: None,
        })
    }

    /// Gemini TTS synthesis via the Generative Language API.
    ///
    /// Uses `generateContent` with `response_modalities: ["AUDIO"]` and an
    /// optional `speech_config.voice_config.prebuilt_voice_config.voice_name`.
    /// Voice persona instructions are passed as a system instruction.
    async fn synthesize_gemini(
        &self,
        api_key: &Secret<String>,
        model: &str,
        request: SynthesizeRequest,
    ) -> Result<AudioOutput> {
        let voice_name = request.voice_id.or_else(|| self.voice.clone());

        // Build the speech config with optional voice name.
        let speech_config = voice_name.map(|v| {
            serde_json::json!({
                "voiceConfig": {
                    "prebuiltVoiceConfig": {
                        "voiceName": v,
                    }
                }
            })
        });

        // Build the generation config (Gemini API uses camelCase).
        let mut generation_config = serde_json::json!({
            "responseModalities": ["AUDIO"],
            "responseMimeType": "audio/mp3",
        });
        if let Some(sc) = speech_config {
            generation_config["speechConfig"] = sc;
        }

        // Build the request body.
        let mut body = serde_json::json!({
            "contents": [{
                "parts": [{ "text": request.text }]
            }],
            "generationConfig": generation_config,
        });

        // Inject persona instructions as a system instruction.
        if let Some(ref instructions) = request.instructions {
            body["systemInstruction"] = serde_json::json!({
                "parts": [{ "text": instructions }]
            });
        }

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            model,
            api_key.expose_secret()
        );

        let resp = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Gemini TTS API error {}: {}", status, err_body));
        }

        let resp_body: serde_json::Value = resp.json().await?;

        // Extract inline audio data from the Gemini response.
        let audio_b64 = resp_body
            .pointer("/candidates/0/content/parts/0/inlineData/data")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Gemini TTS response missing audio data"))?;

        use base64::Engine;
        let audio_data = base64::engine::general_purpose::STANDARD.decode(audio_b64)?;

        Ok(AudioOutput {
            data: Bytes::from(audio_data),
            format: AudioFormat::Mp3,
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

    #[test]
    fn test_is_gemini_model() {
        assert!(GoogleTts::is_gemini_model(Some(
            "gemini-2.5-flash-preview-tts"
        )));
        assert!(GoogleTts::is_gemini_model(Some("gemini-2.0-flash-lite")));
        assert!(!GoogleTts::is_gemini_model(Some("en-US-Neural2-A")));
        assert!(!GoogleTts::is_gemini_model(None));
    }

    #[test]
    fn test_with_model() {
        let config = GoogleTtsConfig {
            api_key: Some(Secret::new("test".to_string())),
            ..Default::default()
        };
        let tts = GoogleTts::new(&config).with_model(Some("gemini-2.5-flash-preview-tts".into()));
        assert!(GoogleTts::is_gemini_model(tts.model.as_deref()));
    }

    #[test]
    fn language_prefix_extraction_handles_short_tags() {
        let config = GoogleTtsConfig {
            api_key: Some(Secret::new("test".to_string())),
            language_code: Some("e".to_string()),
            ..Default::default()
        };
        let tts = GoogleTts::new(&config);
        assert_eq!(tts.language_code, "e");
    }
}
