//! Live TTS service implementation.

use {
    async_trait::async_trait,
    base64::Engine,
    serde_json::{Value, json},
    tracing::{debug, info, warn},
};

use moltis_voice::{
    AudioFormat, CoquiTts, ElevenLabsTts, GoogleTts, OpenAiTts, PiperTts, SynthesizeRequest,
    TtsConfig, TtsProvider, TtsProviderId, parse_tts_directives, strip_ssml_tags,
};

use crate::services::{ServiceError, ServiceResult, TtsService};

use super::{load_voice_config, resolve_openai_key, resolve_openai_tts_base_url};

/// Live TTS service that delegates to voice providers.
/// Reads fresh config on each operation to pick up changes.
pub struct LiveTtsService {
    _marker: std::marker::PhantomData<()>,
}

impl std::fmt::Debug for LiveTtsService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveTtsService").finish()
    }
}

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

    /// Load fresh TTS config from disk (with KeyStore voice keys merged).
    fn load_config() -> TtsConfig {
        let cfg = load_voice_config();
        TtsConfig {
            enabled: cfg.voice.tts.enabled,
            provider: cfg.voice.tts.provider,
            auto: moltis_voice::TtsAutoMode::Off,
            max_text_length: 8000,
            elevenlabs: moltis_voice::ElevenLabsConfig {
                api_key: cfg.voice.tts.elevenlabs.api_key.clone(),
                voice_id: cfg.voice.tts.elevenlabs.voice_id.clone(),
                model: cfg.voice.tts.elevenlabs.model.clone(),
                stability: None,
                similarity_boost: None,
            },
            openai: moltis_voice::OpenAiTtsConfig {
                api_key: resolve_openai_key(cfg.voice.tts.openai.api_key.as_ref(), &cfg),
                base_url: resolve_openai_tts_base_url(&cfg),
                voice: cfg.voice.tts.openai.voice.clone(),
                model: cfg.voice.tts.openai.model.clone(),
                speed: None,
            },
            google: moltis_voice::GoogleTtsConfig {
                api_key: cfg.voice.tts.google.api_key.clone(),
                voice: cfg.voice.tts.google.voice.clone(),
                model: cfg.voice.tts.google.model.clone(),
                language_code: cfg.voice.tts.google.language_code.clone(),
                speaking_rate: cfg.voice.tts.google.speaking_rate,
                pitch: cfg.voice.tts.google.pitch,
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
            TtsProviderId::OpenAi => {
                let provider = OpenAiTts::with_defaults(
                    config.openai.api_key.clone(),
                    config.openai.base_url.clone(),
                    config.openai.voice.clone(),
                    config.openai.model.clone(),
                );
                if provider.is_configured() {
                    Some(Box::new(provider) as Box<dyn TtsProvider + Send + Sync>)
                } else {
                    None
                }
            },
            TtsProviderId::Google => config.google.api_key.as_ref().map(|_| {
                Box::new(GoogleTts::new(&config.google).with_model(config.google.model.clone()))
                    as Box<dyn TtsProvider + Send + Sync>
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
            (
                TtsProviderId::OpenAi,
                config.openai.api_key.is_some() || config.openai.base_url.is_some(),
            ),
            (TtsProviderId::Google, config.google.api_key.is_some()),
            (TtsProviderId::Piper, config.piper.model_path.is_some()),
            (TtsProviderId::Coqui, true), // Always available if server running
        ]
    }

    /// Resolve the active provider: explicit config value, or first configured.
    fn resolve_provider(config_provider: Option<TtsProviderId>) -> Option<TtsProviderId> {
        if let Some(id) = config_provider {
            return Some(id);
        }
        // Auto-select: first configured provider
        Self::list_providers()
            .into_iter()
            .find(|(_, configured)| *configured)
            .map(|(id, _)| id)
    }

    /// Parse provider from JSON params, falling back to config/auto-select.
    fn resolve_from_params(
        params: &Value,
        config_provider: Option<TtsProviderId>,
    ) -> Result<TtsProviderId, ServiceError> {
        match params.get("provider").and_then(|v| v.as_str()) {
            Some(s) => TtsProviderId::parse(s)
                .ok_or_else(|| ServiceError::message(format!("unknown TTS provider '{s}'"))),
            None => Self::resolve_provider(config_provider)
                .ok_or_else(|| ServiceError::message("no TTS provider configured")),
        }
    }
}

#[async_trait]
impl TtsService for LiveTtsService {
    async fn status(&self) -> ServiceResult {
        let config = Self::load_config();
        let providers = Self::list_providers();
        let any_configured = providers.iter().any(|(_, configured)| *configured);
        let resolved = Self::resolve_provider(config.provider);

        Ok(json!({
            "enabled": config.enabled && any_configured,
            "provider": resolved.map(|p| p.to_string()).unwrap_or_default(),
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
        let provider_id = Self::resolve_from_params(&params, config.provider)?;

        if Self::create_provider(provider_id).is_none() {
            return Err(format!("provider '{}' not configured", provider_id).into());
        }

        // Update config file
        moltis_config::update_config(|cfg| {
            cfg.voice.tts.provider = Some(provider_id);
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
            warn!("TTS convert called but TTS is not enabled");
            return Err("TTS is not enabled".into());
        }

        let raw_text = params
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or("missing 'text' parameter")?;

        if raw_text.len() > config.max_text_length {
            return Err(format!(
                "text exceeds max length ({} > {})",
                raw_text.len(),
                config.max_text_length
            )
            .into());
        }

        // Parse [[tts:persona=... provider=...]] directives from message text.
        let (text_after_directives, directives) = parse_tts_directives(raw_text);

        // Parse persona early so its preferred provider can influence provider selection.
        // Directive persona overrides the JSON persona param.
        // Directive persona overrides the JSON persona param.
        let persona: Option<moltis_voice::VoicePersona> = if let Some(ref dir_persona) =
            directives.persona
        {
            // Directive-specified persona — match against the resolved persona object.
            params
                .get("persona")
                .and_then(|v| serde_json::from_value::<moltis_voice::VoicePersona>(v.clone()).ok())
                .filter(|p| p.id == *dir_persona)
        } else {
            params
                .get("persona")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
        };

        // Provider resolution: explicit param -> directive -> persona's provider -> config default.
        // Track if the provider was explicitly chosen (disables fallback on failure).
        let explicit_provider = params.get("provider").and_then(|v| v.as_str()).is_some()
            || directives.provider.is_some();

        let provider_id = if let Some(s) = params.get("provider").and_then(|v| v.as_str()) {
            TtsProviderId::parse(s)
                .ok_or_else(|| ServiceError::message(format!("unknown TTS provider '{s}'")))?
        } else if let Some(ref dp) = directives.provider
            && let Some(id) = TtsProviderId::parse(dp)
            && Self::create_provider(id).is_some()
        {
            id
        } else if let Some(ref p) = persona
            && let Some(prov) = p.provider
            && Self::create_provider(prov).is_some()
        {
            prov
        } else {
            Self::resolve_provider(config.provider)
                .ok_or_else(|| ServiceError::message("no TTS provider configured"))?
        };

        let text = text_after_directives.as_ref();

        info!(
            provider = %provider_id,
            persona = ?persona.as_ref().map(|p| &p.id),
            text_len = text.len(),
            "TTS convert request"
        );

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
            .map(AudioFormat::from_short_name)
            .unwrap_or(AudioFormat::Mp3);

        let mut request = SynthesizeRequest {
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
            instructions: params
                .get("instructions")
                .and_then(|v| v.as_str())
                .map(String::from),
        };

        // Apply voice persona overrides (voice_id, model, speed, instructions).
        let persona_binding = if let Some(ref persona) = persona {
            match crate::voice_persona::apply_persona_to_request(&mut request, persona, provider_id)
            {
                Ok(()) => "applied",
                Err(moltis_voice::FallbackPolicy::ProviderDefaults) => "missing",
                Err(_policy) => {
                    // Fail policy — try fallback providers below.
                    "blocked"
                },
            }
        } else {
            "none"
        };

        // Apply per-field directive overrides (take precedence over persona bindings).
        if directives.voice_id.is_some() {
            request.voice_id = directives.voice_id.clone();
        }
        if directives.model.is_some() {
            request.model = directives.model.clone();
        }
        if let Some(s) = directives.speed {
            request.speed = Some(s);
        }
        if let Some(s) = directives.stability {
            request.stability = Some(s);
        }
        if let Some(s) = directives.similarity_boost {
            request.similarity_boost = Some(s);
        }

        // Provider fallback chain: try the selected provider, then fall through
        // to other configured providers if synthesis fails.
        // Explicit provider selection disables fallback — fail immediately.
        let mut attempted_providers = vec![provider_id];

        let output = match provider.synthesize(request.clone()).await {
            Ok(output) => output,
            Err(e) if explicit_provider => {
                warn!(provider = %provider_id, error = %e, "TTS synthesis failed (explicit provider, no fallback)");
                return Err(format!("TTS synthesis failed: {e}").into());
            },
            Err(e) => {
                warn!(provider = %provider_id, error = %e, "TTS synthesis failed, trying fallback providers");
                let mut last_error = e.to_string();

                let mut fallback_output = None;
                for (fallback_id, configured) in Self::list_providers() {
                    if !configured || attempted_providers.contains(&fallback_id) {
                        continue;
                    }
                    attempted_providers.push(fallback_id);
                    if let Some(fallback_provider) = Self::create_provider(fallback_id) {
                        // Re-apply persona for the new provider if applicable.
                        let mut fb_request = request.clone();
                        if let Some(ref persona) = persona {
                            let _ = crate::voice_persona::apply_persona_to_request(
                                &mut fb_request,
                                persona,
                                fallback_id,
                            );
                        }
                        match fallback_provider.synthesize(fb_request).await {
                            Ok(output) => {
                                info!(provider = %fallback_id, "TTS fallback succeeded");
                                fallback_output = Some((fallback_id, output));
                                break;
                            },
                            Err(fb_err) => {
                                warn!(provider = %fallback_id, error = %fb_err, "TTS fallback failed");
                                last_error = fb_err.to_string();
                            },
                        }
                    }
                }

                match fallback_output {
                    Some((_fb_id, output)) => output,
                    None => return Err(format!("TTS synthesis failed: {last_error}").into()),
                }
            },
        };

        info!(
            provider = %provider_id,
            format = ?output.format,
            audio_bytes = output.data.len(),
            duration_ms = ?output.duration_ms,
            "TTS synthesis complete"
        );

        let audio_base64 = base64::engine::general_purpose::STANDARD.encode(&output.data);

        Ok(json!({
            "audio": audio_base64,
            "format": format!("{:?}", output.format).to_lowercase(),
            "mimeType": output.format.mime_type(),
            "durationMs": output.duration_ms,
            "size": output.data.len(),
            "personaBinding": persona_binding,
            "providersAttempted": attempted_providers.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
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
            return Err(format!("provider '{}' not configured", provider_id).into());
        }

        moltis_config::update_config(|cfg| {
            cfg.voice.tts.provider = Some(provider_id);
        })
        .map_err(|e| format!("failed to update config: {}", e))?;

        debug!("TTS provider set to: {}", provider_id);

        Ok(json!({
            "provider": provider_id,  // Uses serde serialization
        }))
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, serde_json::json, tempfile::TempDir};

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
    fn test_live_tts_resolve_provider_handles_explicit_and_auto_selection() {
        assert_eq!(
            LiveTtsService::resolve_provider(Some(TtsProviderId::OpenAi)),
            Some(TtsProviderId::OpenAi)
        );
        // None means auto-select — returns first configured.
        assert!(LiveTtsService::resolve_provider(None).is_some());
    }

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
        // enable() may call update_config() which writes to the config dir.
        // Hold the config lock so concurrent tests don't see our writes.
        let _guard = VoiceConfigTestGuard::with_config("");
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
}
