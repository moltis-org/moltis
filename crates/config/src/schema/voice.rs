use {
    secrecy::Secret,
    serde::{Deserialize, Serialize},
};

/// Voice configuration (TTS and STT).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceConfig {
    pub tts: VoiceTtsConfig,
    pub stt: VoiceSttConfig,
}

/// Voice TTS configuration for moltis.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceTtsConfig {
    /// Enable TTS globally.
    pub enabled: bool,
    /// Active provider: "openai", "elevenlabs", "google", "piper", "coqui".
    /// Empty string means auto-select the first configured provider.
    pub provider: String,
    /// Provider IDs to list in the UI. Empty means list all.
    pub providers: Vec<String>,
    /// ElevenLabs-specific settings.
    pub elevenlabs: VoiceElevenLabsConfig,
    /// OpenAI TTS settings.
    pub openai: VoiceOpenAiConfig,
    /// Google Cloud TTS settings.
    pub google: VoiceGoogleTtsConfig,
    /// Piper (local) settings.
    pub piper: VoicePiperTtsConfig,
    /// Coqui TTS (local server) settings.
    pub coqui: VoiceCoquiTtsConfig,
}

impl Default for VoiceTtsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: String::new(),
            providers: Vec::new(),
            elevenlabs: VoiceElevenLabsConfig::default(),
            openai: VoiceOpenAiConfig::default(),
            google: VoiceGoogleTtsConfig::default(),
            piper: VoicePiperTtsConfig::default(),
            coqui: VoiceCoquiTtsConfig::default(),
        }
    }
}

/// ElevenLabs provider configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceElevenLabsConfig {
    /// API key (from ELEVENLABS_API_KEY env or config).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::schema::serialize_option_secret",
        deserialize_with = "crate::schema::deserialize_option_secret"
    )]
    pub api_key: Option<Secret<String>>,
    /// Default voice ID.
    pub voice_id: Option<String>,
    /// Model to use (e.g., "eleven_flash_v2_5" for lowest latency).
    pub model: Option<String>,
}

/// OpenAI TTS/STT provider configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceOpenAiConfig {
    /// API key (from OPENAI_API_KEY env or config).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::schema::serialize_option_secret",
        deserialize_with = "crate::schema::deserialize_option_secret"
    )]
    pub api_key: Option<Secret<String>>,
    /// Override the OpenAI TTS endpoint for compatible local servers.
    pub base_url: Option<String>,
    /// Voice to use for TTS (alloy, echo, fable, onyx, nova, shimmer).
    pub voice: Option<String>,
    /// Model to use for TTS (tts-1, tts-1-hd).
    pub model: Option<String>,
}

/// Google Cloud TTS provider configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceGoogleTtsConfig {
    /// API key for Google Cloud Text-to-Speech.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::schema::serialize_option_secret",
        deserialize_with = "crate::schema::deserialize_option_secret"
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
pub struct VoicePiperTtsConfig {
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

/// Coqui TTS (local server) configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceCoquiTtsConfig {
    /// Coqui TTS server endpoint (default: http://localhost:5002).
    pub endpoint: String,
    /// Model name to use (if server supports multiple models).
    pub model: Option<String>,
    /// Speaker name or ID for multi-speaker models.
    pub speaker: Option<String>,
    /// Language code for multilingual models.
    pub language: Option<String>,
}

impl Default for VoiceCoquiTtsConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:5002".into(),
            model: None,
            speaker: None,
            language: None,
        }
    }
}

/// Voice STT configuration for moltis.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceSttConfig {
    /// Enable STT globally.
    pub enabled: bool,
    /// Active provider. None means auto-select the first configured provider.
    pub provider: Option<VoiceSttProvider>,
    /// Provider IDs to list in the UI. Empty means list all.
    pub providers: Vec<String>,
    /// Whisper (OpenAI) settings.
    pub whisper: VoiceWhisperConfig,
    /// Groq (Whisper-compatible) settings.
    pub groq: VoiceGroqSttConfig,
    /// Deepgram settings.
    pub deepgram: VoiceDeepgramConfig,
    /// Google Cloud Speech-to-Text settings.
    pub google: VoiceGoogleSttConfig,
    /// Mistral AI (Voxtral Transcribe) settings.
    pub mistral: VoiceMistralSttConfig,
    /// ElevenLabs Scribe settings.
    pub elevenlabs: VoiceElevenLabsSttConfig,
    /// Voxtral local (vLLM server) settings.
    pub voxtral_local: VoiceVoxtralLocalConfig,
    /// whisper-cli (whisper.cpp) settings.
    pub whisper_cli: VoiceWhisperCliConfig,
    /// sherpa-onnx offline settings.
    pub sherpa_onnx: VoiceSherpaOnnxConfig,
}

impl Default for VoiceSttConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: None,
            providers: Vec::new(),
            whisper: VoiceWhisperConfig::default(),
            groq: VoiceGroqSttConfig::default(),
            deepgram: VoiceDeepgramConfig::default(),
            google: VoiceGoogleSttConfig::default(),
            mistral: VoiceMistralSttConfig::default(),
            elevenlabs: VoiceElevenLabsSttConfig::default(),
            voxtral_local: VoiceVoxtralLocalConfig::default(),
            whisper_cli: VoiceWhisperCliConfig::default(),
            sherpa_onnx: VoiceSherpaOnnxConfig::default(),
        }
    }
}

/// Speech-to-Text provider identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoiceSttProvider {
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
    #[serde(rename = "elevenlabs-stt", alias = "elevenlabs")]
    ElevenLabs,
    #[serde(rename = "voxtral-local")]
    VoxtralLocal,
    #[serde(rename = "whisper-cli")]
    WhisperCli,
    #[serde(rename = "sherpa-onnx")]
    SherpaOnnx,
}

impl VoiceSttProvider {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Whisper => "whisper",
            Self::Groq => "groq",
            Self::Deepgram => "deepgram",
            Self::Google => "google",
            Self::Mistral => "mistral",
            Self::ElevenLabs => "elevenlabs-stt",
            Self::VoxtralLocal => "voxtral-local",
            Self::WhisperCli => "whisper-cli",
            Self::SherpaOnnx => "sherpa-onnx",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "whisper" => Some(Self::Whisper),
            "groq" => Some(Self::Groq),
            "deepgram" => Some(Self::Deepgram),
            "google" => Some(Self::Google),
            "mistral" => Some(Self::Mistral),
            "elevenlabs" | "elevenlabs-stt" => Some(Self::ElevenLabs),
            "voxtral-local" => Some(Self::VoxtralLocal),
            "whisper-cli" => Some(Self::WhisperCli),
            "sherpa-onnx" => Some(Self::SherpaOnnx),
            _ => None,
        }
    }
}

impl std::fmt::Display for VoiceSttProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// OpenAI Whisper configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceWhisperConfig {
    /// API key (from OPENAI_API_KEY env or config).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::schema::serialize_option_secret",
        deserialize_with = "crate::schema::deserialize_option_secret"
    )]
    pub api_key: Option<Secret<String>>,
    /// Override the Whisper endpoint for compatible local servers.
    pub base_url: Option<String>,
    /// Model to use (whisper-1).
    pub model: Option<String>,
    /// Language hint (ISO 639-1 code).
    pub language: Option<String>,
}

/// Groq STT configuration (Whisper-compatible API).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceGroqSttConfig {
    /// API key (from GROQ_API_KEY env or config).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::schema::serialize_option_secret",
        deserialize_with = "crate::schema::deserialize_option_secret"
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
pub struct VoiceDeepgramConfig {
    /// API key (from DEEPGRAM_API_KEY env or config).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::schema::serialize_option_secret",
        deserialize_with = "crate::schema::deserialize_option_secret"
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
pub struct VoiceGoogleSttConfig {
    /// API key for Google Cloud Speech-to-Text.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::schema::serialize_option_secret",
        deserialize_with = "crate::schema::deserialize_option_secret"
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
pub struct VoiceMistralSttConfig {
    /// API key (from MISTRAL_API_KEY env or config).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::schema::serialize_option_secret",
        deserialize_with = "crate::schema::deserialize_option_secret"
    )]
    pub api_key: Option<Secret<String>>,
    /// Model to use (e.g., "voxtral-mini-latest").
    pub model: Option<String>,
    /// Language hint (ISO 639-1 code).
    pub language: Option<String>,
}

/// ElevenLabs Scribe STT configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceElevenLabsSttConfig {
    /// API key (from ELEVENLABS_API_KEY env or config).
    /// Shared with TTS if not specified separately.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::schema::serialize_option_secret",
        deserialize_with = "crate::schema::deserialize_option_secret"
    )]
    pub api_key: Option<Secret<String>>,
    /// Model to use (scribe_v1 or scribe_v2).
    pub model: Option<String>,
    /// Language hint (ISO 639-1 code).
    pub language: Option<String>,
}

/// Voxtral local (vLLM server) configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceVoxtralLocalConfig {
    /// vLLM server endpoint (default: http://localhost:8000).
    pub endpoint: String,
    /// Model to use (optional, server default if not set).
    pub model: Option<String>,
    /// Language hint (ISO 639-1 code).
    pub language: Option<String>,
}

impl Default for VoiceVoxtralLocalConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:8000".into(),
            model: None,
            language: None,
        }
    }
}

/// whisper-cli (whisper.cpp) configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceWhisperCliConfig {
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
pub struct VoiceSherpaOnnxConfig {
    /// Path to sherpa-onnx-offline binary. If not set, looks in PATH.
    pub binary_path: Option<String>,
    /// Path to the ONNX model directory.
    pub model_dir: Option<String>,
    /// Language hint (ISO 639-1 code).
    pub language: Option<String>,
}
