//! Voice configuration types.

use {
    secrecy::Secret,
    serde::{Deserialize, Serialize},
};

// ── Provider ID Enums ───────────────────────────────────────────────────────
//
// Canonical definitions live in `moltis-config`. Re-exported here so
// downstream crates can import from `moltis_voice` without pulling in
// the full config crate.

/// Text-to-Speech provider identifiers.
pub type TtsProviderId = moltis_config::VoiceTtsProvider;

/// Speech-to-Text provider identifiers.
pub type SttProviderId = moltis_config::VoiceSttProvider;

// ── Configuration Structs ───────────────────────────────────────────────────

/// Top-level voice configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceConfig {
    pub tts: TtsConfig,
    pub stt: SttConfig,
}

/// Text-to-Speech configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TtsConfig {
    /// Enable TTS globally.
    pub enabled: bool,

    /// Preferred provider. `None` means auto-select the first configured.
    pub provider: Option<TtsProviderId>,

    /// Auto-speak mode.
    pub auto: TtsAutoMode,

    /// Max text length before skipping TTS (characters).
    pub max_text_length: usize,

    /// ElevenLabs-specific settings.
    pub elevenlabs: ElevenLabsConfig,

    /// OpenAI TTS settings.
    pub openai: OpenAiTtsConfig,

    /// Google Cloud TTS settings.
    pub google: GoogleTtsConfig,

    /// Piper (local) settings.
    pub piper: PiperTtsConfig,

    /// Coqui TTS (local) settings.
    pub coqui: CoquiTtsConfig,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: None,
            auto: TtsAutoMode::Off,
            max_text_length: 2000,
            elevenlabs: ElevenLabsConfig::default(),
            openai: OpenAiTtsConfig::default(),
            google: GoogleTtsConfig::default(),
            piper: PiperTtsConfig::default(),
            coqui: CoquiTtsConfig::default(),
        }
    }
}

/// Auto-speak mode for TTS.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TtsAutoMode {
    /// Speak all responses.
    Always,
    /// Never auto-speak.
    #[default]
    Off,
    /// Only when user sent voice input.
    Inbound,
    /// Only with explicit [[tts]] markup.
    Tagged,
}

/// ElevenLabs provider configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ElevenLabsConfig {
    /// API key (from ELEVENLABS_API_KEY env or config).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_option_secret",
        deserialize_with = "deserialize_option_secret"
    )]
    pub api_key: Option<Secret<String>>,

    /// Default voice ID.
    pub voice_id: Option<String>,

    /// Model to use (e.g., "eleven_flash_v2_5" for lowest latency).
    pub model: Option<String>,

    /// Voice stability (0.0 - 1.0).
    pub stability: Option<f32>,

    /// Similarity boost (0.0 - 1.0).
    pub similarity_boost: Option<f32>,
}

/// OpenAI TTS provider configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenAiTtsConfig {
    /// API key (from OPENAI_API_KEY env or config).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_option_secret",
        deserialize_with = "deserialize_option_secret"
    )]
    pub api_key: Option<Secret<String>>,

    /// API base URL (default: https://api.openai.com/v1).
    /// Override for OpenAI-compatible TTS servers (e.g. Chatterbox, local TTS).
    pub base_url: Option<String>,

    /// Voice to use (alloy, echo, fable, onyx, nova, shimmer).
    pub voice: Option<String>,

    /// Model to use (tts-1, tts-1-hd).
    pub model: Option<String>,

    /// Speed (0.25 - 4.0, default 1.0).
    pub speed: Option<f32>,
}

/// Google Cloud TTS provider configuration.
///
/// Supports both standard Cloud TTS v1 voices and Gemini TTS models.
/// Set `model` to a `gemini-*` value (e.g., `"gemini-2.5-flash-preview-tts"`)
/// to use Gemini TTS with voice persona instruction support.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GoogleTtsConfig {
    /// API key for Google Cloud Text-to-Speech.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_option_secret",
        deserialize_with = "deserialize_option_secret"
    )]
    pub api_key: Option<Secret<String>>,

    /// Voice name (e.g., "en-US-Neural2-A", "Algieba" for Gemini).
    pub voice: Option<String>,

    /// Model to use. Set to a `gemini-*` value for Gemini TTS
    /// (e.g., `"gemini-2.5-flash-preview-tts"`).
    pub model: Option<String>,

    /// Language code (e.g., "en-US", "fr-FR").
    pub language_code: Option<String>,

    /// Speaking rate (0.25 - 4.0, default 1.0).
    pub speaking_rate: Option<f32>,

    /// Pitch (-20.0 - 20.0, default 0.0).
    pub pitch: Option<f32>,
}

/// Piper TTS (local) configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PiperTtsConfig {
    /// Path to piper binary. If not set, looks in PATH.
    pub binary_path: Option<String>,

    /// Path to the voice model file (.onnx).
    pub model_path: Option<String>,

    /// Path to the model config file (.onnx.json). If not set, uses model_path + ".json".
    pub config_path: Option<String>,

    /// Speaker ID for multi-speaker models.
    pub speaker_id: Option<u32>,

    /// Speaking rate multiplier (default 1.0).
    pub length_scale: Option<f32>,
}

/// Coqui TTS (local) configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CoquiTtsConfig {
    /// Coqui TTS server endpoint (default: http://localhost:5002).
    pub endpoint: String,

    /// Model name to use (if server supports multiple models).
    pub model: Option<String>,

    /// Speaker name or ID for multi-speaker models.
    pub speaker: Option<String>,

    /// Language code for multilingual models.
    pub language: Option<String>,
}

impl Default for CoquiTtsConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:5002".into(),
            model: None,
            speaker: None,
            language: None,
        }
    }
}

/// ElevenLabs Scribe STT configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ElevenLabsSttConfig {
    /// API key (from ELEVENLABS_API_KEY env or config).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_option_secret",
        deserialize_with = "deserialize_option_secret"
    )]
    pub api_key: Option<Secret<String>>,

    /// Model to use (e.g., "scribe_v2").
    pub model: Option<String>,

    /// Language hint (ISO 639-1 code).
    pub language: Option<String>,
}

/// Speech-to-Text configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SttConfig {
    /// Enable STT globally.
    pub enabled: bool,

    /// Default provider: "whisper", "groq", "deepgram", "google", "mistral", "voxtral-local", "whisper-cli", "sherpa-onnx", "elevenlabs".
    pub provider: String,

    /// OpenAI Whisper settings.
    pub whisper: WhisperConfig,

    /// Groq (Whisper-compatible) settings.
    pub groq: GroqSttConfig,

    /// Deepgram settings.
    pub deepgram: DeepgramConfig,

    /// Google Cloud Speech-to-Text settings.
    pub google: GoogleSttConfig,

    /// Mistral AI (Voxtral) settings.
    pub mistral: MistralSttConfig,

    /// Voxtral local (vLLM) settings.
    pub voxtral_local: VoxtralLocalConfig,

    /// Whisper local (OpenAI-compatible server) settings.
    pub whisper_local: WhisperLocalConfig,

    /// whisper-cli (whisper.cpp) settings.
    pub whisper_cli: WhisperCliConfig,

    /// sherpa-onnx offline settings.
    pub sherpa_onnx: SherpaOnnxConfig,

    /// ElevenLabs Scribe settings.
    pub elevenlabs: ElevenLabsSttConfig,
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "whisper".into(),
            whisper: WhisperConfig::default(),
            groq: GroqSttConfig::default(),
            deepgram: DeepgramConfig::default(),
            google: GoogleSttConfig::default(),
            mistral: MistralSttConfig::default(),
            voxtral_local: VoxtralLocalConfig::default(),
            whisper_local: WhisperLocalConfig::default(),
            whisper_cli: WhisperCliConfig::default(),
            sherpa_onnx: SherpaOnnxConfig::default(),
            elevenlabs: ElevenLabsSttConfig::default(),
        }
    }
}

/// OpenAI Whisper configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WhisperConfig {
    /// API key (from OPENAI_API_KEY env or config).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_option_secret",
        deserialize_with = "deserialize_option_secret"
    )]
    pub api_key: Option<Secret<String>>,

    /// API base URL (default: https://api.openai.com/v1).
    /// Override for OpenAI-compatible STT servers (e.g. faster-whisper-server).
    pub base_url: Option<String>,

    /// Model to use (whisper-1).
    pub model: Option<String>,

    /// Language hint (ISO 639-1 code).
    pub language: Option<String>,
}

/// Groq STT configuration (Whisper-compatible API).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GroqSttConfig {
    /// API key (from GROQ_API_KEY env or config).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_option_secret",
        deserialize_with = "deserialize_option_secret"
    )]
    pub api_key: Option<Secret<String>>,

    /// Model to use (e.g., "whisper-large-v3-turbo").
    pub model: Option<String>,

    /// Language hint (ISO 639-1 code).
    pub language: Option<String>,
}

/// Deepgram STT configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DeepgramConfig {
    /// API key (from DEEPGRAM_API_KEY env or config).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_option_secret",
        deserialize_with = "deserialize_option_secret"
    )]
    pub api_key: Option<Secret<String>>,

    /// Model to use (e.g., "nova-3").
    pub model: Option<String>,

    /// Language hint (e.g., "en-US").
    pub language: Option<String>,

    /// Enable smart formatting (punctuation, capitalization).
    pub smart_format: bool,
}

/// Google Cloud Speech-to-Text configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GoogleSttConfig {
    /// API key for Google Cloud Speech-to-Text.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_option_secret",
        deserialize_with = "deserialize_option_secret"
    )]
    pub api_key: Option<Secret<String>>,

    /// Path to service account JSON file (alternative to API key).
    pub service_account_json: Option<String>,

    /// Language code (e.g., "en-US").
    pub language: Option<String>,

    /// Model variant (e.g., "latest_long", "latest_short").
    pub model: Option<String>,
}

/// Mistral AI (Voxtral Transcribe) configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MistralSttConfig {
    /// API key (from MISTRAL_API_KEY env or config).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_option_secret",
        deserialize_with = "deserialize_option_secret"
    )]
    pub api_key: Option<Secret<String>>,

    /// Model to use (e.g., "voxtral-mini-latest").
    pub model: Option<String>,

    /// Language hint (ISO 639-1 code).
    pub language: Option<String>,
}

/// whisper-cli (whisper.cpp) configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WhisperCliConfig {
    /// Path to whisper-cli binary. If not set, looks in PATH.
    pub binary_path: Option<String>,

    /// Path to the GGML model file (e.g., "~/.moltis/models/ggml-base.en.bin").
    pub model_path: Option<String>,

    /// Language hint (ISO 639-1 code).
    pub language: Option<String>,
}

/// sherpa-onnx offline configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SherpaOnnxConfig {
    /// Path to sherpa-onnx-offline binary. If not set, looks in PATH.
    pub binary_path: Option<String>,

    /// Path to the ONNX model directory.
    pub model_dir: Option<String>,

    /// Language hint (ISO 639-1 code).
    pub language: Option<String>,
}

/// Voxtral local (vLLM) configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VoxtralLocalConfig {
    /// vLLM server endpoint (default: http://localhost:8000).
    pub endpoint: String,

    /// Model to use (optional, server default if not set).
    pub model: Option<String>,

    /// Language hint (ISO 639-1 code).
    pub language: Option<String>,
}

impl Default for VoxtralLocalConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:8000".into(),
            model: None,
            language: None,
        }
    }
}

/// Whisper local (OpenAI-compatible server) configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WhisperLocalConfig {
    /// Server endpoint (default: http://localhost:8080).
    pub endpoint: String,

    /// Model to use (optional, server default if not set).
    pub model: Option<String>,

    /// Language hint (ISO 639-1 code).
    pub language: Option<String>,
}

impl Default for WhisperLocalConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:8080".into(),
            model: None,
            language: None,
        }
    }
}

// ── Voice Persona Types ───────────────────────────────────────────────────

/// A named voice persona — a reusable voice identity for TTS.
///
/// Instead of improvising voice "flair" per-message, a persona defines a
/// stable spoken identity that is injected deterministically into every
/// TTS synthesis call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoicePersona {
    /// Unique identifier (lowercase alphanumeric + hyphens, 1-50 chars).
    pub id: String,
    /// Display name (e.g., "Alfred", "Narrator").
    pub label: String,
    /// Human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Preferred TTS provider for this persona.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<TtsProviderId>,
    /// What to do when the active provider has no binding for this persona.
    #[serde(default)]
    pub fallback_policy: FallbackPolicy,
    /// Provider-neutral voice direction fields.
    #[serde(default)]
    pub prompt: VoicePersonaPrompt,
    /// Per-provider voice/model overrides.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_bindings: Vec<VoicePersonaProviderBinding>,
}

/// Provider-neutral voice character direction.
///
/// These fields describe *how* the voice should sound, independent of any
/// specific TTS provider. Providers that support instruction-based control
/// (e.g., OpenAI `gpt-4o-mini-tts`) receive a rendered version of these
/// fields. Others use the provider binding overrides (voice_id, model, etc.).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VoicePersonaPrompt {
    /// Character profile (e.g., "A wise British butler with dry wit").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Delivery style (e.g., "Measured, deliberate, slightly amused").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// Accent description (e.g., "Received Pronunciation").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    /// Speech pacing guidance (e.g., "Unhurried, with dramatic pauses").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pacing: Option<String>,
    /// Scene or context (e.g., "Speaking from a grand library").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene: Option<String>,
    /// Constraints on delivery (e.g., "Never shout", "Avoid slang").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<String>,
}

impl VoicePersonaPrompt {
    /// Render the prompt fields into a single instruction string for TTS providers.
    #[must_use]
    pub fn render(&self, label: &str) -> Option<String> {
        let mut parts = Vec::new();

        parts.push(format!("Persona: {label}"));

        if let Some(ref profile) = self.profile {
            parts.push(format!("Profile: {profile}"));
        }
        if let Some(ref style) = self.style {
            parts.push(format!("Style: {style}"));
        }
        if let Some(ref accent) = self.accent {
            parts.push(format!("Accent: {accent}"));
        }
        if let Some(ref pacing) = self.pacing {
            parts.push(format!("Pacing: {pacing}"));
        }
        if let Some(ref scene) = self.scene {
            parts.push(format!("Scene: {scene}"));
        }
        if !self.constraints.is_empty() {
            parts.push(format!("Constraints: {}", self.constraints.join(". ")));
        }

        // Only the label — no real content to send.
        if parts.len() <= 1 {
            return None;
        }

        Some(parts.join("\n"))
    }

    /// Returns `true` when all fields are empty / default.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.profile.is_none()
            && self.style.is_none()
            && self.accent.is_none()
            && self.pacing.is_none()
            && self.scene.is_none()
            && self.constraints.is_empty()
    }
}

/// Per-provider voice/model overrides for a persona.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoicePersonaProviderBinding {
    /// Which provider this binding applies to.
    pub provider: TtsProviderId,
    /// Override the default voice ID for this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_id: Option<String>,
    /// Override the default model for this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Speed multiplier override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f32>,
    /// ElevenLabs stability override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stability: Option<f32>,
    /// ElevenLabs similarity boost override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub similarity_boost: Option<f32>,
    /// Google speaking rate override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaking_rate: Option<f32>,
    /// Google pitch override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch: Option<f32>,
}

/// What to do when the active TTS provider has no binding for the persona.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FallbackPolicy {
    /// Use the provider-neutral prompt fields even without a binding.
    #[default]
    PreservePersona,
    /// Drop the persona entirely — use provider defaults.
    ProviderDefaults,
    /// Fail the synthesis attempt for this provider.
    Fail,
}

impl VoicePersona {
    /// Find the provider binding for the given provider, if one exists.
    #[must_use]
    pub fn binding_for(&self, provider: TtsProviderId) -> Option<&VoicePersonaProviderBinding> {
        self.provider_bindings
            .iter()
            .find(|b| b.provider == provider)
    }

    /// Render the persona prompt as a single instruction string.
    #[must_use]
    pub fn render_instructions(&self) -> Option<String> {
        self.prompt.render(&self.label)
    }
}

// ── Secret serialization helpers ───────────────────────────────────────────

fn serialize_option_secret<S>(
    value: &Option<Secret<String>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use secrecy::ExposeSecret;
    match value {
        Some(secret) => serializer.serialize_some(secret.expose_secret()),
        None => serializer.serialize_none(),
    }
}

fn deserialize_option_secret<'de, D>(deserializer: D) -> Result<Option<Secret<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt: Option<String> = Option::deserialize(deserializer)?;
    Ok(opt.map(Secret::new))
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tts_provider_parse_accepts_ui_aliases() {
        assert_eq!(
            TtsProviderId::parse("openai-tts"),
            Some(TtsProviderId::OpenAi)
        );
        assert_eq!(
            TtsProviderId::parse("google-tts"),
            Some(TtsProviderId::Google)
        );
    }

    #[test]
    fn test_default_tts_config() {
        let config = TtsConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.provider, None);
        assert_eq!(config.auto, TtsAutoMode::Off);
        assert_eq!(config.max_text_length, 2000);
    }

    #[test]
    fn test_default_stt_config() {
        let config = SttConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.provider, "whisper");
    }

    #[test]
    fn test_tts_auto_mode_serde() {
        let json = r#""always""#;
        let mode: TtsAutoMode = serde_json::from_str(json).unwrap();
        assert_eq!(mode, TtsAutoMode::Always);

        let json = r#""off""#;
        let mode: TtsAutoMode = serde_json::from_str(json).unwrap();
        assert_eq!(mode, TtsAutoMode::Off);
    }

    #[test]
    fn test_voice_persona_prompt_render() {
        let prompt = VoicePersonaPrompt {
            profile: Some("A wise British butler".into()),
            style: Some("Measured, deliberate".into()),
            accent: Some("Received Pronunciation".into()),
            pacing: None,
            scene: None,
            constraints: vec!["Never shout".into(), "Avoid slang".into()],
        };

        let rendered = prompt.render("Alfred").unwrap();
        assert!(rendered.contains("Persona: Alfred"));
        assert!(rendered.contains("Profile: A wise British butler"));
        assert!(rendered.contains("Style: Measured, deliberate"));
        assert!(rendered.contains("Accent: Received Pronunciation"));
        assert!(rendered.contains("Constraints: Never shout. Avoid slang"));
        assert!(!rendered.contains("Pacing:"));
        assert!(!rendered.contains("Scene:"));
    }

    #[test]
    fn test_voice_persona_prompt_render_empty() {
        let prompt = VoicePersonaPrompt::default();
        assert!(prompt.is_empty());
        assert!(prompt.render("Empty").is_none());
    }

    #[test]
    fn test_voice_persona_binding_for() {
        let persona = VoicePersona {
            id: "alfred".into(),
            label: "Alfred".into(),
            description: None,
            provider: Some(TtsProviderId::OpenAi),
            fallback_policy: FallbackPolicy::PreservePersona,
            prompt: VoicePersonaPrompt::default(),
            provider_bindings: vec![
                VoicePersonaProviderBinding {
                    provider: TtsProviderId::OpenAi,
                    voice_id: Some("cedar".into()),
                    model: Some("gpt-4o-mini-tts".into()),
                    speed: None,
                    stability: None,
                    similarity_boost: None,
                    speaking_rate: None,
                    pitch: None,
                },
                VoicePersonaProviderBinding {
                    provider: TtsProviderId::ElevenLabs,
                    voice_id: Some("voice123".into()),
                    model: None,
                    speed: None,
                    stability: Some(0.65),
                    similarity_boost: Some(0.8),
                    speaking_rate: None,
                    pitch: None,
                },
            ],
        };

        let openai = persona.binding_for(TtsProviderId::OpenAi).unwrap();
        assert_eq!(openai.voice_id.as_deref(), Some("cedar"));
        assert_eq!(openai.model.as_deref(), Some("gpt-4o-mini-tts"));

        let elevenlabs = persona.binding_for(TtsProviderId::ElevenLabs).unwrap();
        assert_eq!(elevenlabs.stability, Some(0.65));

        assert!(persona.binding_for(TtsProviderId::Google).is_none());
    }

    #[test]
    fn test_fallback_policy_serde() {
        let json = r#""preserve-persona""#;
        let policy: FallbackPolicy = serde_json::from_str(json).unwrap();
        assert_eq!(policy, FallbackPolicy::PreservePersona);

        let json = r#""provider-defaults""#;
        let policy: FallbackPolicy = serde_json::from_str(json).unwrap();
        assert_eq!(policy, FallbackPolicy::ProviderDefaults);

        let json = r#""fail""#;
        let policy: FallbackPolicy = serde_json::from_str(json).unwrap();
        assert_eq!(policy, FallbackPolicy::Fail);
    }

    #[test]
    fn test_voice_persona_roundtrip() {
        let persona = VoicePersona {
            id: "narrator".into(),
            label: "Narrator".into(),
            description: Some("Epic story narrator".into()),
            provider: Some(TtsProviderId::OpenAi),
            fallback_policy: FallbackPolicy::Fail,
            prompt: VoicePersonaPrompt {
                profile: Some("Dramatic voice".into()),
                style: Some("Commanding".into()),
                accent: None,
                pacing: Some("Slow, deliberate".into()),
                scene: Some("Narrating an epic tale".into()),
                constraints: vec!["Never whisper".into()],
            },
            provider_bindings: vec![VoicePersonaProviderBinding {
                provider: TtsProviderId::OpenAi,
                voice_id: Some("onyx".into()),
                model: Some("gpt-4o-mini-tts".into()),
                speed: Some(0.9),
                stability: None,
                similarity_boost: None,
                speaking_rate: None,
                pitch: None,
            }],
        };

        let json = serde_json::to_string(&persona).unwrap();
        let parsed: VoicePersona = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "narrator");
        assert_eq!(parsed.fallback_policy, FallbackPolicy::Fail);
        assert_eq!(parsed.provider_bindings.len(), 1);
        assert_eq!(
            parsed.prompt.scene.as_deref(),
            Some("Narrating an epic tale")
        );
    }

    #[test]
    fn test_voice_config_roundtrip() {
        let config = VoiceConfig {
            tts: TtsConfig {
                enabled: true,
                provider: Some(TtsProviderId::OpenAi),
                auto: TtsAutoMode::Inbound,
                max_text_length: 1000,
                elevenlabs: ElevenLabsConfig {
                    voice_id: Some("test-voice".into()),
                    ..Default::default()
                },
                openai: OpenAiTtsConfig::default(),
                google: GoogleTtsConfig::default(),
                piper: PiperTtsConfig::default(),
                coqui: CoquiTtsConfig::default(),
            },
            stt: SttConfig::default(),
        };

        let json = serde_json::to_string(&config).unwrap();
        let parsed: VoiceConfig = serde_json::from_str(&json).unwrap();

        assert!(parsed.tts.enabled);
        assert_eq!(parsed.tts.provider, Some(TtsProviderId::OpenAi));
        assert_eq!(parsed.tts.auto, TtsAutoMode::Inbound);
    }
}
