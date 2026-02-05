//! Voice capabilities for moltis: Text-to-Speech (TTS) and Speech-to-Text (STT).
//!
//! This crate provides provider-agnostic abstractions for voice services,
//! with implementations for popular providers like ElevenLabs, OpenAI, and Whisper.

pub mod config;
pub mod stt;
pub mod tts;

pub use {
    config::{
        DeepgramConfig, GoogleSttConfig, GroqSttConfig, SherpaOnnxConfig, SttConfig, TtsAutoMode,
        TtsConfig, VoiceConfig, WhisperCliConfig, WhisperConfig,
    },
    stt::{
        DeepgramStt, GoogleStt, GroqStt, SherpaOnnxStt, SttProvider, TranscribeRequest, Transcript,
        WhisperCliStt, WhisperStt,
    },
    tts::{
        AudioFormat, AudioOutput, ElevenLabsTts, OpenAiTts, SynthesizeRequest, TtsProvider, Voice,
    },
};
