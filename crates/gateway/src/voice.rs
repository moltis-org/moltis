//! Voice service implementations for TTS and STT.
//!
//! This module provides concrete implementations of the `TtsService` and
//! `SttService` traits using the moltis-voice crate's providers.

use std::sync::Arc;

use {
    async_trait::async_trait,
    base64::Engine,
    secrecy::Secret,
    serde_json::{Value, json},
    tokio::sync::RwLock,
    tracing::debug,
};

use moltis_voice::{
    AudioFormat, ElevenLabsTts, OpenAiTts, SttProvider, SynthesizeRequest, TranscribeRequest,
    TtsConfig, TtsProvider, WhisperStt,
};

use crate::services::{ServiceResult, TtsService};

// ── TTS Service ─────────────────────────────────────────────────────────────

/// Live TTS service that delegates to voice providers.
pub struct LiveTtsService {
    config: Arc<RwLock<TtsConfig>>,
    elevenlabs: Option<ElevenLabsTts>,
    openai: Option<OpenAiTts>,
}

impl std::fmt::Debug for LiveTtsService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveTtsService")
            .field("elevenlabs_configured", &self.elevenlabs.is_some())
            .field("openai_configured", &self.openai.is_some())
            .finish()
    }
}

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

        Self {
            config: Arc::new(RwLock::new(config)),
            elevenlabs,
            openai,
        }
    }

    /// Create from environment variables.
    pub fn from_env() -> Self {
        let elevenlabs_key = std::env::var("ELEVENLABS_API_KEY").ok().map(Secret::new);
        let openai_key = std::env::var("OPENAI_API_KEY").ok().map(Secret::new);

        let elevenlabs = elevenlabs_key.map(|key| ElevenLabsTts::new(Some(key)));
        let openai = openai_key.map(|key| OpenAiTts::new(Some(key)));

        Self {
            config: Arc::new(RwLock::new(TtsConfig::default())),
            elevenlabs,
            openai,
        }
    }

    /// Get the active provider based on configuration.
    fn get_provider(&self, provider_id: &str) -> Option<&dyn TtsProvider> {
        match provider_id {
            "elevenlabs" => self.elevenlabs.as_ref().map(|p| p as &dyn TtsProvider),
            "openai" => self.openai.as_ref().map(|p| p as &dyn TtsProvider),
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
                "OpenAI",
                self.openai
                    .as_ref()
                    .is_some_and(|p: &OpenAiTts| p.is_configured()),
            ),
        ]
    }
}

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
pub struct LiveSttService {
    provider: String,
    whisper: Option<WhisperStt>,
}

impl std::fmt::Debug for LiveSttService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveSttService")
            .field("provider", &self.provider)
            .field("whisper_configured", &self.whisper.is_some())
            .finish()
    }
}

impl LiveSttService {
    /// Create a new STT service.
    pub fn new(openai_key: Option<Secret<String>>) -> Self {
        let whisper = openai_key.map(|key| WhisperStt::new(Some(key)));

        Self {
            provider: "whisper".into(),
            whisper,
        }
    }

    /// Create from environment variables.
    pub fn from_env() -> Self {
        let openai_key = std::env::var("OPENAI_API_KEY").ok().map(Secret::new);
        Self::new(openai_key)
    }

    /// Get the active provider.
    fn get_provider(&self, provider_id: &str) -> Option<&dyn SttProvider> {
        match provider_id {
            "whisper" => self.whisper.as_ref().map(|p| p as &dyn SttProvider),
            _ => None,
        }
    }
}

#[async_trait]
impl SttService for LiveSttService {
    async fn status(&self) -> ServiceResult {
        let configured = self
            .whisper
            .as_ref()
            .is_some_and(|p: &WhisperStt| p.is_configured());

        Ok(json!({
            "enabled": configured,
            "provider": self.provider,
            "configured": configured,
        }))
    }

    async fn providers(&self) -> ServiceResult {
        let providers = vec![json!({
            "id": "whisper",
            "name": "OpenAI Whisper",
            "configured": self.whisper.as_ref().is_some_and(|p: &WhisperStt| p.is_configured()),
        })];

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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(providers_arr.len(), 2);

        let ids: Vec<_> = providers_arr
            .iter()
            .filter_map(|p| p["id"].as_str())
            .collect();
        assert!(ids.contains(&"elevenlabs"));
        assert!(ids.contains(&"openai"));
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
        let service = LiveSttService::new(None);
        let status = service.status().await.unwrap();

        assert_eq!(status["enabled"], false);
        assert_eq!(status["configured"], false);
    }

    #[tokio::test]
    async fn test_live_stt_service_providers() {
        let service = LiveSttService::new(None);
        let providers = service.providers().await.unwrap();

        let providers_arr = providers.as_array().unwrap();
        assert_eq!(providers_arr.len(), 1);
        assert_eq!(providers_arr[0]["id"], "whisper");
    }

    #[tokio::test]
    async fn test_live_stt_service_transcribe_unconfigured() {
        let service = LiveSttService::new(None);
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
