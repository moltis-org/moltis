//! Voice service implementations for TTS and STT.
//!
//! This module provides concrete implementations of the `TtsService` and
//! `SttService` traits using the moltis-voice crate's providers.

use {
    async_trait::async_trait,
    serde_json::{Value, json},
};

use crate::services::ServiceResult;

#[cfg(feature = "voice")]
use {base64::Engine, secrecy::Secret, tracing::debug};

#[cfg(feature = "voice")]
use moltis_voice::{
    AudioFormat, CoquiTts, DeepgramStt, ElevenLabsStt, ElevenLabsTts, GoogleStt, GoogleTts,
    GroqStt, MistralStt, OpenAiTts, PiperTts, SherpaOnnxStt, SttProvider, SttProviderId,
    SynthesizeRequest, TranscribeRequest, TtsConfig, TtsProvider, TtsProviderId, VoxtralLocalStt,
    WhisperCliStt, WhisperStt, strip_ssml_tags,
};

#[cfg(feature = "voice")]
use crate::services::TtsService;

#[cfg(feature = "voice")]
trait IntoVoiceSttProvider {
    fn into_voice_stt_provider(self) -> moltis_config::VoiceSttProvider;
}

#[cfg(feature = "voice")]
impl IntoVoiceSttProvider for SttProviderId {
    fn into_voice_stt_provider(self) -> moltis_config::VoiceSttProvider {
        match self {
            SttProviderId::Whisper => moltis_config::VoiceSttProvider::Whisper,
            SttProviderId::Groq => moltis_config::VoiceSttProvider::Groq,
            SttProviderId::Deepgram => moltis_config::VoiceSttProvider::Deepgram,
            SttProviderId::Google => moltis_config::VoiceSttProvider::Google,
            SttProviderId::Mistral => moltis_config::VoiceSttProvider::Mistral,
            SttProviderId::VoxtralLocal => moltis_config::VoiceSttProvider::VoxtralLocal,
            SttProviderId::WhisperCli => moltis_config::VoiceSttProvider::WhisperCli,
            SttProviderId::SherpaOnnx => moltis_config::VoiceSttProvider::SherpaOnnx,
            SttProviderId::ElevenLabs => moltis_config::VoiceSttProvider::ElevenLabs,
        }
    }
}

// ── TTS Service ─────────────────────────────────────────────────────────────

/// Live TTS service that delegates to voice providers.
/// Reads fresh config on each operation to pick up changes.
#[cfg(feature = "voice")]
pub struct LiveTtsService {
    _marker: std::marker::PhantomData<()>,
}

#[cfg(feature = "voice")]
impl std::fmt::Debug for LiveTtsService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveTtsService").finish()
    }
}

#[cfg(feature = "voice")]
impl LiveTtsService {
    /// Create a new TTS service. Config is read fresh on each operation.
    pub fn new(_config: TtsConfig) -> Self {
        Self {
            _marker: std::marker::PhantomData,
        }
    }

    /// Create from environment variables (same as new, config read on demand).
    pub fn from_env() -> Self {
        Self {
            _marker: std::marker::PhantomData,
        }
    }

    /// Load fresh TTS config from disk.
    fn load_config() -> moltis_voice::TtsConfig {
        let cfg = moltis_config::discover_and_load();
        moltis_voice::TtsConfig {
            enabled: cfg.voice.tts.enabled,
            provider: cfg.voice.tts.provider.clone(),
            auto: moltis_voice::TtsAutoMode::Off,
            max_text_length: 2000,
            elevenlabs: moltis_voice::ElevenLabsConfig {
                api_key: cfg.voice.tts.elevenlabs.api_key.clone(),
                voice_id: cfg.voice.tts.elevenlabs.voice_id.clone(),
                model: cfg.voice.tts.elevenlabs.model.clone(),
                stability: None,
                similarity_boost: None,
            },
            openai: moltis_voice::OpenAiTtsConfig {
                api_key: cfg.voice.tts.openai.api_key.clone(),
                voice: cfg.voice.tts.openai.voice.clone(),
                model: cfg.voice.tts.openai.model.clone(),
                speed: None,
            },
            google: moltis_voice::GoogleTtsConfig {
                api_key: cfg.voice.tts.google.api_key.clone(),
                voice: cfg.voice.tts.google.voice.clone(),
                language_code: cfg.voice.tts.google.language_code.clone(),
                speaking_rate: None,
                pitch: None,
            },
            piper: moltis_voice::PiperTtsConfig {
                binary_path: cfg.voice.tts.piper.binary_path.clone(),
                model_path: cfg.voice.tts.piper.model_path.clone(),
                config_path: None,
                speaker_id: None,
                length_scale: None,
            },
            coqui: moltis_voice::CoquiTtsConfig {
                endpoint: cfg.voice.tts.coqui.endpoint.clone(),
                model: cfg.voice.tts.coqui.model.clone(),
                speaker: None,
                language: None,
            },
        }
    }

    /// Create a provider on-demand from fresh config.
    fn create_provider(provider_id: TtsProviderId) -> Option<Box<dyn TtsProvider + Send + Sync>> {
        let config = Self::load_config();
        match provider_id {
            TtsProviderId::ElevenLabs => config.elevenlabs.api_key.as_ref().map(|key| {
                Box::new(ElevenLabsTts::with_defaults(
                    Some(key.clone()),
                    config.elevenlabs.voice_id.clone(),
                    config.elevenlabs.model.clone(),
                )) as Box<dyn TtsProvider + Send + Sync>
            }),
            TtsProviderId::OpenAi => config.openai.api_key.as_ref().map(|key| {
                Box::new(OpenAiTts::with_defaults(
                    Some(key.clone()),
                    config.openai.voice.clone(),
                    config.openai.model.clone(),
                )) as Box<dyn TtsProvider + Send + Sync>
            }),
            TtsProviderId::Google => config.google.api_key.as_ref().map(|_| {
                Box::new(GoogleTts::new(&config.google)) as Box<dyn TtsProvider + Send + Sync>
            }),
            TtsProviderId::Piper => {
                let piper = PiperTts::new(&config.piper);
                if piper.is_configured() {
                    Some(Box::new(piper) as Box<dyn TtsProvider + Send + Sync>)
                } else {
                    None
                }
            },
            TtsProviderId::Coqui => {
                let coqui = CoquiTts::new(&config.coqui);
                if coqui.is_configured() {
                    Some(Box::new(coqui) as Box<dyn TtsProvider + Send + Sync>)
                } else {
                    None
                }
            },
        }
    }

    /// List all providers with their configuration status.
    fn list_providers() -> Vec<(TtsProviderId, bool)> {
        let config = Self::load_config();
        vec![
            (
                TtsProviderId::ElevenLabs,
                config.elevenlabs.api_key.is_some(),
            ),
            (TtsProviderId::OpenAi, config.openai.api_key.is_some()),
            (TtsProviderId::Google, config.google.api_key.is_some()),
            (TtsProviderId::Piper, config.piper.model_path.is_some()),
            (TtsProviderId::Coqui, true), // Always available if server running
        ]
    }
}

#[cfg(feature = "voice")]
#[async_trait]
impl TtsService for LiveTtsService {
    async fn status(&self) -> ServiceResult {
        let config = Self::load_config();
        let providers = Self::list_providers();
        let any_configured = providers.iter().any(|(_, configured)| *configured);

        Ok(json!({
            "enabled": config.enabled && any_configured,
            "provider": config.provider,
            "auto": format!("{:?}", config.auto).to_lowercase(),
            "maxTextLength": config.max_text_length,
            "configured": any_configured,
        }))
    }

    async fn providers(&self) -> ServiceResult {
        let providers: Vec<_> = Self::list_providers()
            .into_iter()
            .map(|(id, configured)| {
                json!({
                    "id": id,  // Uses serde serialization for consistent IDs
                    "name": id.name(),
                    "configured": configured,
                })
            })
            .collect();

        Ok(json!(providers))
    }

    async fn enable(&self, params: Value) -> ServiceResult {
        let config = Self::load_config();

        let provider_str = params
            .get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or(&config.provider);

        let provider_id = TtsProviderId::parse(provider_str)
            .ok_or_else(|| format!("unknown TTS provider '{}'", provider_str))?;

        if Self::create_provider(provider_id).is_none() {
            return Err(format!("provider '{}' not configured", provider_id));
        }

        // Update config file
        moltis_config::update_config(|cfg| {
            cfg.voice.tts.provider = provider_id.to_string();
            cfg.voice.tts.enabled = true;
        })
        .map_err(|e| format!("failed to update config: {}", e))?;

        debug!("TTS enabled with provider: {}", provider_id);

        Ok(json!({
            "enabled": true,
            "provider": provider_id,  // Uses serde serialization
        }))
    }

    async fn disable(&self) -> ServiceResult {
        moltis_config::update_config(|cfg| {
            cfg.voice.tts.enabled = false;
        })
        .map_err(|e| format!("failed to update config: {}", e))?;

        debug!("TTS disabled");

        Ok(json!({ "enabled": false }))
    }

    async fn convert(&self, params: Value) -> ServiceResult {
        let config = Self::load_config();

        if !config.enabled {
            return Err("TTS is not enabled".to_string());
        }

        let text = params
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or("missing 'text' parameter")?;

        if text.len() > config.max_text_length {
            return Err(format!(
                "text exceeds max length ({} > {})",
                text.len(),
                config.max_text_length
            ));
        }

        let provider_str = params
            .get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or(&config.provider);

        let provider_id = TtsProviderId::parse(provider_str)
            .ok_or_else(|| format!("unknown TTS provider '{}'", provider_str))?;

        let provider = Self::create_provider(provider_id)
            .ok_or_else(|| format!("provider '{}' not configured", provider_id))?;

        // Strip SSML tags for providers that don't support them natively
        let text = if provider.supports_ssml() {
            text.to_string()
        } else {
            strip_ssml_tags(text).into_owned()
        };

        let format = params
            .get("format")
            .and_then(|v| v.as_str())
            .map(|f| match f {
                "opus" | "ogg" => AudioFormat::Opus,
                "aac" => AudioFormat::Aac,
                "pcm" => AudioFormat::Pcm,
                _ => AudioFormat::Mp3,
            })
            .unwrap_or(AudioFormat::Mp3);

        let request = SynthesizeRequest {
            text,
            voice_id: params
                .get("voiceId")
                .and_then(|v| v.as_str())
                .map(String::from),
            model: params
                .get("model")
                .and_then(|v| v.as_str())
                .map(String::from),
            output_format: format,
            speed: params
                .get("speed")
                .and_then(|v| v.as_f64())
                .map(|v| v as f32),
            stability: params
                .get("stability")
                .and_then(|v| v.as_f64())
                .map(|v| v as f32),
            similarity_boost: params
                .get("similarityBoost")
                .and_then(|v| v.as_f64())
                .map(|v| v as f32),
        };

        let output = provider
            .synthesize(request)
            .await
            .map_err(|e| format!("TTS synthesis failed: {}", e))?;

        let audio_base64 = base64::engine::general_purpose::STANDARD.encode(&output.data);

        Ok(json!({
            "audio": audio_base64,
            "format": format!("{:?}", output.format).to_lowercase(),
            "mimeType": output.format.mime_type(),
            "durationMs": output.duration_ms,
            "size": output.data.len(),
        }))
    }

    async fn set_provider(&self, params: Value) -> ServiceResult {
        let provider_str = params
            .get("provider")
            .and_then(|v| v.as_str())
            .ok_or("missing 'provider' parameter")?;

        let provider_id = TtsProviderId::parse(provider_str)
            .ok_or_else(|| format!("unknown TTS provider '{}'", provider_str))?;

        if Self::create_provider(provider_id).is_none() {
            return Err(format!("provider '{}' not configured", provider_id));
        }

        moltis_config::update_config(|cfg| {
            cfg.voice.tts.provider = provider_id.to_string();
        })
        .map_err(|e| format!("failed to update config: {}", e))?;

        debug!("TTS provider set to: {}", provider_id);

        Ok(json!({
            "provider": provider_id,  // Uses serde serialization
        }))
    }
}

// ── STT Service ─────────────────────────────────────────────────────────────

/// Trait for speech-to-text services.
#[async_trait]
pub trait SttService: Send + Sync {
    /// Get STT service status.
    async fn status(&self) -> ServiceResult;
    /// List available STT providers.
    async fn providers(&self) -> ServiceResult;
    /// Transcribe audio to text.
    async fn transcribe(&self, params: Value) -> ServiceResult;
    /// Set the active STT provider.
    async fn set_provider(&self, params: Value) -> ServiceResult;
}

/// Live STT service that delegates to voice providers.
/// Reads fresh config on each operation to pick up changes.
#[cfg(feature = "voice")]
pub struct LiveSttService {
    _marker: std::marker::PhantomData<()>,
}

#[cfg(feature = "voice")]
impl std::fmt::Debug for LiveSttService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveSttService").finish()
    }
}

/// Configuration for constructing LiveSttService.
#[cfg(feature = "voice")]
pub struct SttServiceConfig {
    pub provider: String,
    pub openai_key: Option<Secret<String>>,
    pub groq_key: Option<Secret<String>>,
    pub groq_model: Option<String>,
    pub groq_language: Option<String>,
    pub deepgram_key: Option<Secret<String>>,
    pub deepgram_model: Option<String>,
    pub deepgram_language: Option<String>,
    pub deepgram_smart_format: bool,
    pub google_key: Option<Secret<String>>,
    pub google_language: Option<String>,
    pub google_model: Option<String>,
    pub mistral_key: Option<Secret<String>>,
    pub mistral_model: Option<String>,
    pub mistral_language: Option<String>,
    pub voxtral_local_endpoint: Option<String>,
    pub voxtral_local_model: Option<String>,
    pub voxtral_local_language: Option<String>,
    pub whisper_cli_binary: Option<String>,
    pub whisper_cli_model: Option<String>,
    pub whisper_cli_language: Option<String>,
    pub sherpa_onnx_binary: Option<String>,
    pub sherpa_onnx_model_dir: Option<String>,
    pub sherpa_onnx_language: Option<String>,
    pub elevenlabs_key: Option<Secret<String>>,
    pub elevenlabs_model: Option<String>,
    pub elevenlabs_language: Option<String>,
}

#[cfg(feature = "voice")]
impl Default for SttServiceConfig {
    fn default() -> Self {
        Self {
            provider: "whisper".into(),
            openai_key: None,
            groq_key: None,
            groq_model: None,
            groq_language: None,
            deepgram_key: None,
            deepgram_model: None,
            deepgram_language: None,
            deepgram_smart_format: true,
            google_key: None,
            google_language: None,
            google_model: None,
            mistral_key: None,
            mistral_model: None,
            mistral_language: None,
            voxtral_local_endpoint: None,
            voxtral_local_model: None,
            voxtral_local_language: None,
            whisper_cli_binary: None,
            whisper_cli_model: None,
            whisper_cli_language: None,
            sherpa_onnx_binary: None,
            sherpa_onnx_model_dir: None,
            sherpa_onnx_language: None,
            elevenlabs_key: None,
            elevenlabs_model: None,
            elevenlabs_language: None,
        }
    }
}

#[cfg(feature = "voice")]
impl LiveSttService {
    /// Create a new STT service. Config is read fresh on each operation.
    #[allow(unused_variables)]
    pub fn new(config: SttServiceConfig) -> Self {
        Self {
            _marker: std::marker::PhantomData,
        }
    }

    /// Create from environment variables (same as new, config read on demand).
    pub fn from_env() -> Self {
        Self {
            _marker: std::marker::PhantomData,
        }
    }

    /// Load fresh STT config from disk and create provider on demand.
    fn create_provider(provider_id: SttProviderId) -> Option<Box<dyn SttProvider + Send + Sync>> {
        let cfg = moltis_config::discover_and_load();
        match provider_id {
            SttProviderId::Whisper => cfg.voice.stt.whisper.api_key.as_ref().map(|key| {
                Box::new(WhisperStt::new(Some(key.clone()))) as Box<dyn SttProvider + Send + Sync>
            }),
            SttProviderId::Groq => cfg.voice.stt.groq.api_key.as_ref().map(|key| {
                Box::new(GroqStt::with_options(
                    Some(key.clone()),
                    cfg.voice.stt.groq.model.clone(),
                    cfg.voice.stt.groq.language.clone(),
                )) as Box<dyn SttProvider + Send + Sync>
            }),
            SttProviderId::Deepgram => cfg.voice.stt.deepgram.api_key.as_ref().map(|key| {
                Box::new(DeepgramStt::with_options(
                    Some(key.clone()),
                    cfg.voice.stt.deepgram.model.clone(),
                    cfg.voice.stt.deepgram.language.clone(),
                    cfg.voice.stt.deepgram.smart_format,
                )) as Box<dyn SttProvider + Send + Sync>
            }),
            SttProviderId::Google => cfg.voice.stt.google.api_key.as_ref().map(|key| {
                Box::new(GoogleStt::with_options(
                    Some(key.clone()),
                    cfg.voice.stt.google.language.clone(),
                    cfg.voice.stt.google.model.clone(),
                )) as Box<dyn SttProvider + Send + Sync>
            }),
            SttProviderId::Mistral => cfg.voice.stt.mistral.api_key.as_ref().map(|key| {
                Box::new(MistralStt::with_options(
                    Some(key.clone()),
                    cfg.voice.stt.mistral.model.clone(),
                    cfg.voice.stt.mistral.language.clone(),
                )) as Box<dyn SttProvider + Send + Sync>
            }),
            SttProviderId::VoxtralLocal => {
                let provider = VoxtralLocalStt::with_options(
                    Some(cfg.voice.stt.voxtral_local.endpoint.clone()),
                    cfg.voice.stt.voxtral_local.model.clone(),
                    cfg.voice.stt.voxtral_local.language.clone(),
                );
                if provider.is_configured() {
                    Some(Box::new(provider) as Box<dyn SttProvider + Send + Sync>)
                } else {
                    None
                }
            },
            SttProviderId::WhisperCli => {
                let provider = WhisperCliStt::with_options(
                    cfg.voice.stt.whisper_cli.binary_path.clone(),
                    cfg.voice.stt.whisper_cli.model_path.clone(),
                    cfg.voice.stt.whisper_cli.language.clone(),
                );
                if provider.is_configured() {
                    Some(Box::new(provider) as Box<dyn SttProvider + Send + Sync>)
                } else {
                    None
                }
            },
            SttProviderId::SherpaOnnx => {
                let provider = SherpaOnnxStt::with_options(
                    cfg.voice.stt.sherpa_onnx.binary_path.clone(),
                    cfg.voice.stt.sherpa_onnx.model_dir.clone(),
                    cfg.voice.stt.sherpa_onnx.language.clone(),
                );
                if provider.is_configured() {
                    Some(Box::new(provider) as Box<dyn SttProvider + Send + Sync>)
                } else {
                    None
                }
            },
            SttProviderId::ElevenLabs => cfg.voice.stt.elevenlabs.api_key.as_ref().map(|key| {
                Box::new(ElevenLabsStt::with_options(
                    Some(key.clone()),
                    cfg.voice.stt.elevenlabs.model.clone(),
                    cfg.voice.stt.elevenlabs.language.clone(),
                )) as Box<dyn SttProvider + Send + Sync>
            }),
        }
    }

    /// List all providers with their configuration status (reads fresh config).
    fn list_providers() -> Vec<(SttProviderId, bool)> {
        let cfg = moltis_config::discover_and_load();
        vec![
            (
                SttProviderId::Whisper,
                cfg.voice.stt.whisper.api_key.is_some(),
            ),
            (SttProviderId::Groq, cfg.voice.stt.groq.api_key.is_some()),
            (
                SttProviderId::Deepgram,
                cfg.voice.stt.deepgram.api_key.is_some(),
            ),
            (
                SttProviderId::Google,
                cfg.voice.stt.google.api_key.is_some(),
            ),
            (
                SttProviderId::Mistral,
                cfg.voice.stt.mistral.api_key.is_some(),
            ),
            (SttProviderId::VoxtralLocal, true), // Always available
            (
                SttProviderId::WhisperCli,
                cfg.voice.stt.whisper_cli.model_path.is_some(),
            ),
            (
                SttProviderId::SherpaOnnx,
                cfg.voice.stt.sherpa_onnx.model_dir.is_some(),
            ),
            (
                SttProviderId::ElevenLabs,
                cfg.voice.stt.elevenlabs.api_key.is_some(),
            ),
        ]
    }
}

#[cfg(feature = "voice")]
#[async_trait]
impl SttService for LiveSttService {
    async fn status(&self) -> ServiceResult {
        let cfg = moltis_config::discover_and_load();
        let providers = Self::list_providers();
        let any_configured = providers.iter().any(|(_, configured)| *configured);

        Ok(json!({
            "enabled": any_configured,
            "provider": cfg.voice.stt.provider,
            "configured": any_configured,
        }))
    }

    async fn providers(&self) -> ServiceResult {
        let providers: Vec<_> = Self::list_providers()
            .into_iter()
            .map(|(id, configured)| {
                json!({
                    "id": id,  // Uses serde serialization for consistent IDs
                    "name": id.name(),
                    "configured": configured,
                })
            })
            .collect();

        Ok(json!(providers))
    }

    async fn transcribe(&self, params: Value) -> ServiceResult {
        let cfg = moltis_config::discover_and_load();
        let provider_str = params
            .get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or(cfg.voice.stt.provider.as_str());

        let provider_id = SttProviderId::parse(provider_str)
            .ok_or_else(|| format!("unknown STT provider '{}'", provider_str))?;

        let provider: Box<dyn SttProvider + Send + Sync> = Self::create_provider(provider_id)
            .ok_or_else(|| format!("STT provider '{}' not configured", provider_id))?;

        let audio_base64 = params
            .get("audio")
            .and_then(|v| v.as_str())
            .ok_or("missing 'audio' parameter (base64-encoded)")?;

        let audio_data = base64::engine::general_purpose::STANDARD
            .decode(audio_base64)
            .map_err(|e| format!("invalid base64 audio: {}", e))?;

        let format = params
            .get("format")
            .and_then(|v| v.as_str())
            .map(|f| match f {
                "opus" | "ogg" => AudioFormat::Opus,
                "aac" => AudioFormat::Aac,
                "pcm" => AudioFormat::Pcm,
                _ => AudioFormat::Mp3,
            })
            .unwrap_or(AudioFormat::Mp3);

        let request = TranscribeRequest {
            audio: audio_data.into(),
            format,
            language: params
                .get("language")
                .and_then(|v| v.as_str())
                .map(String::from),
            prompt: params
                .get("prompt")
                .and_then(|v| v.as_str())
                .map(String::from),
        };

        let transcript = provider
            .transcribe(request)
            .await
            .map_err(|e| format!("transcription failed: {}", e))?;

        Ok(json!({
            "text": transcript.text,
            "language": transcript.language,
            "confidence": transcript.confidence,
            "durationSeconds": transcript.duration_seconds,
            "words": transcript.words,
        }))
    }

    async fn set_provider(&self, params: Value) -> ServiceResult {
        let provider_str = params
            .get("provider")
            .and_then(|v| v.as_str())
            .ok_or("missing 'provider' parameter")?;

        let provider_id = SttProviderId::parse(provider_str)
            .ok_or_else(|| format!("unknown STT provider '{}'", provider_str))?;

        if Self::create_provider(provider_id).is_none() {
            return Err(format!("provider '{}' not configured", provider_id));
        }

        // Update config file
        moltis_config::update_config(|cfg| {
            cfg.voice.stt.provider = provider_id.into_voice_stt_provider();
        })
        .map_err(|e| format!("failed to update config: {}", e))?;

        debug!("STT provider set to: {}", provider_id);

        Ok(json!({
            "provider": provider_id,  // Uses serde serialization
        }))
    }
}

/// No-op STT service for when voice is not configured.
pub struct NoopSttService;

#[async_trait]
impl SttService for NoopSttService {
    async fn status(&self) -> ServiceResult {
        Ok(json!({ "enabled": false, "configured": false }))
    }

    async fn providers(&self) -> ServiceResult {
        Ok(json!([]))
    }

    async fn transcribe(&self, _params: Value) -> ServiceResult {
        Err("STT not available".to_string())
    }

    async fn set_provider(&self, _params: Value) -> ServiceResult {
        Err("STT not available".to_string())
    }
}

#[cfg(all(test, feature = "voice"))]
mod tests {
    use {super::*, serde_json::json};

    #[tokio::test]
    async fn test_live_tts_service_status() {
        let service = LiveTtsService::new(TtsConfig::default());
        let status = service.status().await.unwrap();

        // Status should always contain these fields
        assert!(status.get("enabled").is_some());
        assert!(status.get("configured").is_some());
        assert!(status.get("provider").is_some());
        // Coqui is always considered "configured" (local service)
        // so configured will be true even with no API keys
        assert_eq!(status["configured"], true);
    }

    #[tokio::test]
    async fn test_live_tts_service_providers() {
        let service = LiveTtsService::new(TtsConfig::default());
        let providers = service.providers().await.unwrap();

        let providers_arr = providers.as_array().unwrap();
        // 5 providers: elevenlabs, openai, google, piper, coqui
        assert_eq!(providers_arr.len(), 5);

        let ids: Vec<_> = providers_arr
            .iter()
            .filter_map(|p| p["id"].as_str())
            .collect();
        assert!(ids.contains(&"elevenlabs"));
        assert!(ids.contains(&"openai"));
        assert!(ids.contains(&"google"));
        assert!(ids.contains(&"piper"));
        assert!(ids.contains(&"coqui"));
    }

    #[tokio::test]
    async fn test_live_tts_service_enable() {
        let service = LiveTtsService::new(TtsConfig::default());
        let result = service.enable(json!({})).await;

        // Result depends on whether a provider is configured in the environment
        // We just verify it returns a proper result (ok or error)
        let _ = result;
    }

    #[tokio::test]
    async fn test_live_tts_service_convert() {
        let service = LiveTtsService::new(TtsConfig::default());
        let result = service.convert(json!({ "text": "hello" })).await;

        // Result depends on whether TTS is enabled and configured
        // We just verify it returns a proper result (ok or error)
        let _ = result;
    }

    #[tokio::test]
    async fn test_live_stt_service_status() {
        let service = LiveSttService::new(SttServiceConfig::default());
        let status = service.status().await.unwrap();

        // Status should always contain these fields
        assert!(status.get("enabled").is_some());
        assert!(status.get("configured").is_some());
        assert!(status.get("provider").is_some());
        // voxtral-local is always considered "configured" (local service)
        // so configured will be true even with no API keys
        assert_eq!(status["configured"], true);
    }

    #[tokio::test]
    async fn test_live_stt_service_providers() {
        let service = LiveSttService::new(SttServiceConfig::default());
        let providers = service.providers().await.unwrap();

        let providers_arr = providers.as_array().unwrap();
        // Now we have 9 providers (6 cloud + 3 local)
        assert_eq!(providers_arr.len(), 9);
        // Check all providers are listed
        let ids: Vec<_> = providers_arr
            .iter()
            .filter_map(|p| p["id"].as_str())
            .collect();
        assert!(ids.contains(&"whisper"));
        assert!(ids.contains(&"groq"));
        assert!(ids.contains(&"deepgram"));
        assert!(ids.contains(&"google"));
        assert!(ids.contains(&"mistral"));
        assert!(ids.contains(&"voxtral-local"));
        assert!(ids.contains(&"whisper-cli"));
        assert!(ids.contains(&"sherpa-onnx"));
        assert!(ids.contains(&"elevenlabs-stt"));
    }

    #[tokio::test]
    async fn test_live_stt_service_transcribe() {
        let service = LiveSttService::new(SttServiceConfig::default());
        let result = service
            .transcribe(json!({
                "audio": base64::engine::general_purpose::STANDARD.encode(b"fake audio"),
                "format": "mp3"
            }))
            .await;

        // Result depends on whether an STT provider is configured
        // We just verify it returns a proper result (ok or error)
        let _ = result;
    }

    #[tokio::test]
    async fn test_noop_stt_service() {
        let service = NoopSttService;

        let status = service.status().await.unwrap();
        assert_eq!(status["enabled"], false);

        let providers = service.providers().await.unwrap();
        assert_eq!(providers.as_array().unwrap().len(), 0);

        let result = service.transcribe(json!({})).await;
        assert!(result.is_err());
    }
}
