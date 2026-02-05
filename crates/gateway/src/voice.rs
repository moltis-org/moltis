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
use std::sync::Arc;

#[cfg(feature = "voice")]
use {base64::Engine, secrecy::Secret, tokio::sync::RwLock, tracing::debug};

#[cfg(feature = "voice")]
use moltis_voice::{
    AudioFormat, CoquiTts, DeepgramStt, ElevenLabsTts, GoogleStt, GoogleTts, GroqStt, MistralStt,
    OpenAiTts, PiperTts, SherpaOnnxStt, SttProvider, SynthesizeRequest, TranscribeRequest,
    TtsConfig, TtsProvider, VoxtralLocalStt, WhisperCliStt, WhisperStt,
};

#[cfg(feature = "voice")]
use crate::services::TtsService;

// ── TTS Service ─────────────────────────────────────────────────────────────

/// Live TTS service that delegates to voice providers.
#[cfg(feature = "voice")]
pub struct LiveTtsService {
    config: Arc<RwLock<TtsConfig>>,
    elevenlabs: Option<ElevenLabsTts>,
    openai: Option<OpenAiTts>,
    google: Option<GoogleTts>,
    piper: Option<PiperTts>,
    coqui: Option<CoquiTts>,
}

#[cfg(feature = "voice")]
impl std::fmt::Debug for LiveTtsService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveTtsService")
            .field("elevenlabs_configured", &self.elevenlabs.is_some())
            .field("openai_configured", &self.openai.is_some())
            .field("google_configured", &self.google.is_some())
            .field("piper_configured", &self.piper.is_some())
            .field("coqui_configured", &self.coqui.is_some())
            .finish()
    }
}

#[cfg(feature = "voice")]
impl LiveTtsService {
    /// Create a new TTS service from configuration.
    pub fn new(config: TtsConfig) -> Self {
        let elevenlabs = config.elevenlabs.api_key.as_ref().map(|key| {
            ElevenLabsTts::with_defaults(
                Some(key.clone()),
                config.elevenlabs.voice_id.clone(),
                config.elevenlabs.model.clone(),
            )
        });

        let openai = config.openai.api_key.as_ref().map(|key| {
            OpenAiTts::with_defaults(
                Some(key.clone()),
                config.openai.voice.clone(),
                config.openai.model.clone(),
            )
        });

        // Google Cloud TTS - requires API key
        let google = config
            .google
            .api_key
            .as_ref()
            .map(|_| GoogleTts::new(&config.google));

        // Piper (local) - always created, checks model_path internally
        let piper = Some(PiperTts::new(&config.piper));

        // Coqui TTS (local server) - always created, checks config internally
        let coqui = Some(CoquiTts::new(&config.coqui));

        Self {
            config: Arc::new(RwLock::new(config)),
            elevenlabs,
            openai,
            google,
            piper,
            coqui,
        }
    }

    /// Create from environment variables.
    pub fn from_env() -> Self {
        let elevenlabs_key = std::env::var("ELEVENLABS_API_KEY").ok().map(Secret::new);
        let openai_key = std::env::var("OPENAI_API_KEY").ok().map(Secret::new);
        let google_key = std::env::var("GOOGLE_API_KEY").ok().map(Secret::new);

        let elevenlabs = elevenlabs_key.map(|key| ElevenLabsTts::new(Some(key)));
        let openai = openai_key.map(|key| OpenAiTts::new(Some(key)));
        let google = google_key.map(|_| {
            GoogleTts::new(&moltis_voice::GoogleTtsConfig {
                api_key: std::env::var("GOOGLE_API_KEY").ok().map(Secret::new),
                ..Default::default()
            })
        });

        // Local providers
        let piper = Some(PiperTts::new(&moltis_voice::PiperTtsConfig::default()));
        let coqui = Some(CoquiTts::new(&moltis_voice::CoquiTtsConfig::default()));

        Self {
            config: Arc::new(RwLock::new(TtsConfig::default())),
            elevenlabs,
            openai,
            google,
            piper,
            coqui,
        }
    }

    /// Get the active provider based on configuration.
    fn get_provider(&self, provider_id: &str) -> Option<&dyn TtsProvider> {
        match provider_id {
            "elevenlabs" => self.elevenlabs.as_ref().map(|p| p as &dyn TtsProvider),
            "openai" => self.openai.as_ref().map(|p| p as &dyn TtsProvider),
            "google" => self.google.as_ref().map(|p| p as &dyn TtsProvider),
            "piper" => self.piper.as_ref().map(|p| p as &dyn TtsProvider),
            "coqui" => self.coqui.as_ref().map(|p| p as &dyn TtsProvider),
            _ => None,
        }
    }

    /// List all configured providers.
    fn list_providers(&self) -> Vec<(&'static str, &'static str, bool)> {
        vec![
            (
                "elevenlabs",
                "ElevenLabs",
                self.elevenlabs
                    .as_ref()
                    .is_some_and(|p: &ElevenLabsTts| p.is_configured()),
            ),
            (
                "openai",
                "OpenAI TTS",
                self.openai
                    .as_ref()
                    .is_some_and(|p: &OpenAiTts| p.is_configured()),
            ),
            (
                "google",
                "Google Cloud TTS",
                self.google
                    .as_ref()
                    .is_some_and(|p: &GoogleTts| p.is_configured()),
            ),
            (
                "piper",
                "Piper",
                self.piper
                    .as_ref()
                    .is_some_and(|p: &PiperTts| p.is_configured()),
            ),
            (
                "coqui",
                "Coqui TTS",
                self.coqui
                    .as_ref()
                    .is_some_and(|p: &CoquiTts| p.is_configured()),
            ),
        ]
    }
}

#[cfg(feature = "voice")]
#[async_trait]
impl TtsService for LiveTtsService {
    async fn status(&self) -> ServiceResult {
        let config = self.config.read().await;
        let providers = self.list_providers();
        let any_configured = providers.iter().any(|(_, _, configured)| *configured);

        Ok(json!({
            "enabled": config.enabled && any_configured,
            "provider": config.provider,
            "auto": format!("{:?}", config.auto).to_lowercase(),
            "maxTextLength": config.max_text_length,
            "configured": any_configured,
        }))
    }

    async fn providers(&self) -> ServiceResult {
        let providers: Vec<_> = self
            .list_providers()
            .into_iter()
            .map(|(id, name, configured)| {
                json!({
                    "id": id,
                    "name": name,
                    "configured": configured,
                })
            })
            .collect();

        Ok(json!(providers))
    }

    async fn enable(&self, params: Value) -> ServiceResult {
        let mut config = self.config.write().await;

        let provider_id = params
            .get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or(&config.provider);

        if self.get_provider(provider_id).is_none() {
            return Err(format!("provider '{}' not configured", provider_id));
        }

        config.provider = provider_id.to_string();
        config.enabled = true;
        debug!("TTS enabled with provider: {}", config.provider);

        Ok(json!({
            "enabled": true,
            "provider": config.provider,
        }))
    }

    async fn disable(&self) -> ServiceResult {
        let mut config = self.config.write().await;
        config.enabled = false;
        debug!("TTS disabled");

        Ok(json!({ "enabled": false }))
    }

    async fn convert(&self, params: Value) -> ServiceResult {
        let config = self.config.read().await;

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

        let provider_id = params
            .get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or(&config.provider);

        let provider = self
            .get_provider(provider_id)
            .ok_or_else(|| format!("provider '{}' not configured", provider_id))?;

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
            text: text.to_string(),
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
        let provider_id = params
            .get("provider")
            .and_then(|v| v.as_str())
            .ok_or("missing 'provider' parameter")?;

        if self.get_provider(provider_id).is_none() {
            return Err(format!("provider '{}' not configured", provider_id));
        }

        let mut config = self.config.write().await;
        config.provider = provider_id.to_string();
        debug!("TTS provider set to: {}", provider_id);

        Ok(json!({
            "provider": provider_id,
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
#[cfg(feature = "voice")]
pub struct LiveSttService {
    provider: String,
    whisper: Option<WhisperStt>,
    groq: Option<GroqStt>,
    deepgram: Option<DeepgramStt>,
    google: Option<GoogleStt>,
    mistral: Option<MistralStt>,
    voxtral_local: Option<VoxtralLocalStt>,
    whisper_cli: Option<WhisperCliStt>,
    sherpa_onnx: Option<SherpaOnnxStt>,
}

#[cfg(feature = "voice")]
impl std::fmt::Debug for LiveSttService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveSttService")
            .field("provider", &self.provider)
            .field(
                "whisper_configured",
                &self.whisper.as_ref().is_some_and(|p| p.is_configured()),
            )
            .field(
                "groq_configured",
                &self.groq.as_ref().is_some_and(|p| p.is_configured()),
            )
            .field(
                "deepgram_configured",
                &self.deepgram.as_ref().is_some_and(|p| p.is_configured()),
            )
            .field(
                "google_configured",
                &self.google.as_ref().is_some_and(|p| p.is_configured()),
            )
            .field(
                "mistral_configured",
                &self.mistral.as_ref().is_some_and(|p| p.is_configured()),
            )
            .field(
                "voxtral_local_configured",
                &self
                    .voxtral_local
                    .as_ref()
                    .is_some_and(|p| p.is_configured()),
            )
            .field(
                "whisper_cli_configured",
                &self.whisper_cli.as_ref().is_some_and(|p| p.is_configured()),
            )
            .field(
                "sherpa_onnx_configured",
                &self.sherpa_onnx.as_ref().is_some_and(|p| p.is_configured()),
            )
            .finish()
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
        }
    }
}

#[cfg(feature = "voice")]
impl LiveSttService {
    /// Create a new STT service from configuration.
    pub fn new(config: SttServiceConfig) -> Self {
        let whisper = config.openai_key.map(|key| WhisperStt::new(Some(key)));

        let groq = config
            .groq_key
            .map(|key| GroqStt::with_options(Some(key), config.groq_model, config.groq_language));

        let deepgram = config.deepgram_key.map(|key| {
            DeepgramStt::with_options(
                Some(key),
                config.deepgram_model,
                config.deepgram_language,
                config.deepgram_smart_format,
            )
        });

        let google = config.google_key.map(|key| {
            GoogleStt::with_options(Some(key), config.google_language, config.google_model)
        });

        let mistral = config.mistral_key.map(|key| {
            MistralStt::with_options(Some(key), config.mistral_model, config.mistral_language)
        });

        // Voxtral local is always created (connects to vLLM server)
        let voxtral_local = Some(VoxtralLocalStt::with_options(
            config.voxtral_local_endpoint,
            config.voxtral_local_model,
            config.voxtral_local_language,
        ));

        // Local providers are always created, they check config internally
        let whisper_cli = Some(WhisperCliStt::with_options(
            config.whisper_cli_binary,
            config.whisper_cli_model,
            config.whisper_cli_language,
        ));

        let sherpa_onnx = Some(SherpaOnnxStt::with_options(
            config.sherpa_onnx_binary,
            config.sherpa_onnx_model_dir,
            config.sherpa_onnx_language,
        ));

        Self {
            provider: config.provider,
            whisper,
            groq,
            deepgram,
            google,
            mistral,
            voxtral_local,
            whisper_cli,
            sherpa_onnx,
        }
    }

    /// Create from environment variables (backwards compatible).
    pub fn from_env() -> Self {
        let openai_key = std::env::var("OPENAI_API_KEY").ok().map(Secret::new);
        let groq_key = std::env::var("GROQ_API_KEY").ok().map(Secret::new);
        let deepgram_key = std::env::var("DEEPGRAM_API_KEY").ok().map(Secret::new);
        let google_key = std::env::var("GOOGLE_CLOUD_API_KEY").ok().map(Secret::new);
        let mistral_key = std::env::var("MISTRAL_API_KEY").ok().map(Secret::new);

        Self::new(SttServiceConfig {
            openai_key,
            groq_key,
            deepgram_key,
            google_key,
            mistral_key,
            ..Default::default()
        })
    }

    /// Get the active provider.
    fn get_provider(&self, provider_id: &str) -> Option<&dyn SttProvider> {
        match provider_id {
            "whisper" => self.whisper.as_ref().map(|p| p as &dyn SttProvider),
            "groq" => self.groq.as_ref().map(|p| p as &dyn SttProvider),
            "deepgram" => self.deepgram.as_ref().map(|p| p as &dyn SttProvider),
            "google" => self.google.as_ref().map(|p| p as &dyn SttProvider),
            "mistral" => self.mistral.as_ref().map(|p| p as &dyn SttProvider),
            "voxtral-local" => self.voxtral_local.as_ref().map(|p| p as &dyn SttProvider),
            "whisper-cli" => self.whisper_cli.as_ref().map(|p| p as &dyn SttProvider),
            "sherpa-onnx" => self.sherpa_onnx.as_ref().map(|p| p as &dyn SttProvider),
            _ => None,
        }
    }

    /// List all providers with their configuration status.
    fn list_providers(&self) -> Vec<(&'static str, &'static str, bool)> {
        vec![
            (
                "whisper",
                "OpenAI Whisper",
                self.whisper.as_ref().is_some_and(|p| p.is_configured()),
            ),
            (
                "groq",
                "Groq",
                self.groq.as_ref().is_some_and(|p| p.is_configured()),
            ),
            (
                "deepgram",
                "Deepgram",
                self.deepgram.as_ref().is_some_and(|p| p.is_configured()),
            ),
            (
                "google",
                "Google Cloud",
                self.google.as_ref().is_some_and(|p| p.is_configured()),
            ),
            (
                "mistral",
                "Mistral AI",
                self.mistral.as_ref().is_some_and(|p| p.is_configured()),
            ),
            (
                "voxtral-local",
                "Voxtral (Local)",
                self.voxtral_local
                    .as_ref()
                    .is_some_and(|p| p.is_configured()),
            ),
            (
                "whisper-cli",
                "whisper.cpp",
                self.whisper_cli.as_ref().is_some_and(|p| p.is_configured()),
            ),
            (
                "sherpa-onnx",
                "sherpa-onnx",
                self.sherpa_onnx.as_ref().is_some_and(|p| p.is_configured()),
            ),
        ]
    }
}

#[cfg(feature = "voice")]
#[async_trait]
impl SttService for LiveSttService {
    async fn status(&self) -> ServiceResult {
        let providers = self.list_providers();
        let any_configured = providers.iter().any(|(_, _, configured)| *configured);

        Ok(json!({
            "enabled": any_configured,
            "provider": self.provider,
            "configured": any_configured,
        }))
    }

    async fn providers(&self) -> ServiceResult {
        let providers: Vec<_> = self
            .list_providers()
            .into_iter()
            .map(|(id, name, configured)| {
                json!({
                    "id": id,
                    "name": name,
                    "configured": configured,
                })
            })
            .collect();

        Ok(json!(providers))
    }

    async fn transcribe(&self, params: Value) -> ServiceResult {
        let provider = self
            .get_provider(&self.provider)
            .ok_or("STT provider not configured")?;

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
        let provider_id = params
            .get("provider")
            .and_then(|v| v.as_str())
            .ok_or("missing 'provider' parameter")?;

        if self.get_provider(provider_id).is_none() {
            return Err(format!("provider '{}' not configured", provider_id));
        }

        // In a real impl, we'd persist this. For now, just validate.
        Ok(json!({
            "provider": provider_id,
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
    async fn test_live_tts_service_status_unconfigured() {
        let service = LiveTtsService::new(TtsConfig::default());
        let status = service.status().await.unwrap();

        assert_eq!(status["enabled"], false);
        assert_eq!(status["configured"], false);
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
    async fn test_live_tts_service_enable_without_provider() {
        let service = LiveTtsService::new(TtsConfig::default());
        let result = service.enable(json!({})).await;

        // Should fail because no provider is configured
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_live_tts_service_convert_disabled() {
        let service = LiveTtsService::new(TtsConfig::default());
        let result = service.convert(json!({ "text": "hello" })).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not enabled"));
    }

    #[tokio::test]
    async fn test_live_stt_service_status_unconfigured() {
        let service = LiveSttService::new(SttServiceConfig::default());
        let status = service.status().await.unwrap();

        assert_eq!(status["enabled"], false);
        assert_eq!(status["configured"], false);
    }

    #[tokio::test]
    async fn test_live_stt_service_providers() {
        let service = LiveSttService::new(SttServiceConfig::default());
        let providers = service.providers().await.unwrap();

        let providers_arr = providers.as_array().unwrap();
        // Now we have 8 providers (5 cloud + 3 local)
        assert_eq!(providers_arr.len(), 8);
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
    }

    #[tokio::test]
    async fn test_live_stt_service_transcribe_unconfigured() {
        let service = LiveSttService::new(SttServiceConfig::default());
        let result = service
            .transcribe(json!({
                "audio": base64::engine::general_purpose::STANDARD.encode(b"fake audio"),
                "format": "mp3"
            }))
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not configured"));
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
