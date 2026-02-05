//! Voice configuration types.

use {
    secrecy::Secret,
    serde::{Deserialize, Serialize},
};

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

    /// Default provider: "elevenlabs", "openai".
    pub provider: String,

    /// Auto-speak mode.
    pub auto: TtsAutoMode,

    /// Max text length before skipping TTS (characters).
    pub max_text_length: usize,

    /// ElevenLabs-specific settings.
    pub elevenlabs: ElevenLabsConfig,

    /// OpenAI TTS settings.
    pub openai: OpenAiTtsConfig,
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

/// Speech-to-Text configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SttConfig {
    /// Enable STT globally.
    pub enabled: bool,

    /// Default provider: "whisper".
    pub provider: String,

    /// OpenAI Whisper settings.
    pub whisper: WhisperConfig,
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "whisper".into(),
            whisper: WhisperConfig::default(),
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
