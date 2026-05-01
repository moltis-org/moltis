//! Live STT service implementation.

use {
    async_trait::async_trait,
    base64::Engine,
    secrecy::Secret,
    serde_json::{Value, json},
    tracing::{debug, warn},
};

use moltis_voice::{
    AudioFormat, DeepgramStt, ElevenLabsStt, GoogleStt, GroqStt, MistralStt, SherpaOnnxStt,
    SttProvider, SttProviderId, TranscribeRequest, VoxtralLocalStt, WhisperCliStt, WhisperStt,
};

use crate::services::{ServiceResult, SttService};

use super::{load_voice_config, resolve_openai_key, resolve_openai_whisper_base_url};

/// Live STT service that delegates to voice providers.
/// Reads fresh config on each operation to pick up changes.
pub struct LiveSttService {
    _marker: std::marker::PhantomData<()>,
}

impl std::fmt::Debug for LiveSttService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveSttService").finish()
    }
}

/// Configuration for constructing LiveSttService.
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

    /// Load fresh STT config from disk (with KeyStore voice keys merged) and
    /// create provider on demand.
    fn create_provider(provider_id: SttProviderId) -> Option<Box<dyn SttProvider + Send + Sync>> {
        let cfg = load_voice_config();
        match provider_id {
            SttProviderId::Whisper => {
                let key = resolve_openai_key(cfg.voice.stt.whisper.api_key.as_ref(), &cfg);
                let provider = WhisperStt::with_options(
                    key,
                    resolve_openai_whisper_base_url(&cfg),
                    cfg.voice.stt.whisper.model.clone(),
                    cfg.voice.stt.whisper.language.clone(),
                );
                if provider.is_configured() {
                    Some(Box::new(provider) as Box<dyn SttProvider + Send + Sync>)
                } else {
                    None
                }
            },
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

    /// List all providers with their configuration status (reads fresh config
    /// with KeyStore voice keys merged).
    fn list_providers() -> Vec<(SttProviderId, bool)> {
        let cfg = load_voice_config();
        vec![
            (
                SttProviderId::Whisper,
                cfg.voice.stt.whisper.api_key.is_some()
                    || resolve_openai_whisper_base_url(&cfg).is_some(),
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

    /// Resolve the active provider: explicit config value, or first configured.
    fn resolve_provider(
        config_provider: Option<moltis_config::VoiceSttProvider>,
    ) -> Option<SttProviderId> {
        if let Some(p) = config_provider {
            return SttProviderId::parse(p.as_str());
        }
        // Auto-select: first configured provider
        Self::list_providers()
            .into_iter()
            .find(|(_, configured)| *configured)
            .map(|(id, _)| id)
    }
}

#[async_trait]
impl SttService for LiveSttService {
    async fn status(&self) -> ServiceResult {
        let cfg = load_voice_config();
        let providers = Self::list_providers();
        let any_configured = providers.iter().any(|(_, configured)| *configured);
        let resolved = Self::resolve_provider(cfg.voice.stt.provider);

        Ok(json!({
            "enabled": any_configured,
            "provider": resolved.map(|p| p.to_string()).unwrap_or_default(),
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
        let audio_base64 = params
            .get("audio")
            .and_then(|v| v.as_str())
            .ok_or("missing 'audio' parameter (base64-encoded)")?;

        let audio_data = base64::engine::general_purpose::STANDARD
            .decode(audio_base64)
            .map_err(|e| format!("invalid base64 audio: {}", e))?;

        let format_str = params
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("mp3");

        self.transcribe_bytes(
            audio_data.into(),
            format_str,
            params.get("provider").and_then(|v| v.as_str()),
            params.get("language").and_then(|v| v.as_str()),
            params.get("prompt").and_then(|v| v.as_str()),
        )
        .await
    }

    async fn transcribe_bytes(
        &self,
        audio: bytes::Bytes,
        format: &str,
        provider: Option<&str>,
        language: Option<&str>,
        prompt: Option<&str>,
    ) -> ServiceResult {
        let cfg = load_voice_config();
        let audio_len = audio.len();

        let provider_id = match provider {
            Some(s) => {
                SttProviderId::parse(s).ok_or_else(|| format!("unknown STT provider '{s}'"))?
            },
            None => Self::resolve_provider(cfg.voice.stt.provider)
                .ok_or_else(|| "no STT provider configured".to_string())?,
        };

        let stt_provider: Box<dyn SttProvider + Send + Sync> =
            Self::create_provider(provider_id)
                .ok_or_else(|| format!("STT provider '{}' not configured", provider_id))?;

        let request = TranscribeRequest {
            audio,
            format: AudioFormat::from_short_name(format),
            language: language.map(String::from),
            prompt: prompt.map(String::from),
        };

        debug!(
            provider = %provider_id,
            format,
            audio_bytes = audio_len,
            language = language.unwrap_or("auto"),
            has_prompt = prompt.is_some(),
            "STT transcription request"
        );

        let transcript = stt_provider.transcribe(request).await.map_err(|e| {
            warn!(
                provider = %provider_id,
                format,
                audio_bytes = audio_len,
                error = %e,
                "STT transcription failed"
            );
            format!("transcription failed: {}", e)
        })?;

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
            return Err(format!("provider '{}' not configured", provider_id).into());
        }

        // Update config file
        moltis_config::update_config(|cfg| {
            cfg.voice.stt.provider = Some(provider_id);
        })
        .map_err(|e| format!("failed to update config: {}", e))?;

        debug!("STT provider set to: {}", provider_id);

        Ok(json!({
            "provider": provider_id,  // Uses serde serialization
        }))
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, crate::services::NoopSttService, serde_json::json, tempfile::TempDir};

    struct VoiceConfigTestGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        _config_dir: TempDir,
        _data_dir: TempDir,
    }

    impl VoiceConfigTestGuard {
        fn with_config(config_toml: &str) -> Self {
            let lock = crate::config_override_test_lock();
            let config_dir = tempfile::tempdir()
                .unwrap_or_else(|error| panic!("config tempdir should be created: {error}"));
            let data_dir = tempfile::tempdir()
                .unwrap_or_else(|error| panic!("data tempdir should be created: {error}"));
            std::fs::write(config_dir.path().join("moltis.toml"), config_toml)
                .unwrap_or_else(|error| panic!("config should be written: {error}"));
            moltis_config::set_config_dir(config_dir.path().to_path_buf());
            moltis_config::set_data_dir(data_dir.path().to_path_buf());
            Self {
                _lock: lock,
                _config_dir: config_dir,
                _data_dir: data_dir,
            }
        }
    }

    impl Drop for VoiceConfigTestGuard {
        fn drop(&mut self) {
            moltis_config::clear_config_dir();
            moltis_config::clear_data_dir();
        }
    }

    #[test]
    fn test_live_stt_resolve_provider_handles_explicit_and_auto_selection() {
        assert_eq!(
            LiveSttService::resolve_provider(Some(moltis_config::VoiceSttProvider::Whisper)),
            Some(SttProviderId::Whisper)
        );
        assert!(LiveSttService::resolve_provider(None).is_some());
    }

    #[test]
    fn test_live_stt_whisper_base_url_counts_as_configured() {
        let _guard = VoiceConfigTestGuard::with_config(
            r#"
[server]
port = 18080

[voice.stt.whisper]
base_url = "http://127.0.0.1:8001/"
"#,
        );

        let providers = LiveSttService::list_providers();
        let whisper = providers
            .into_iter()
            .find(|(id, _)| *id == SttProviderId::Whisper);

        assert_eq!(whisper, Some((SttProviderId::Whisper, true)));
        // With explicit provider selection, Whisper is chosen
        assert_eq!(
            LiveSttService::resolve_provider(Some(moltis_config::VoiceSttProvider::Whisper)),
            Some(SttProviderId::Whisper)
        );
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

        let result = service
            .transcribe_bytes(bytes::Bytes::from_static(b"fake"), "mp3", None, None, None)
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "STT not available");
    }
}
