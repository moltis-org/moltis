//! Voice service implementations for TTS and STT.
//!
//! This module provides concrete implementations of the `TtsService` and
//! `SttService` traits using the moltis-voice crate's providers.

#[cfg(feature = "voice")]
use async_trait::async_trait;
#[cfg(feature = "voice")]
use serde_json::{Value, json};

#[cfg(feature = "voice")]
use {
    base64::Engine,
    secrecy::Secret,
    tracing::{debug, info, warn},
};

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
use crate::services::{ServiceError, ServiceResult};

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

/// Load config with voice API keys merged from the credential store.
///
/// Voice API keys are stored in the [`KeyStore`] (not `moltis.toml`) so they
/// benefit from vault encryption when enabled.  This function loads the TOML
/// config and overlays any voice-specific keys found in the store, giving the
/// store priority over legacy TOML values.
#[cfg(feature = "voice")]
pub(crate) fn load_voice_config() -> moltis_config::MoltisConfig {
    let mut cfg = moltis_config::discover_and_load();
    merge_voice_keys(&mut cfg);
    cfg
}

/// Overlay voice API keys from [`KeyStore`] onto the given config.
///
/// Keys in the store take precedence over those in the TOML config.
/// Shared keys (ElevenLabs, Google) are applied to both TTS and STT sections.
#[cfg(feature = "voice")]
pub(crate) fn merge_voice_keys(cfg: &mut moltis_config::MoltisConfig) {
    let store = crate::provider_setup::KeyStore::new();

    // ElevenLabs (shared TTS + STT)
    if let Some(key) = store.load("voice-elevenlabs") {
        let secret = Secret::new(key);
        cfg.voice.tts.elevenlabs.api_key = Some(secret.clone());
        cfg.voice.stt.elevenlabs.api_key = Some(secret);
    }

    // Google (shared TTS + STT)
    if let Some(key) = store.load("voice-google") {
        let secret = Secret::new(key);
        cfg.voice.tts.google.api_key = Some(secret.clone());
        cfg.voice.stt.google.api_key = Some(secret);
    }

    // OpenAI TTS (voice-specific, separate from LLM provider key)
    if let Some(key) = store.load("voice-openai") {
        cfg.voice.tts.openai.api_key = Some(Secret::new(key.clone()));
        // Also set STT whisper key since they share the same OpenAI API
        if cfg.voice.stt.whisper.api_key.is_none() {
            cfg.voice.stt.whisper.api_key = Some(Secret::new(key));
        }
    }

    // Whisper STT (voice-specific OpenAI key for STT only)
    if let Some(key) = store.load("voice-whisper") {
        cfg.voice.stt.whisper.api_key = Some(Secret::new(key));
    }

    // Groq STT
    if let Some(key) = store.load("voice-groq") {
        cfg.voice.stt.groq.api_key = Some(Secret::new(key));
    }

    // Deepgram STT
    if let Some(key) = store.load("voice-deepgram") {
        cfg.voice.stt.deepgram.api_key = Some(Secret::new(key));
    }

    // Mistral STT
    if let Some(key) = store.load("voice-mistral") {
        cfg.voice.stt.mistral.api_key = Some(Secret::new(key));
    }
}

/// Map a UI provider name to its credential-store key name.
///
/// Shared providers (e.g. ElevenLabs TTS + STT) map to a single key so the
/// secret is stored once.
#[cfg(feature = "voice")]
pub(crate) fn voice_key_store_name(provider: &str) -> String {
    match provider {
        "elevenlabs" | "elevenlabs-stt" => "voice-elevenlabs".to_string(),
        "openai" | "openai-tts" => "voice-openai".to_string(),
        "google" | "google-tts" => "voice-google".to_string(),
        "whisper" => "voice-whisper".to_string(),
        "groq" => "voice-groq".to_string(),
        "deepgram" => "voice-deepgram".to_string(),
        "mistral" => "voice-mistral".to_string(),
        other => format!("voice-{other}"),
    }
}

/// One-time migration: move voice API keys from `moltis.toml` into the
/// [`KeyStore`] and clear them from the config file.
///
/// Called once at gateway startup.  If a voice key already exists in the
/// store the TOML value is ignored (store wins).  After migration the
/// TOML file no longer contains voice secrets.
#[cfg(feature = "voice")]
pub(crate) fn migrate_voice_keys_to_key_store(config: &moltis_config::MoltisConfig) {
    use secrecy::ExposeSecret;

    let store = crate::provider_setup::KeyStore::new();

    // (store_key, tts_key, stt_key) — for shared providers both may be Some.
    let candidates: Vec<(&str, Option<&Secret<String>>, Option<&Secret<String>>)> = vec![
        (
            "voice-elevenlabs",
            config.voice.tts.elevenlabs.api_key.as_ref(),
            config.voice.stt.elevenlabs.api_key.as_ref(),
        ),
        (
            "voice-openai",
            config.voice.tts.openai.api_key.as_ref(),
            None,
        ),
        (
            "voice-google",
            config.voice.tts.google.api_key.as_ref(),
            config.voice.stt.google.api_key.as_ref(),
        ),
        (
            "voice-whisper",
            None,
            config.voice.stt.whisper.api_key.as_ref(),
        ),
        ("voice-groq", None, config.voice.stt.groq.api_key.as_ref()),
        (
            "voice-deepgram",
            None,
            config.voice.stt.deepgram.api_key.as_ref(),
        ),
        (
            "voice-mistral",
            None,
            config.voice.stt.mistral.api_key.as_ref(),
        ),
    ];

    let mut migrated = Vec::new();
    for (store_key, tts_key, stt_key) in &candidates {
        // Skip if the store already has this key.
        if store.load(store_key).is_some() {
            continue;
        }
        // Pick whichever TOML key is present (TTS first).
        let value = tts_key.or(*stt_key);
        if let Some(secret) = value {
            let plaintext = secret.expose_secret();
            if !plaintext.is_empty() && !plaintext.starts_with('$') {
                if let Err(e) =
                    store.save_config(store_key, Some(plaintext.to_string()), None, None)
                {
                    tracing::warn!(key = store_key, error = %e, "failed to migrate voice key");
                    continue;
                }
                migrated.push(*store_key);
            }
        }
    }

    if migrated.is_empty() {
        return;
    }

    // Clear the TOML entries so secrets don't linger in the config file.
    if let Err(e) = moltis_config::update_config(|cfg| {
        for key in &migrated {
            match *key {
                "voice-elevenlabs" => {
                    cfg.voice.tts.elevenlabs.api_key = None;
                    cfg.voice.stt.elevenlabs.api_key = None;
                },
                "voice-openai" => {
                    cfg.voice.tts.openai.api_key = None;
                },
                "voice-google" => {
                    cfg.voice.tts.google.api_key = None;
                    cfg.voice.stt.google.api_key = None;
                },
                "voice-whisper" => {
                    cfg.voice.stt.whisper.api_key = None;
                },
                "voice-groq" => {
                    cfg.voice.stt.groq.api_key = None;
                },
                "voice-deepgram" => {
                    cfg.voice.stt.deepgram.api_key = None;
                },
                "voice-mistral" => {
                    cfg.voice.stt.mistral.api_key = None;
                },
                _ => {},
            }
        }
    }) {
        tracing::warn!(error = %e, "failed to clear migrated voice keys from config");
    } else {
        tracing::info!(
            count = migrated.len(),
            keys = ?migrated,
            "migrated voice API keys from moltis.toml to credential store"
        );
    }
}

/// Resolve an OpenAI API key with fallback: voice-specific config → `OPENAI_API_KEY`
/// env var → LLM provider config (`providers.openai.api_key`).
#[cfg(feature = "voice")]
fn resolve_openai_key(
    voice_key: Option<&Secret<String>>,
    cfg: &moltis_config::MoltisConfig,
) -> Option<Secret<String>> {
    voice_key
        .cloned()
        .or_else(|| std::env::var("OPENAI_API_KEY").ok().map(Secret::new))
        .or_else(|| cfg.providers.get("openai").and_then(|p| p.api_key.clone()))
}

#[cfg(feature = "voice")]
fn resolve_openai_provider_base_url(cfg: &moltis_config::MoltisConfig) -> Option<String> {
    cfg.providers.get("openai").and_then(|p| p.base_url.clone())
}

#[cfg(feature = "voice")]
fn resolve_openai_tts_base_url(cfg: &moltis_config::MoltisConfig) -> Option<String> {
    cfg.voice
        .tts
        .openai
        .base_url
        .clone()
        .or_else(|| resolve_openai_provider_base_url(cfg))
}

#[cfg(feature = "voice")]
fn resolve_openai_whisper_base_url(cfg: &moltis_config::MoltisConfig) -> Option<String> {
    cfg.voice
        .stt
        .whisper
        .base_url
        .clone()
        .or_else(|| resolve_openai_provider_base_url(cfg))
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

    /// Load fresh TTS config from disk (with KeyStore voice keys merged).
    fn load_config() -> TtsConfig {
        let cfg = load_voice_config();
        TtsConfig {
            enabled: cfg.voice.tts.enabled,
            provider: cfg.voice.tts.provider.clone(),
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
    fn resolve_provider(config_provider: &str) -> Option<TtsProviderId> {
        if !config_provider.is_empty() {
            return TtsProviderId::parse(config_provider);
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
        config_provider: &str,
    ) -> Result<TtsProviderId, ServiceError> {
        match params.get("provider").and_then(|v| v.as_str()) {
            Some(s) => TtsProviderId::parse(s)
                .ok_or_else(|| ServiceError::message(format!("unknown TTS provider '{s}'"))),
            None => Self::resolve_provider(config_provider)
                .ok_or_else(|| ServiceError::message("no TTS provider configured")),
        }
    }
}

#[cfg(feature = "voice")]
#[async_trait]
impl TtsService for LiveTtsService {
    async fn status(&self) -> ServiceResult {
        let config = Self::load_config();
        let providers = Self::list_providers();
        let any_configured = providers.iter().any(|(_, configured)| *configured);
        let resolved = Self::resolve_provider(&config.provider);

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
        let provider_id = Self::resolve_from_params(&params, &config.provider)?;

        if Self::create_provider(provider_id).is_none() {
            return Err(format!("provider '{}' not configured", provider_id).into());
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
            warn!("TTS convert called but TTS is not enabled");
            return Err("TTS is not enabled".into());
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
            )
            .into());
        }

        let provider_id = Self::resolve_from_params(&params, &config.provider)?;

        info!(
            provider = %provider_id,
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

        let output = provider.synthesize(request).await.map_err(|e| {
            warn!(provider = %provider_id, error = %e, "TTS synthesis failed");
            format!("TTS synthesis failed: {}", e)
        })?;

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

// `SttService` trait and `NoopSttService` are defined in `moltis-service-traits`
// and re-exported via `crate::services::*`.
pub use crate::services::{NoopSttService, SttService};

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

#[cfg(feature = "voice")]
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
            cfg.voice.stt.provider = Some(provider_id.into_voice_stt_provider());
        })
        .map_err(|e| format!("failed to update config: {}", e))?;

        debug!("STT provider set to: {}", provider_id);

        Ok(json!({
            "provider": provider_id,  // Uses serde serialization
        }))
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(all(test, feature = "voice"))]
mod tests {
    use {super::*, secrecy::ExposeSecret, serde_json::json, tempfile::TempDir};

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
    fn test_resolve_openai_key_prefers_voice_key_over_llm_provider_key() {
        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.providers.providers.insert(
            "openai".to_string(),
            moltis_config::schema::ProviderEntry {
                api_key: Some(Secret::new("llm-openai-key".to_string())),
                ..moltis_config::schema::ProviderEntry::default()
            },
        );

        let resolved = resolve_openai_key(Some(&Secret::new("voice-openai-key".to_string())), &cfg)
            .map(|value| value.expose_secret().to_string());
        assert_eq!(resolved.as_deref(), Some("voice-openai-key"));
    }

    #[test]
    fn test_resolve_openai_key_uses_llm_provider_key_when_voice_key_missing() {
        if std::env::var("OPENAI_API_KEY").is_ok() {
            return;
        }

        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.providers.providers.insert(
            "openai".to_string(),
            moltis_config::schema::ProviderEntry {
                api_key: Some(Secret::new("llm-openai-key".to_string())),
                ..moltis_config::schema::ProviderEntry::default()
            },
        );

        let resolved =
            resolve_openai_key(None, &cfg).map(|value| value.expose_secret().to_string());
        assert_eq!(resolved.as_deref(), Some("llm-openai-key"));
    }

    #[test]
    fn test_live_tts_resolve_provider_handles_explicit_and_auto_selection() {
        assert_eq!(
            LiveTtsService::resolve_provider("openai"),
            Some(TtsProviderId::OpenAi)
        );
        assert_eq!(LiveTtsService::resolve_provider("unknown"), None);
        assert!(LiveTtsService::resolve_provider("").is_some());
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

    #[test]
    fn test_resolve_openai_tts_base_url_prefers_voice_specific_value() {
        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.voice.tts.openai.base_url = Some("http://127.0.0.1:8003".to_string());
        cfg.providers.providers.insert(
            "openai".to_string(),
            moltis_config::schema::ProviderEntry {
                base_url: Some("http://127.0.0.1:8001".to_string()),
                ..moltis_config::schema::ProviderEntry::default()
            },
        );

        assert_eq!(
            resolve_openai_tts_base_url(&cfg).as_deref(),
            Some("http://127.0.0.1:8003")
        );
    }

    #[test]
    fn test_resolve_openai_tts_base_url_falls_back_to_provider_value() {
        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.providers.providers.insert(
            "openai".to_string(),
            moltis_config::schema::ProviderEntry {
                base_url: Some("http://127.0.0.1:8001".to_string()),
                ..moltis_config::schema::ProviderEntry::default()
            },
        );

        assert_eq!(
            resolve_openai_tts_base_url(&cfg).as_deref(),
            Some("http://127.0.0.1:8001")
        );
    }

    #[test]
    fn test_resolve_openai_whisper_base_url_prefers_voice_specific_value() {
        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.voice.stt.whisper.base_url = Some("http://127.0.0.1:8002".to_string());
        cfg.providers.providers.insert(
            "openai".to_string(),
            moltis_config::schema::ProviderEntry {
                base_url: Some("http://127.0.0.1:8001".to_string()),
                ..moltis_config::schema::ProviderEntry::default()
            },
        );

        assert_eq!(
            resolve_openai_whisper_base_url(&cfg).as_deref(),
            Some("http://127.0.0.1:8002")
        );
    }

    #[test]
    fn test_resolve_openai_whisper_base_url_falls_back_to_provider_value() {
        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.providers.providers.insert(
            "openai".to_string(),
            moltis_config::schema::ProviderEntry {
                base_url: Some("http://127.0.0.1:8001".to_string()),
                ..moltis_config::schema::ProviderEntry::default()
            },
        );

        assert_eq!(
            resolve_openai_whisper_base_url(&cfg).as_deref(),
            Some("http://127.0.0.1:8001")
        );
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

    #[test]
    fn voice_key_store_name_maps_shared_providers() {
        assert_eq!(voice_key_store_name("elevenlabs"), "voice-elevenlabs");
        assert_eq!(voice_key_store_name("elevenlabs-stt"), "voice-elevenlabs");
        assert_eq!(voice_key_store_name("openai"), "voice-openai");
        assert_eq!(voice_key_store_name("openai-tts"), "voice-openai");
        assert_eq!(voice_key_store_name("google"), "voice-google");
        assert_eq!(voice_key_store_name("google-tts"), "voice-google");
        assert_eq!(voice_key_store_name("whisper"), "voice-whisper");
        assert_eq!(voice_key_store_name("groq"), "voice-groq");
        assert_eq!(voice_key_store_name("deepgram"), "voice-deepgram");
        assert_eq!(voice_key_store_name("mistral"), "voice-mistral");
        assert_eq!(voice_key_store_name("custom"), "voice-custom");
    }

    #[test]
    fn merge_voice_keys_populates_config_from_key_store() {
        let guard = VoiceConfigTestGuard::with_config("");

        // Save a key to the store via the public save_config method.
        let store = crate::provider_setup::KeyStore::new();
        store
            .save_config("voice-elevenlabs", Some("el-test-key".into()), None, None)
            .unwrap();

        let mut cfg = moltis_config::MoltisConfig::default();
        assert!(cfg.voice.tts.elevenlabs.api_key.is_none());

        merge_voice_keys(&mut cfg);

        assert!(cfg.voice.tts.elevenlabs.api_key.is_some());
        assert_eq!(
            cfg.voice.tts.elevenlabs.api_key.unwrap().expose_secret(),
            "el-test-key"
        );

        drop(guard);
    }

    #[test]
    fn migrate_voice_keys_moves_config_keys_to_key_store() {
        let guard = VoiceConfigTestGuard::with_config("");

        // Build a config with voice keys as if they came from TOML.
        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.voice.tts.elevenlabs.api_key = Some(Secret::new("el-legacy-key".to_string()));
        cfg.voice.stt.groq.api_key = Some(Secret::new("groq-legacy-key".to_string()));

        migrate_voice_keys_to_key_store(&cfg);

        // Keys should now be in the store.
        let store = crate::provider_setup::KeyStore::new();
        assert_eq!(
            store.load("voice-elevenlabs").as_deref(),
            Some("el-legacy-key")
        );
        assert_eq!(store.load("voice-groq").as_deref(), Some("groq-legacy-key"));

        // Running again with empty config is a no-op (keys already in store).
        let cfg2 = moltis_config::MoltisConfig::default();
        migrate_voice_keys_to_key_store(&cfg2);
        assert_eq!(
            store.load("voice-elevenlabs").as_deref(),
            Some("el-legacy-key")
        );

        drop(guard);
    }

    #[test]
    fn migrate_voice_keys_skips_env_var_references() {
        let guard = VoiceConfigTestGuard::with_config("");

        let mut cfg = moltis_config::MoltisConfig::default();
        cfg.voice.tts.elevenlabs.api_key = Some(Secret::new("${ELEVENLABS_API_KEY}".to_string()));

        migrate_voice_keys_to_key_store(&cfg);

        // Env var reference should NOT be migrated.
        let store = crate::provider_setup::KeyStore::new();
        assert!(store.load("voice-elevenlabs").is_none());

        drop(guard);
    }
}
