//! Voice capabilities for moltis: Text-to-Speech (TTS) and Speech-to-Text (STT).
//!
//! This crate provides provider-agnostic abstractions for voice services,
//! with implementations for popular providers like ElevenLabs, OpenAI, and Whisper.

pub mod config;
pub mod stt;
pub mod tts;

pub use {
    config::{
        CoquiTtsConfig, DeepgramConfig, ElevenLabsConfig, ElevenLabsSttConfig, FallbackPolicy,
        GoogleSttConfig, GoogleTtsConfig, GroqSttConfig, MistralSttConfig, OpenAiTtsConfig,
        PiperTtsConfig, SherpaOnnxConfig, SttConfig, SttProviderId, TtsAutoMode, TtsConfig,
        TtsProviderId, VoiceConfig, VoicePersona, VoicePersonaPrompt, VoicePersonaProviderBinding,
        VoxtralLocalConfig, WhisperCliConfig, WhisperConfig, WhisperLocalConfig,
    },
    stt::{
        DeepgramStt, ElevenLabsStt, GoogleStt, GroqStt, MistralStt, SherpaOnnxStt, SttProvider,
        TranscribeRequest, Transcript, VoxtralLocalStt, WhisperCliStt, WhisperLocalStt, WhisperStt,
    },
    tts::{
        AudioFormat, AudioOutput, CoquiTts, ElevenLabsTts, GoogleTts, OpenAiTts, PiperTts,
        SynthesizeRequest, TtsDirectives, TtsProvider, Voice, contains_ssml, parse_tts_directives,
        sanitize_text_for_tts, strip_ssml_tags,
    },
};
