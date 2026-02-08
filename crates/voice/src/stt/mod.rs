//! Speech-to-Text provider abstraction and implementations.

mod cli_utils;
mod deepgram;
mod elevenlabs;
mod google;
mod groq;
mod mistral;
mod sherpa_onnx;
mod voxtral_local;
mod whisper;
mod whisper_cli;

pub use {
    deepgram::DeepgramStt, elevenlabs::ElevenLabsStt, google::GoogleStt, groq::GroqStt,
    mistral::MistralStt, sherpa_onnx::SherpaOnnxStt, voxtral_local::VoxtralLocalStt,
    whisper::WhisperStt, whisper_cli::WhisperCliStt,
};

use {
    anyhow::Result,
    async_trait::async_trait,
    bytes::Bytes,
    serde::{Deserialize, Serialize},
};

use crate::tts::AudioFormat;

/// Request to transcribe audio to text.
#[derive(Debug, Clone)]
pub struct TranscribeRequest {
    /// Raw audio data.
    pub audio: Bytes,
    /// Audio format.
    pub format: AudioFormat,
    /// Language hint (ISO 639-1 code, e.g., "en", "es", "fr").
    pub language: Option<String>,
    /// Optional prompt to guide transcription (context, terminology).
    pub prompt: Option<String>,
}

/// Transcription result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    /// Transcribed text.
    pub text: String,
    /// Detected language (ISO 639-1 code).
    pub language: Option<String>,
    /// Confidence score (0.0 - 1.0).
    pub confidence: Option<f32>,
    /// Duration of audio in seconds.
    pub duration_seconds: Option<f32>,
    /// Word-level timestamps (if available).
    pub words: Option<Vec<Word>>,
}

/// Word with timing information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Word {
    /// The word text.
    pub word: String,
    /// Start time in seconds.
    pub start: f32,
    /// End time in seconds.
    pub end: f32,
}

/// Speech-to-Text provider trait.
///
/// Implementations provide audio transcription using various services.
#[async_trait]
pub trait SttProvider: Send + Sync {
    /// Provider identifier (e.g., "whisper").
    fn id(&self) -> &'static str;

    /// Human-readable provider name.
    fn name(&self) -> &'static str;

    /// Check if the provider is configured and ready.
    fn is_configured(&self) -> bool;

    /// Transcribe audio to text.
    async fn transcribe(&self, request: TranscribeRequest) -> Result<Transcript>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transcript_serialization() {
        let transcript = Transcript {
            text: "Hello world".into(),
            language: Some("en".into()),
            confidence: Some(0.95),
            duration_seconds: Some(1.5),
            words: Some(vec![
                Word {
                    word: "Hello".into(),
                    start: 0.0,
                    end: 0.5,
                },
                Word {
                    word: "world".into(),
                    start: 0.6,
                    end: 1.0,
                },
            ]),
        };

        let json = serde_json::to_string(&transcript).unwrap();
        assert!(json.contains("\"text\":\"Hello world\""));
        assert!(json.contains("\"language\":\"en\""));

        let parsed: Transcript = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.text, "Hello world");
        assert_eq!(parsed.words.unwrap().len(), 2);
    }

    #[test]
    fn test_transcribe_request() {
        let request = TranscribeRequest {
            audio: Bytes::from_static(b"fake audio data"),
            format: AudioFormat::Mp3,
            language: Some("en".into()),
            prompt: Some("Technical discussion about Rust".into()),
        };

        assert_eq!(request.format, AudioFormat::Mp3);
        assert_eq!(request.language.as_deref(), Some("en"));
    }
}
