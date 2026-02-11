# Voice Services

Moltis provides text-to-speech (TTS) and speech-to-text (STT) capabilities
through the `moltis-voice` crate and gateway integration.

## Feature Flag

Voice services are behind the `voice` cargo feature, enabled by default:

```toml
# Cargo.toml (gateway crate)
[features]
default = ["voice", ...]
voice = ["dep:moltis-voice"]
```

To disable voice features at compile time:
```bash
cargo build --no-default-features --features "file-watcher,tailscale,tls,web-ui"
```

When disabled:
- TTS/STT RPC methods are not registered
- Voice settings section is hidden in the UI
- Microphone button is hidden in the chat interface
- `voice_enabled: false` is set in the gon data

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      Voice Crate                            │
│                   (crates/voice/)                           │
├─────────────────────────────────────────────────────────────┤
│  TtsProvider trait         │  SttProvider trait             │
│  ├─ ElevenLabsTts          │  ├─ WhisperStt (OpenAI)        │
│  ├─ OpenAiTts              │  ├─ GroqStt (Groq)             │
│  ├─ GoogleTts              │  ├─ DeepgramStt                │
│  ├─ PiperTts (local)       │  ├─ GoogleStt                  │
│  └─ CoquiTts (local)       │  ├─ MistralStt                 │
│                            │  ├─ VoxtralLocalStt (local)    │
│                            │  ├─ WhisperCliStt (local)      │
│                            │  └─ SherpaOnnxStt (local)      │
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
│  tts.status, tts.providers, tts.enable, tts.disable,        │
│  tts.convert, tts.setProvider                               │
│  stt.status, stt.providers, stt.transcribe, stt.setProvider │
└─────────────────────────────────────────────────────────────┘
```

## Text-to-Speech (TTS)

### Supported Providers

Moltis supports multiple TTS providers across cloud and local backends.

| Category | Notes |
|----------|-------|
| Cloud TTS providers | Hosted neural voices with low-latency streaming |
| Local TTS providers | Offline/on-device synthesis for privacy-sensitive workflows |

*More voice providers are coming soon.*

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
providers = []           # Optional UI allowlist, empty = show all TTS providers
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

[voice.tts.google]
api_key = "..."  # Google Cloud API key
voice = "en-US-Neural2-D"  # See Google Cloud TTS voices
language_code = "en-US"
speaking_rate = 1.0

# Local providers - no API key required

[voice.tts.piper]
# binary_path = "/usr/local/bin/piper"  # optional, searches PATH
model_path = "~/.moltis/models/en_US-lessac-medium.onnx"  # required
# config_path = "~/.moltis/models/en_US-lessac-medium.onnx.json"  # optional
# speaker_id = 0  # for multi-speaker models
# length_scale = 1.0  # speaking rate (lower = faster)

[voice.tts.coqui]
endpoint = "http://localhost:5002"  # Coqui TTS server
# model = "tts_models/en/ljspeech/tacotron2-DDC"  # optional
```

### Local TTS Provider Setup

#### Piper TTS

Piper is a fast, local neural text-to-speech system that runs entirely offline.

1. Install Piper:
   ```bash
   # Via pip
   pip install piper-tts

   # Or download pre-built binaries from:
   # https://github.com/rhasspy/piper/releases
   ```

2. Download a voice model from [Piper Voices](https://github.com/rhasspy/piper#voices):
   ```bash
   mkdir -p ~/.moltis/models
   curl -L -o ~/.moltis/models/en_US-lessac-medium.onnx \
     https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx
   ```

3. Configure in `moltis.toml`:
   ```toml
   [voice.tts]
   provider = "piper"

   [voice.tts.piper]
   model_path = "~/.moltis/models/en_US-lessac-medium.onnx"
   ```

#### Coqui TTS

Coqui TTS is a high-quality neural TTS with voice cloning capabilities.

1. Install and start the server:
   ```bash
   # Via pip
   pip install TTS
   tts-server --model_name tts_models/en/ljspeech/tacotron2-DDC

   # Or via Docker
   docker run -p 5002:5002 ghcr.io/coqui-ai/tts
   ```

2. Configure in `moltis.toml`:
   ```toml
   [voice.tts]
   provider = "coqui"

   [voice.tts.coqui]
   endpoint = "http://localhost:5002"
   ```

Browse available models at [Coqui TTS Models](https://github.com/coqui-ai/TTS#available-models).

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

Moltis supports multiple STT providers across cloud and local backends.

| Category | Notes |
|----------|-------|
| Cloud STT providers | Managed transcription APIs with language/model options |
| Local STT providers | Offline transcription through local binaries or services |

*More voice providers are coming soon.*

### Configuration

```toml
[voice.stt]
enabled = true
provider = "whisper"  # set to any configured STT provider id
providers = []        # Optional UI allowlist, empty = show all STT providers

# Cloud providers - API key required
[voice.stt.whisper]
api_key = "sk-..."  # Uses OPENAI_API_KEY if not set
model = "whisper-1"
language = "en"     # Optional ISO 639-1 hint

[voice.stt.groq]
api_key = "gsk_..."
model = "whisper-large-v3-turbo"  # default
language = "en"

[voice.stt.deepgram]
api_key = "..."
model = "nova-3"  # default
language = "en"
smart_format = true

[voice.stt.google]
api_key = "..."
language = "en-US"
# model = "latest_long"  # optional

[voice.stt.mistral]
api_key = "..."
model = "voxtral-mini-latest"  # default
language = "en"

# Local providers - no API key, requires server or binary

# Voxtral local via vLLM server
[voice.stt.voxtral_local]
# endpoint = "http://localhost:8000"  # default vLLM endpoint
# model = "mistralai/Voxtral-Mini-3B-2507"  # optional, server default
# language = "en"  # optional

[voice.stt.whisper_cli]
# binary_path = "/usr/local/bin/whisper-cli"  # optional, searches PATH
model_path = "~/.moltis/models/ggml-base.en.bin"  # required
language = "en"

[voice.stt.sherpa_onnx]
# binary_path = "/usr/local/bin/sherpa-onnx-offline"  # optional
model_dir = "~/.moltis/models/sherpa-onnx-whisper-tiny.en"  # required
language = "en"
```

### Local Provider Setup

#### Voxtral via vLLM

Voxtral is an open-weights model from Mistral AI that can run locally using vLLM.
It supports 13 languages with fast transcription.

**Requirements:**
- Python 3.10+
- CUDA-capable GPU with ~9.5GB VRAM (or CPU with more memory)
- vLLM with audio support

**Setup:**

1. Install vLLM with audio support:
   ```bash
   pip install "vllm[audio]"
   ```

2. Start the vLLM server:
   ```bash
   vllm serve mistralai/Voxtral-Mini-3B-2507 \
     --tokenizer_mode mistral \
     --config_format mistral \
     --load_format mistral
   ```

   The server exposes an OpenAI-compatible endpoint at `http://localhost:8000`.

3. Configure in `moltis.toml`:
   ```toml
   [voice.stt]
   provider = "voxtral-local"

   [voice.stt.voxtral_local]
   # Default endpoint works if vLLM is running locally
   # endpoint = "http://localhost:8000"
   ```

**Supported Languages:**
English, French, German, Spanish, Portuguese, Italian, Dutch, Polish, Swedish,
Norwegian, Danish, Finnish, Arabic

**Note:** Unlike the embedded local providers (whisper.cpp, sherpa-onnx), this
requires running vLLM as a separate server process. The model is downloaded
automatically on first vLLM startup.

#### whisper.cpp

1. Install the binary:
   ```bash
   # macOS
   brew install whisper-cpp

   # From source: https://github.com/ggerganov/whisper.cpp
   ```

2. Download a model from [Hugging Face](https://huggingface.co/ggerganov/whisper.cpp):
   ```bash
   mkdir -p ~/.moltis/models
   curl -L -o ~/.moltis/models/ggml-base.en.bin \
     https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin
   ```

3. Configure in `moltis.toml`:
   ```toml
   [voice.stt]
   provider = "whisper-cli"

   [voice.stt.whisper_cli]
   model_path = "~/.moltis/models/ggml-base.en.bin"
   ```

#### sherpa-onnx

1. Install following the [official docs](https://k2-fsa.github.io/sherpa/onnx/install.html)

2. Download a model from the [model list](https://k2-fsa.github.io/sherpa/onnx/pretrained_models/index.html)

3. Configure in `moltis.toml`:
   ```toml
   [voice.stt]
   provider = "sherpa-onnx"

   [voice.stt.sherpa_onnx]
   model_dir = "~/.moltis/models/sherpa-onnx-whisper-tiny.en"
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
  { "id": "whisper", "name": "OpenAI Whisper", "configured": true },
  { "id": "groq", "name": "Groq", "configured": false },
  { "id": "deepgram", "name": "Deepgram", "configured": false },
  { "id": "google", "name": "Google Cloud", "configured": false },
  { "id": "mistral", "name": "Mistral AI", "configured": false },
  { "id": "voxtral-local", "name": "Voxtral (Local)", "configured": false },
  { "id": "whisper-cli", "name": "whisper.cpp", "configured": false },
  { "id": "sherpa-onnx", "name": "sherpa-onnx", "configured": false }
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
{ "provider": "groq" }
```

Valid provider IDs: `whisper`, `groq`, `deepgram`, `google`, `mistral`, `voxtral-local`, `whisper-cli`, `sherpa-onnx`

## Code Structure

### Voice Crate (`crates/voice/`)

```
src/
├── lib.rs           # Crate entry, re-exports
├── config.rs        # VoiceConfig, TtsConfig, SttConfig
├── tts/
│   ├── mod.rs       # TtsProvider trait, AudioFormat, types
│   ├── elevenlabs.rs # ElevenLabs implementation
│   ├── openai.rs    # OpenAI TTS implementation
│   ├── google.rs    # Google Cloud TTS implementation
│   ├── piper.rs     # Piper local TTS implementation
│   └── coqui.rs     # Coqui TTS server implementation
└── stt/
    ├── mod.rs          # SttProvider trait, Transcript types
    ├── whisper.rs      # OpenAI Whisper implementation
    ├── groq.rs         # Groq Whisper implementation
    ├── deepgram.rs     # Deepgram implementation
    ├── google.rs       # Google Cloud Speech-to-Text
    ├── mistral.rs      # Mistral AI Voxtral cloud implementation
    ├── voxtral_local.rs # Voxtral via local vLLM server
    ├── cli_utils.rs    # Shared utilities for CLI providers
    ├── whisper_cli.rs  # whisper.cpp CLI wrapper
    └── sherpa_onnx.rs  # sherpa-onnx CLI wrapper
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

## Web UI Integration

The voice feature integrates with the web UI:

- **Microphone button**: Record voice input in the chat interface
- **Settings page**: Configure and enable/disable voice providers
- **Auto-detection**: API keys are detected from environment variables and LLM provider configs
- **Toggle switches**: Enable/disable providers without removing configuration
- **Setup instructions**: Step-by-step guides for local provider installation

## Future Enhancements

- **Streaming TTS**: Chunked audio delivery for lower latency
- **VoiceWake**: Wake word detection and continuous listening
- **Audio playback**: Play TTS responses directly in the chat
- **Channel Integration**: Auto-transcribe Telegram voice messages
- **Per-Agent Voices**: Different voices for different agents
