//! Voice capabilities for moltis: Text-to-Speech (TTS) and Speech-to-Text (STT).
//!
//! This crate provides provider-agnostic abstractions for voice services,
//! with implementations for popular providers like ElevenLabs, OpenAI, and Whisper.

pub mod config;
pub mod stt;
pub mod tts;

pub use {
    config::{
        CoquiTtsConfig, DeepgramConfig, ElevenLabsConfig, ElevenLabsSttConfig, GoogleSttConfig,
        GoogleTtsConfig, GroqSttConfig, MistralSttConfig, OpenAiTtsConfig, PiperTtsConfig,
        SherpaOnnxConfig, SttConfig, SttProviderId, TtsAutoMode, TtsConfig, TtsProviderId,
        VoiceConfig, VoxtralLocalConfig, WhisperCliConfig, WhisperConfig,
    },
    stt::{
        DeepgramStt, ElevenLabsStt, GoogleStt, GroqStt, MistralStt, SherpaOnnxStt, SttProvider,
        TranscribeRequest, Transcript, VoxtralLocalStt, WhisperCliStt, WhisperStt,
    },
    tts::{
        AudioFormat, AudioOutput, CoquiTts, ElevenLabsTts, GoogleTts, OpenAiTts, PiperTts,
        SynthesizeRequest, TtsProvider, Voice, contains_ssml, sanitize_text_for_tts,
        strip_ssml_tags,
    },
};
