//! Voice configuration types.

use {
    secrecy::Secret,
    serde::{Deserialize, Serialize},
    std::fmt,
};

// ── Provider ID Enums ───────────────────────────────────────────────────────

/// Text-to-Speech provider identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum TtsProviderId {
    #[default]
    #[serde(rename = "elevenlabs")]
    ElevenLabs,
    #[serde(rename = "openai")]
    OpenAi,
    #[serde(rename = "google")]
    Google,
    #[serde(rename = "piper")]
    Piper,
    #[serde(rename = "coqui")]
    Coqui,
}

impl fmt::Display for TtsProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ElevenLabs => write!(f, "elevenlabs"),
            Self::OpenAi => write!(f, "openai"),
            Self::Google => write!(f, "google"),
            Self::Piper => write!(f, "piper"),
            Self::Coqui => write!(f, "coqui"),
        }
    }
}

impl TtsProviderId {
    /// Parse from string (for API compatibility).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "elevenlabs" => Some(Self::ElevenLabs),
            "openai" => Some(Self::OpenAi),
            "google" => Some(Self::Google),
            "piper" => Some(Self::Piper),
            "coqui" => Some(Self::Coqui),
            _ => None,
        }
    }

    /// Get human-readable name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::ElevenLabs => "ElevenLabs",
            Self::OpenAi => "OpenAI TTS",
            Self::Google => "Google Cloud TTS",
            Self::Piper => "Piper",
            Self::Coqui => "Coqui TTS",
        }
    }

    /// All TTS provider IDs.
    pub fn all() -> &'static [Self] {
        &[
            Self::ElevenLabs,
            Self::OpenAi,
            Self::Google,
            Self::Piper,
            Self::Coqui,
        ]
    }
}

/// Speech-to-Text provider identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum SttProviderId {
    #[default]
    #[serde(rename = "whisper")]
    Whisper,
    #[serde(rename = "groq")]
    Groq,
    #[serde(rename = "deepgram")]
    Deepgram,
    #[serde(rename = "google")]
    Google,
    #[serde(rename = "mistral")]
    Mistral,
    #[serde(rename = "voxtral-local")]
    VoxtralLocal,
    #[serde(rename = "whisper-cli")]
    WhisperCli,
    #[serde(rename = "sherpa-onnx")]
    SherpaOnnx,
    #[serde(rename = "elevenlabs-stt", alias = "elevenlabs")]
    ElevenLabs,
}

impl fmt::Display for SttProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Whisper => write!(f, "whisper"),
            Self::Groq => write!(f, "groq"),
            Self::Deepgram => write!(f, "deepgram"),
            Self::Google => write!(f, "google"),
            Self::Mistral => write!(f, "mistral"),
            Self::VoxtralLocal => write!(f, "voxtral-local"),
            Self::WhisperCli => write!(f, "whisper-cli"),
            Self::SherpaOnnx => write!(f, "sherpa-onnx"),
            Self::ElevenLabs => write!(f, "elevenlabs-stt"),
        }
    }
}

impl SttProviderId {
    /// Parse from string (for API compatibility).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "whisper" => Some(Self::Whisper),
            "groq" => Some(Self::Groq),
            "deepgram" => Some(Self::Deepgram),
            "google" => Some(Self::Google),
            "mistral" => Some(Self::Mistral),
            "voxtral-local" => Some(Self::VoxtralLocal),
            "whisper-cli" => Some(Self::WhisperCli),
            "sherpa-onnx" => Some(Self::SherpaOnnx),
            "elevenlabs" | "elevenlabs-stt" => Some(Self::ElevenLabs),
            _ => None,
        }
    }

    /// Get human-readable name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Whisper => "OpenAI Whisper",
            Self::Groq => "Groq",
            Self::Deepgram => "Deepgram",
            Self::Google => "Google Cloud",
            Self::Mistral => "Mistral AI",
            Self::VoxtralLocal => "Voxtral (Local)",
            Self::WhisperCli => "whisper.cpp",
            Self::SherpaOnnx => "sherpa-onnx",
            Self::ElevenLabs => "ElevenLabs Scribe",
        }
    }

    /// All STT provider IDs.
    pub fn all() -> &'static [Self] {
        &[
            Self::Whisper,
            Self::Groq,
            Self::Deepgram,
            Self::Google,
            Self::Mistral,
            Self::VoxtralLocal,
            Self::WhisperCli,
            Self::SherpaOnnx,
            Self::ElevenLabs,
        ]
    }
}

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

    /// Default provider: "elevenlabs", "openai", "google", "piper", "coqui".
    pub provider: String,

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
            provider: "elevenlabs".into(),
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

    /// Voice to use (alloy, echo, fable, onyx, nova, shimmer).
    pub voice: Option<String>,

    /// Model to use (tts-1, tts-1-hd).
    pub model: Option<String>,

    /// Speed (0.25 - 4.0, default 1.0).
    pub speed: Option<f32>,
}

/// Google Cloud TTS provider configuration.
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

    /// Voice name (e.g., "en-US-Neural2-A", "en-US-Wavenet-D").
    pub voice: Option<String>,

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
    fn test_default_tts_config() {
        let config = TtsConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.provider, "elevenlabs");
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
    fn test_voice_config_roundtrip() {
        let config = VoiceConfig {
            tts: TtsConfig {
                enabled: true,
                provider: "openai".into(),
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
        assert_eq!(parsed.tts.provider, "openai");
        assert_eq!(parsed.tts.auto, TtsAutoMode::Inbound);
    }
}
