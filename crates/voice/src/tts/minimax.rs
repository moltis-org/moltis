//! MiniMax speech synthesis provider implementation.

use {
    anyhow::{Context, Result, anyhow},
    async_trait::async_trait,
    base64::{Engine as _, engine::general_purpose::STANDARD},
    reqwest::Client,
    secrecy::{ExposeSecret, Secret},
    serde::{Deserialize, Serialize},
};

use super::{AudioFormat, AudioOutput, SynthesizeRequest, TtsProvider, Voice};

const API_BASE: &str = "https://api.minimax.io/v1/t2a_v2";
const DEFAULT_MODEL: &str = "speech-2.8-hd";

/// MiniMax speech synthesis provider.
#[derive(Clone)]
pub struct MiniMaxTts {
    client: Client,
    api_key: Option<Secret<String>>,
    base_url: String,
    default_voice: Option<String>,
    default_model: String,
}

impl std::fmt::Debug for MiniMaxTts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MiniMaxTts")
            .field("api_key", &"[REDACTED]")
            .field("base_url", &self.base_url)
            .field("default_voice", &self.default_voice)
            .field("default_model", &self.default_model)
            .finish()
    }
}

impl Default for MiniMaxTts {
    fn default() -> Self {
        Self::new(None)
    }
}

impl MiniMaxTts {
    #[must_use]
    pub fn new(api_key: Option<Secret<String>>) -> Self {
        Self::with_defaults(api_key, None, None, None)
    }

    #[must_use]
    pub fn with_defaults(
        api_key: Option<Secret<String>>,
        base_url: Option<String>,
        voice: Option<String>,
        model: Option<String>,
    ) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: base_url
                .unwrap_or_else(|| API_BASE.into())
                .trim_end_matches('/')
                .to_string(),
            default_voice: voice,
            default_model: model.unwrap_or_else(|| DEFAULT_MODEL.into()),
        }
    }

    fn format_name(format: AudioFormat) -> &'static str {
        match format {
            AudioFormat::Pcm => "pcm",
            AudioFormat::Wav => "wav",
            AudioFormat::Aac => "aac",
            AudioFormat::Opus | AudioFormat::Webm | AudioFormat::Mp3 => "mp3",
        }
    }
}

#[async_trait]
impl TtsProvider for MiniMaxTts {
    fn id(&self) -> &'static str {
        "minimax"
    }

    fn name(&self) -> &'static str {
        "MiniMax"
    }

    fn is_configured(&self) -> bool {
        self.api_key.is_some()
    }

    async fn voices(&self) -> Result<Vec<Voice>> {
        Ok(Vec::new())
    }

    async fn synthesize(&self, request: SynthesizeRequest) -> Result<AudioOutput> {
        let api_key = self
            .api_key
            .as_ref()
            .ok_or_else(|| anyhow!("MiniMax API key not configured"))?;
        let model = request.model.as_deref().unwrap_or(&self.default_model);
        let voice_id = request
            .voice_id
            .as_deref()
            .or(self.default_voice.as_deref());
        let body = TtsRequest {
            model,
            text: &request.text,
            voice_setting: voice_id.map(|voice_id| VoiceSetting {
                voice_id,
                speed: request.speed,
            }),
            audio_setting: Some(AudioSetting {
                format: Self::format_name(request.output_format),
            }),
        };

        let response = self
            .client
            .post(&self.base_url)
            .bearer_auth(api_key.expose_secret())
            .json(&body)
            .send()
            .await
            .context("failed to send MiniMax TTS request")?;
        let status = response.status();
        let response: TtsResponse = response
            .json()
            .await
            .context("failed to parse MiniMax TTS response")?;
        if !status.is_success() {
            return Err(anyhow!(
                "MiniMax TTS request failed with HTTP status {status}"
            ));
        }
        if response.base_resp.status_code != 0 {
            return Err(anyhow!(
                "MiniMax TTS request failed: {}",
                response.base_resp.status_msg
            ));
        }
        let audio = STANDARD
            .decode(response.data.audio)
            .context("MiniMax TTS response contained invalid base64 audio")?;
        if audio.is_empty() {
            return Err(anyhow!("MiniMax TTS response contained no audio"));
        }
        Ok(AudioOutput {
            data: audio.into(),
            format: request.output_format,
            duration_ms: None,
        })
    }
}

#[derive(Debug, Serialize)]
struct TtsRequest<'a> {
    model: &'a str,
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    voice_setting: Option<VoiceSetting<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    audio_setting: Option<AudioSetting>,
}

#[derive(Debug, Serialize)]
struct VoiceSetting<'a> {
    voice_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed: Option<f32>,
}

#[derive(Debug, Serialize)]
struct AudioSetting {
    format: &'static str,
}

#[derive(Debug, Deserialize)]
struct TtsResponse {
    data: TtsData,
    base_resp: BaseResponse,
}

#[derive(Debug, Deserialize)]
struct TtsData {
    audio: String,
    #[allow(dead_code)]
    status: i32,
}

#[derive(Debug, Deserialize)]
struct BaseResponse {
    status_code: i32,
    status_msg: String,
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{body_json, method, path},
        },
    };

    #[test]
    fn metadata_and_defaults() {
        let provider = MiniMaxTts::new(None);
        assert_eq!(provider.id(), "minimax");
        assert_eq!(provider.name(), "MiniMax");
        assert!(!provider.is_configured());
        assert_eq!(provider.default_model, DEFAULT_MODEL);
        assert_eq!(provider.base_url, API_BASE);
    }

    #[test]
    fn debug_redacts_api_key() {
        let provider = MiniMaxTts::new(Some(Secret::new("secret-value".into())));
        let debug_output = format!("{provider:?}");
        assert!(debug_output.contains("[REDACTED]"));
        assert!(!debug_output.contains("secret-value"));
    }

    #[tokio::test]
    async fn synthesizes_and_decodes_audio_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/t2a_v2"))
            .and(body_json(serde_json::json!({
                "model": "speech-2.8-hd",
                "text": "hello",
                "audio_setting": {"format": "mp3"}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {"audio": STANDARD.encode("audio-bytes"), "status": 2},
                "base_resp": {"status_code": 0, "status_msg": "success"}
            })))
            .mount(&server)
            .await;

        let provider = MiniMaxTts::with_defaults(
            Some(Secret::new("test-key".into())),
            Some(format!("{}/v1/t2a_v2", server.uri())),
            None,
            None,
        );
        let output = provider
            .synthesize(SynthesizeRequest {
                text: "hello".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(output.data.as_ref(), b"audio-bytes");
        assert_eq!(output.format, AudioFormat::Mp3);
    }
}
