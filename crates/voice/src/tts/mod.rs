//! Text-to-Speech provider abstraction and implementations.

mod coqui;
mod elevenlabs;
mod google;
mod openai;
mod piper;

pub use {
    coqui::CoquiTts, elevenlabs::ElevenLabsTts, google::GoogleTts, openai::OpenAiTts,
    piper::PiperTts,
};

use {
    anyhow::Result,
    async_trait::async_trait,
    bytes::Bytes,
    serde::{Deserialize, Serialize},
};

/// A voice available from a TTS provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Voice {
    /// Provider-specific voice identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Optional description or tags.
    pub description: Option<String>,
    /// Preview URL if available.
    pub preview_url: Option<String>,
}

/// Audio output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioFormat {
    /// MP3 format (widely compatible).
    #[default]
    Mp3,
    /// Opus in OGG container (good for Telegram voice notes).
    Opus,
    /// AAC format.
    Aac,
    /// PCM (raw audio).
    Pcm,
}

impl AudioFormat {
    /// MIME type for this format.
    #[must_use]
    pub fn mime_type(&self) -> &'static str {
        match self {
            Self::Mp3 => "audio/mpeg",
            Self::Opus => "audio/ogg",
            Self::Aac => "audio/aac",
            Self::Pcm => "audio/pcm",
        }
    }

    /// File extension for this format.
    #[must_use]
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Opus => "ogg",
            Self::Aac => "aac",
            Self::Pcm => "pcm",
        }
    }
}

/// Request to synthesize speech from text.
#[derive(Debug, Clone, Default)]
pub struct SynthesizeRequest {
    /// Text to convert to speech.
    pub text: String,
    /// Voice ID (provider-specific).
    pub voice_id: Option<String>,
    /// Model to use (provider-specific).
    pub model: Option<String>,
    /// Output audio format.
    pub output_format: AudioFormat,
    /// Speed multiplier (0.5 - 2.0).
    pub speed: Option<f32>,
    /// Stability setting (ElevenLabs-specific, 0.0 - 1.0).
    pub stability: Option<f32>,
    /// Similarity boost (ElevenLabs-specific, 0.0 - 1.0).
    pub similarity_boost: Option<f32>,
}

/// Audio output from TTS synthesis.
#[derive(Debug, Clone)]
pub struct AudioOutput {
    /// Raw audio data.
    pub data: Bytes,
    /// Audio format.
    pub format: AudioFormat,
    /// Duration in milliseconds (if known).
    pub duration_ms: Option<u64>,
}

/// Text-to-Speech provider trait.
///
/// Implementations provide speech synthesis from text using various services.
#[async_trait]
pub trait TtsProvider: Send + Sync {
    /// Provider identifier (e.g., "elevenlabs", "openai").
    fn id(&self) -> &'static str;

    /// Human-readable provider name.
    fn name(&self) -> &'static str;

    /// Check if the provider is configured and ready.
    fn is_configured(&self) -> bool;

    /// List available voices from this provider.
    async fn voices(&self) -> Result<Vec<Voice>>;

    /// Convert text to speech.
    async fn synthesize(&self, request: SynthesizeRequest) -> Result<AudioOutput>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_format_mime_type() {
        assert_eq!(AudioFormat::Mp3.mime_type(), "audio/mpeg");
        assert_eq!(AudioFormat::Opus.mime_type(), "audio/ogg");
    }

    #[test]
    fn test_audio_format_extension() {
        assert_eq!(AudioFormat::Mp3.extension(), "mp3");
        assert_eq!(AudioFormat::Opus.extension(), "ogg");
    }

    #[test]
    fn test_synthesize_request_default() {
        let req = SynthesizeRequest::default();
        assert!(req.text.is_empty());
        assert_eq!(req.output_format, AudioFormat::Mp3);
    }
}
