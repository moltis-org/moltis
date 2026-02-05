# Voice Services

Moltis provides text-to-speech (TTS) and speech-to-text (STT) capabilities
through the `moltis-voice` crate and gateway integration.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      Voice Crate                            │
│                   (crates/voice/)                           │
├─────────────────────────────────────────────────────────────┤
│  TtsProvider trait          │  SttProvider trait            │
│  ├─ ElevenLabsTts          │  └─ WhisperStt (OpenAI)       │
│  └─ OpenAiTts              │                               │
└─────────────────────────────────────────────────────────────┘
                    │
                    ▼
┌─────────────────────────────────────────────────────────────┐
│                    Gateway Services                         │
│                (crates/gateway/src/voice.rs)                │
├─────────────────────────────────────────────────────────────┤
│  LiveTtsService            │  LiveSttService                │
│  (wraps TTS providers)     │  (wraps STT providers)         │
└─────────────────────────────────────────────────────────────┘
                    │
                    ▼
┌─────────────────────────────────────────────────────────────┐
│                     RPC Methods                             │
├─────────────────────────────────────────────────────────────┤
│  tts.status, tts.providers, tts.enable, tts.disable,       │
│  tts.convert, tts.setProvider                              │
│  stt.status, stt.providers, stt.transcribe, stt.setProvider│
└─────────────────────────────────────────────────────────────┘
```

## Text-to-Speech (TTS)

### Supported Providers

| Provider | Model | Latency | Notes |
|----------|-------|---------|-------|
| ElevenLabs | `eleven_flash_v2_5` | ~75ms | Lowest latency, voice cloning |
| ElevenLabs | `eleven_turbo_v2_5` | ~250ms | Higher quality |
| OpenAI | `tts-1` | ~200ms | Real-time optimized |
| OpenAI | `tts-1-hd` | ~400ms | Higher quality |

### Configuration

Set API keys via environment variables:

```bash
export ELEVENLABS_API_KEY=your-key-here
export OPENAI_API_KEY=your-key-here
```

Or configure in `moltis.toml`:

```toml
[voice.tts]
enabled = true
provider = "elevenlabs"  # or "openai"
auto = "off"             # "always", "off", "inbound", "tagged"
max_text_length = 2000

[voice.tts.elevenlabs]
api_key = "sk-..."
voice_id = "21m00Tcm4TlvDq8ikWAM"  # Rachel (default)
model = "eleven_flash_v2_5"
stability = 0.5
similarity_boost = 0.75

[voice.tts.openai]
api_key = "sk-..."
voice = "alloy"  # alloy, echo, fable, onyx, nova, shimmer
model = "tts-1"
speed = 1.0
```

### RPC Methods

#### `tts.status`

Get current TTS status.

**Response:**
```json
{
  "enabled": true,
  "provider": "elevenlabs",
  "auto": "off",
  "maxTextLength": 2000,
  "configured": true
}
```

#### `tts.providers`

List available TTS providers.

**Response:**
```json
[
  { "id": "elevenlabs", "name": "ElevenLabs", "configured": true },
  { "id": "openai", "name": "OpenAI", "configured": false }
]
```

#### `tts.enable`

Enable TTS with optional provider selection.

**Request:**
```json
{ "provider": "elevenlabs" }
```

**Response:**
```json
{ "enabled": true, "provider": "elevenlabs" }
```

#### `tts.disable`

Disable TTS.

**Response:**
```json
{ "enabled": false }
```

#### `tts.convert`

Convert text to speech.

**Request:**
```json
{
  "text": "Hello, how can I help you today?",
  "provider": "elevenlabs",
  "voiceId": "21m00Tcm4TlvDq8ikWAM",
  "model": "eleven_flash_v2_5",
  "format": "mp3",
  "speed": 1.0,
  "stability": 0.5,
  "similarityBoost": 0.75
}
```

**Response:**
```json
{
  "audio": "base64-encoded-audio-data",
  "format": "mp3",
  "mimeType": "audio/mpeg",
  "durationMs": 2500,
  "size": 45000
}
```

**Audio Formats:**
- `mp3` (default) - Widely compatible
- `opus` / `ogg` - Good for Telegram voice notes
- `aac` - Apple devices
- `pcm` - Raw audio

#### `tts.setProvider`

Change the active TTS provider.

**Request:**
```json
{ "provider": "openai" }
```

### Auto-Speak Modes

| Mode | Description |
|------|-------------|
| `always` | Speak all AI responses |
| `off` | Never auto-speak (default) |
| `inbound` | Only when user sent voice input |
| `tagged` | Only with explicit `[[tts]]` markup |

## Speech-to-Text (STT)

### Supported Providers

| Provider | Model | Notes |
|----------|-------|-------|
| OpenAI Whisper | `whisper-1` | Handles accents, noise, technical terms |

### Configuration

```toml
[voice.stt]
enabled = true
provider = "whisper"

[voice.stt.whisper]
api_key = "sk-..."  # Uses OPENAI_API_KEY if not set
model = "whisper-1"
language = "en"     # Optional ISO 639-1 hint
```

### RPC Methods

#### `stt.status`

Get current STT status.

**Response:**
```json
{
  "enabled": true,
  "provider": "whisper",
  "configured": true
}
```

#### `stt.providers`

List available STT providers.

**Response:**
```json
[
  { "id": "whisper", "name": "OpenAI Whisper", "configured": true }
]
```

#### `stt.transcribe`

Transcribe audio to text.

**Request:**
```json
{
  "audio": "base64-encoded-audio-data",
  "format": "mp3",
  "language": "en",
  "prompt": "Technical discussion about Rust programming"
}
```

**Response:**
```json
{
  "text": "Hello, how are you today?",
  "language": "en",
  "confidence": null,
  "durationSeconds": 2.5,
  "words": [
    { "word": "Hello", "start": 0.0, "end": 0.5 },
    { "word": "how", "start": 0.6, "end": 0.8 },
    { "word": "are", "start": 0.9, "end": 1.0 },
    { "word": "you", "start": 1.1, "end": 1.3 },
    { "word": "today", "start": 1.4, "end": 1.8 }
  ]
}
```

**Parameters:**
- `audio` (required): Base64-encoded audio data
- `format`: Audio format (`mp3`, `opus`, `ogg`, `aac`, `pcm`)
- `language`: ISO 639-1 code to improve accuracy
- `prompt`: Context hint (terminology, topic)

#### `stt.setProvider`

Change the active STT provider.

**Request:**
```json
{ "provider": "whisper" }
```

## Code Structure

### Voice Crate (`crates/voice/`)

```
src/
├── lib.rs           # Crate entry, re-exports
├── config.rs        # VoiceConfig, TtsConfig, SttConfig
├── tts/
│   ├── mod.rs       # TtsProvider trait, AudioFormat, types
│   ├── elevenlabs.rs # ElevenLabs implementation
│   └── openai.rs    # OpenAI TTS implementation
└── stt/
    ├── mod.rs       # SttProvider trait, Transcript types
    └── whisper.rs   # OpenAI Whisper implementation
```

### Key Traits

```rust
/// Text-to-Speech provider trait
#[async_trait]
pub trait TtsProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn is_configured(&self) -> bool;
    async fn voices(&self) -> Result<Vec<Voice>>;
    async fn synthesize(&self, request: SynthesizeRequest) -> Result<AudioOutput>;
}

/// Speech-to-Text provider trait
#[async_trait]
pub trait SttProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn is_configured(&self) -> bool;
    async fn transcribe(&self, request: TranscribeRequest) -> Result<Transcript>;
}
```

### Gateway Integration (`crates/gateway/src/voice.rs`)

- `LiveTtsService`: Wraps TTS providers, implements `TtsService` trait
- `LiveSttService`: Wraps STT providers, implements `SttService` trait
- `NoopSttService`: No-op for when STT is not configured

## Security

- API keys are stored using `secrecy::Secret<String>` to prevent accidental logging
- Debug output redacts all secret values
- Keys can be set via environment variables or config file

## Adding New Providers

### TTS Provider

1. Create `crates/voice/src/tts/newprovider.rs`
2. Implement `TtsProvider` trait
3. Re-export from `crates/voice/src/tts/mod.rs`
4. Add to `LiveTtsService` in gateway

### STT Provider

1. Create `crates/voice/src/stt/newprovider.rs`
2. Implement `SttProvider` trait
3. Re-export from `crates/voice/src/stt/mod.rs`
4. Add to `LiveSttService` in gateway

## Future Enhancements

- **Streaming TTS**: Chunked audio delivery for lower latency
- **VoiceWake**: Wake word detection and continuous listening
- **Web UI**: Audio playback and microphone capture
- **Channel Integration**: Auto-transcribe Telegram voice messages
- **Per-Agent Voices**: Different voices for different agents
