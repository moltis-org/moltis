# Voice Feature Implementation Plan for Moltis

## Overview

This plan outlines the implementation of comprehensive voice capabilities in moltis, based on research of openclaw's architecture. The goal is to provide:

1. **Text-to-Speech (TTS)**: Convert AI responses to spoken audio
2. **Speech-to-Text (STT)**: Transcribe user voice input
3. **VoiceWake**: Trigger word detection and talk mode
4. **Streaming Audio**: Real-time audio playback in the web UI

## Current State

Moltis already has foundational infrastructure:

- **Service Traits** (`crates/gateway/src/services.rs:300-307, 1234-1239`):
  - `TtsService` trait with methods: `status`, `providers`, `enable`, `disable`, `convert`, `set_provider`
  - `VoicewakeService` trait with methods: `get`, `set`, `wake`, `talk_mode`
  - Both have `Noop` implementations that return "not available"

- **RPC Methods** (`crates/gateway/src/methods.rs`):
  - `tts.status`, `tts.providers`, `tts.enable`, `tts.disable`, `tts.convert`, `tts.setProvider`
  - `voicewake.get`, `voicewake.set`, `wake`, `talk.mode`

- **Broadcast System** (`crates/gateway/src/broadcast.rs`): WebSocket event broadcasting to all clients

- **Media Support** (`crates/media/src/`): Audio MIME types (ogg, mp3) already defined

- **Telegram Integration** (`crates/telegram/src/`): Already handles voice messages via `MediaKind::Voice`

---

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      Voice Crate                            │
├─────────────────────────────────────────────────────────────┤
│  TtsProvider trait          │  SttProvider trait            │
│  ├─ ElevenLabsTts          │  ├─ WhisperStt (OpenAI)      │
│  ├─ OpenAiTts              │  └─ (extensible)              │
│  └─ (extensible)           │                               │
└─────────────────────────────────────────────────────────────┘
          │                              │
          ▼                              ▼
┌─────────────────────────────────────────────────────────────┐
│                    Gateway Services                         │
│  ├─ TtsService (concrete impl)                             │
│  ├─ SttService (new trait + impl)                          │
│  └─ VoicewakeService (concrete impl)                       │
└─────────────────────────────────────────────────────────────┘
          │
          ▼
┌─────────────────────────────────────────────────────────────┐
│                     Web UI / Channels                       │
│  ├─ Audio playback (Web Audio API)                         │
│  ├─ Microphone capture (MediaRecorder)                     │
│  └─ Voice indicator / waveform visualization               │
└─────────────────────────────────────────────────────────────┘
```

---

## Implementation Phases

### Phase 1: Voice Crate Foundation
- Create `crates/voice/` with provider traits
- Implement ElevenLabs TTS (primary, ~75ms latency with Flash v2.5)
- Implement OpenAI TTS (fallback)
- Add configuration types

### Phase 2: STT Integration
- Add `SttProvider` trait
- Implement OpenAI Whisper provider
- Support audio transcription

### Phase 3: Gateway Integration
- Replace `NoopTtsService` with `LiveTtsService`
- Add `SttService` trait and implementation
- WebSocket audio streaming

### Phase 4: VoiceWake
- Implement `LiveVoicewakeService`
- Trigger word storage and sync
- WebSocket state broadcasting

### Phase 5: Web UI
- Audio playback module
- Microphone capture
- Voice settings page

### Phase 6: Channel Integration
- Auto-transcribe Telegram voice messages
- Optional TTS for channel responses

---

## Key Design Decisions

1. **Provider Abstraction**: All TTS/STT providers implement traits for easy swapping
2. **Streaming First**: Use streaming APIs where available for lower latency
3. **Secret Handling**: All API keys use `Secret<String>` from secrecy crate
4. **Async All The Way**: No blocking calls in async contexts
5. **Configuration Driven**: All voice settings in `moltis.toml`

---

## References

- [ElevenLabs API Documentation](https://elevenlabs.io/docs)
- [ElevenLabs Latency Optimization](https://elevenlabs.io/docs/developers/best-practices/latency-optimization)
- [OpenAI Whisper API](https://platform.openai.com/docs/guides/speech-to-text)
- [OpenAI TTS API](https://platform.openai.com/docs/guides/text-to-speech)
