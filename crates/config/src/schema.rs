/// Config schema types (agents, channels, tools, session, gateway, plugins).
/// Corresponds to `src/config/types.ts` and `zod-schema.*.ts` in the TS codebase.
use std::collections::HashMap;

use {
    secrecy::{ExposeSecret, Secret},
    serde::{Deserialize, Serialize},
};

/// Agent identity (name, emoji, creature, vibe).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentIdentity {
    pub name: Option<String>,
    pub emoji: Option<String>,
    pub creature: Option<String>,
    pub vibe: Option<String>,
}

/// IANA timezone (e.g. `"Europe/Paris"`).
///
/// Wraps [`chrono_tz::Tz`] and (de)serialises as a plain string so it stays
/// compatible with the YAML frontmatter in `USER.md`.
#[derive(Debug, Clone)]
pub struct Timezone(pub chrono_tz::Tz);

impl Timezone {
    /// The IANA name, e.g. `"Europe/Paris"`.
    #[must_use]
    pub fn name(&self) -> &str {
        self.0.name()
    }

    /// The inner [`chrono_tz::Tz`] value.
    #[must_use]
    pub fn tz(&self) -> chrono_tz::Tz {
        self.0
    }
}

impl std::fmt::Display for Timezone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.name())
    }
}

impl std::str::FromStr for Timezone {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<chrono_tz::Tz>()
            .map(Self)
            .map_err(|_| format!("unknown IANA timezone: {s}"))
    }
}

impl From<chrono_tz::Tz> for Timezone {
    fn from(tz: chrono_tz::Tz) -> Self {
        Self(tz)
    }
}

impl Serialize for Timezone {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0.name())
    }
}

impl<'de> Deserialize<'de> for Timezone {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse::<chrono_tz::Tz>()
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}

/// Geographic coordinates (WGS 84).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoLocation {
    pub latitude: f64,
    pub longitude: f64,
    /// Human-readable place name from reverse geocoding (e.g. "Noe Valley, San Francisco, CA").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    /// Unix epoch seconds when the location was last updated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
}

impl GeoLocation {
    /// Create a new `GeoLocation` stamped with the current time.
    pub fn now(latitude: f64, longitude: f64, place: Option<String>) -> Self {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        Self {
            latitude,
            longitude,
            place,
            updated_at: Some(ts),
        }
    }
}

impl std::fmt::Display for GeoLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(ref place) = self.place {
            write!(f, "{place}")?;
        } else {
            write!(f, "{},{}", self.latitude, self.longitude)?;
        }
        if let Some(ts) = self.updated_at {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            let age_secs = now.saturating_sub(ts);
            let age_str = if age_secs < 60 {
                "just now".to_string()
            } else if age_secs < 3600 {
                format!("{}m ago", age_secs / 60)
            } else if age_secs < 86400 {
                format!("{}h ago", age_secs / 3600)
            } else {
                format!("{}d ago", age_secs / 86400)
            };
            write!(f, " (updated {age_str})")?;
        }
        Ok(())
    }
}

/// User profile collected during onboarding.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UserProfile {
    pub name: Option<String>,
    pub timezone: Option<Timezone>,
    pub location: Option<GeoLocation>,
}

/// Resolved identity combining agent identity and user profile.
/// Used as the API response for `identity_get` and in the gon data blob.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedIdentity {
    pub name: String,
    pub emoji: Option<String>,
    pub creature: Option<String>,
    pub vibe: Option<String>,
    pub soul: Option<String>,
    pub user_name: Option<String>,
}

impl ResolvedIdentity {
    pub fn from_config(cfg: &MoltisConfig) -> Self {
        Self {
            name: cfg.identity.name.clone().unwrap_or_else(|| "moltis".into()),
            emoji: cfg.identity.emoji.clone(),
            creature: cfg.identity.creature.clone(),
            vibe: cfg.identity.vibe.clone(),
            soul: None,
            user_name: cfg.user.name.clone(),
        }
    }
}

impl Default for ResolvedIdentity {
    fn default() -> Self {
        Self {
            name: "moltis".into(),
            emoji: None,
            creature: None,
            vibe: None,
            soul: None,
            user_name: None,
        }
    }
}

/// Root configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MoltisConfig {
    pub server: ServerConfig,
    pub providers: ProvidersConfig,
    pub chat: ChatConfig,
    pub tools: ToolsConfig,
    pub skills: SkillsConfig,
    pub mcp: McpConfig,
    pub channels: ChannelsConfig,
    pub tls: TlsConfig,
    pub auth: AuthConfig,
    pub metrics: MetricsConfig,
    pub identity: AgentIdentity,
    pub user: UserProfile,
    pub hooks: Option<HooksConfig>,
    pub memory: MemoryEmbeddingConfig,
    pub tailscale: TailscaleConfig,
    pub failover: FailoverConfig,
    pub heartbeat: HeartbeatConfig,
    pub voice: VoiceConfig,
    pub cron: CronConfig,
}

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
    /// Default provider: "elevenlabs", "openai", "google", "piper", "coqui".
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
            enabled: false,
            provider: "elevenlabs".into(),
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
    /// Default provider.
    pub provider: VoiceSttProvider,
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
            enabled: false,
            provider: VoiceSttProvider::Whisper,
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

/// Gateway server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// Address to bind to. Defaults to "127.0.0.1".
    pub bind: String,
    /// Port to listen on. When a new config is created, a random available port
    /// is generated so each installation gets a unique port.
    pub port: u16,
    /// Enable verbose Axum/Tower HTTP request logs (`http_request` spans).
    /// Useful for debugging redirects and request flow.
    pub http_request_logs: bool,
    /// Enable WebSocket request/response logs (`ws:` entries).
    /// Useful for debugging RPC calls from the web UI.
    pub ws_request_logs: bool,
    /// Optional GitHub repository URL used by the update checker.
    ///
    /// When unset, Moltis falls back to the package repository metadata.
    pub update_repository_url: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".into(),
            port: 0, // Will be replaced with a random port when config is created
            http_request_logs: false,
            ws_request_logs: false,
            update_repository_url: None,
        }
    }
}

/// Failover configuration for automatic model/provider failover.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FailoverConfig {
    /// Whether failover is enabled. Defaults to true.
    pub enabled: bool,
    /// Ordered list of fallback model IDs to try when the primary fails.
    /// If empty, the chain is built from all registered models.
    #[serde(default)]
    pub fallback_models: Vec<String>,
}

impl Default for FailoverConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            fallback_models: Vec::new(),
        }
    }
}

/// Heartbeat configuration — periodic health-check agent turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HeartbeatConfig {
    /// Whether the heartbeat is enabled. Defaults to true.
    pub enabled: bool,
    /// Interval between heartbeats (e.g. "30m", "1h"). Defaults to "30m".
    pub every: String,
    /// Provider/model override for heartbeat turns (e.g. "anthropic/claude-sonnet-4-20250514").
    pub model: Option<String>,
    /// Custom prompt override. If empty, the built-in default is used.
    pub prompt: Option<String>,
    /// Max characters for an acknowledgment reply before truncation. Defaults to 300.
    pub ack_max_chars: usize,
    /// Active hours window — heartbeats only run during this window.
    pub active_hours: ActiveHoursConfig,
    /// Whether heartbeat runs inside a sandbox. Defaults to true.
    #[serde(default = "default_true")]
    pub sandbox_enabled: bool,
    /// Override sandbox image for heartbeat. If `None`, uses the default image.
    pub sandbox_image: Option<String>,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            every: "30m".into(),
            model: None,
            prompt: None,
            ack_max_chars: 300,
            active_hours: ActiveHoursConfig::default(),
            sandbox_enabled: true,
            sandbox_image: None,
        }
    }
}

/// Active hours window for heartbeats.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ActiveHoursConfig {
    /// Start time in HH:MM format. Defaults to "08:00".
    pub start: String,
    /// End time in HH:MM format. Defaults to "24:00" (midnight = always on until end of day).
    pub end: String,
    /// IANA timezone (e.g. "Europe/Paris") or "local". Defaults to "local".
    pub timezone: String,
}

impl Default for ActiveHoursConfig {
    fn default() -> Self {
        Self {
            start: "08:00".into(),
            end: "24:00".into(),
            timezone: "local".into(),
        }
    }
}

/// Cron scheduler configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CronConfig {
    /// Maximum number of jobs that can be created within the rate limit window.
    /// Defaults to 10.
    pub rate_limit_max: usize,
    /// Rate limit window in seconds. Defaults to 60 (1 minute).
    pub rate_limit_window_secs: u64,
}

impl Default for CronConfig {
    fn default() -> Self {
        Self {
            rate_limit_max: 10,
            rate_limit_window_secs: 60,
        }
    }
}

/// Tailscale Serve/Funnel configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TailscaleConfig {
    /// Tailscale mode: "off", "serve", or "funnel".
    pub mode: String,
    /// Reset tailscale serve/funnel when the gateway shuts down.
    pub reset_on_exit: bool,
}

impl Default for TailscaleConfig {
    fn default() -> Self {
        Self {
            mode: "off".into(),
            reset_on_exit: true,
        }
    }
}

/// Memory embedding provider configuration.
///
/// Controls which embedding provider the memory system uses.
/// If not configured, the system auto-detects from available providers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryEmbeddingConfig {
    /// Memory backend: "builtin" (default) or "qmd" for QMD sidecar.
    pub backend: Option<String>,
    /// Embedding provider: "local", "ollama", "openai", "custom", or None for auto-detect.
    pub provider: Option<String>,
    /// Base URL for the embedding API (e.g. "http://localhost:11434/v1" for Ollama).
    pub base_url: Option<String>,
    /// Model name (e.g. "nomic-embed-text" for Ollama, "text-embedding-3-small" for OpenAI).
    pub model: Option<String>,
    /// API key (optional for local endpoints like Ollama).
    #[serde(
        default,
        serialize_with = "serialize_option_secret",
        skip_serializing_if = "Option::is_none"
    )]
    pub api_key: Option<Secret<String>>,
    /// Citation mode: "on", "off", or "auto" (default).
    /// When "auto", citations are included when results come from multiple files.
    pub citations: Option<String>,
    /// Enable LLM reranking for hybrid search results.
    #[serde(default)]
    pub llm_reranking: bool,
    /// Enable session export to memory for cross-run recall.
    #[serde(default)]
    pub session_export: bool,
    /// QMD-specific configuration (only used when backend = "qmd").
    #[serde(default)]
    pub qmd: QmdConfig,
}

/// QMD backend configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct QmdConfig {
    /// Path to the qmd binary (default: "qmd").
    pub command: Option<String>,
    /// Named collections with paths and glob patterns.
    #[serde(default)]
    pub collections: HashMap<String, QmdCollection>,
    /// Maximum results to retrieve.
    pub max_results: Option<usize>,
    /// Search timeout in milliseconds.
    pub timeout_ms: Option<u64>,
}

/// A QMD collection configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct QmdCollection {
    /// Paths to include in this collection.
    #[serde(default)]
    pub paths: Vec<String>,
    /// Glob patterns to filter files.
    #[serde(default)]
    pub globs: Vec<String>,
}

/// Hooks configuration section (shell hooks defined in config file).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HooksConfig {
    #[serde(default)]
    pub hooks: Vec<ShellHookConfigEntry>,
}

/// A single shell hook defined in the config file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellHookConfigEntry {
    pub name: String,
    pub command: String,
    pub events: Vec<String>,
    #[serde(default = "default_hook_timeout")]
    pub timeout: u64,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

fn default_hook_timeout() -> u64 {
    10
}

/// Authentication configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AuthConfig {
    /// When true, authentication is explicitly disabled (no login required).
    pub disabled: bool,
}

/// Metrics and observability configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MetricsConfig {
    /// Whether metrics collection is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Whether to expose the `/metrics` Prometheus endpoint.
    #[serde(default = "default_true")]
    pub prometheus_endpoint: bool,
    /// Additional labels to add to all metrics.
    #[serde(default)]
    pub labels: HashMap<String, String>,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            prometheus_endpoint: true,
            labels: HashMap::new(),
        }
    }
}

impl MoltisConfig {
    /// Returns `true` when both the agent name and user name have been set
    /// (i.e. the onboarding wizard has been completed).
    pub fn is_onboarded(&self) -> bool {
        self.identity.name.is_some() && self.user.name.is_some()
    }
}

/// Skills configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SkillsConfig {
    /// Whether the skills system is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Extra directories to search for skills.
    #[serde(default)]
    pub search_paths: Vec<String>,
    /// Skills to always load (by name) without explicit activation.
    #[serde(default)]
    pub auto_load: Vec<String>,
}

fn default_true() -> bool {
    true
}

/// MCP (Model Context Protocol) server configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct McpConfig {
    /// Configured MCP servers, keyed by server name.
    #[serde(default)]
    pub servers: HashMap<String, McpServerEntry>,
}

/// Configuration for a single MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerEntry {
    /// Command to spawn the server process (stdio transport).
    #[serde(default)]
    pub command: String,
    /// Arguments to the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables to set for the process.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Whether this server is enabled. Defaults to true.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Transport type: "stdio" (default) or "sse".
    #[serde(default)]
    pub transport: String,
    /// URL for SSE transport. Required when `transport` is "sse".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Channel configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelsConfig {
    /// Telegram bot accounts, keyed by account ID.
    #[serde(default)]
    pub telegram: HashMap<String, serde_json::Value>,
}

/// TLS configuration for the gateway HTTPS server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TlsConfig {
    /// Enable HTTPS with auto-generated certificates. Defaults to true.
    pub enabled: bool,
    /// Auto-generate a local CA and server certificate on first run.
    pub auto_generate: bool,
    /// Path to a custom server certificate (PEM). Overrides auto-generation.
    pub cert_path: Option<String>,
    /// Path to a custom server private key (PEM). Overrides auto-generation.
    pub key_path: Option<String>,
    /// Path to the CA certificate (PEM) used for trust instructions.
    pub ca_cert_path: Option<String>,
    /// Port for the plain-HTTP redirect/CA-download server.
    /// Defaults to the gateway port + 1 when not set.
    pub http_redirect_port: Option<u16>,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_generate: true,
            cert_path: None,
            key_path: None,
            ca_cert_path: None,
            http_redirect_port: None,
        }
    }
}

/// Chat configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChatConfig {
    /// How to handle messages that arrive while an agent run is active.
    pub message_queue_mode: MessageQueueMode,
    /// Preferred model IDs to show first in selectors (full or raw model IDs).
    pub priority_models: Vec<String>,
}

/// Behaviour when `chat.send()` is called during an active run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageQueueMode {
    /// Queue each message; replay them one-by-one after the current run.
    #[default]
    Followup,
    /// Buffer messages; concatenate and process as a single message after the current run.
    Collect,
}

/// Tools configuration (exec, sandbox, policy, web, browser).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolsConfig {
    pub exec: ExecConfig,
    pub policy: ToolPolicyConfig,
    pub web: WebConfig,
    pub browser: BrowserConfig,
    /// Maximum wall-clock seconds for an agent run (0 = no timeout). Default 600.
    #[serde(default = "default_agent_timeout_secs")]
    pub agent_timeout_secs: u64,
    /// Maximum bytes for a single tool result before truncation. Default 50KB.
    #[serde(default = "default_max_tool_result_bytes")]
    pub max_tool_result_bytes: usize,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            exec: ExecConfig::default(),
            policy: ToolPolicyConfig::default(),
            web: WebConfig::default(),
            browser: BrowserConfig::default(),
            agent_timeout_secs: default_agent_timeout_secs(),
            max_tool_result_bytes: default_max_tool_result_bytes(),
        }
    }
}

fn default_agent_timeout_secs() -> u64 {
    600
}

fn default_max_tool_result_bytes() -> usize {
    50_000
}

/// Web tools configuration (search, fetch).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WebConfig {
    pub search: WebSearchConfig,
    pub fetch: WebFetchConfig,
}

/// Search provider selection.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SearchProvider {
    #[default]
    Brave,
    Perplexity,
}

/// Web search tool configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebSearchConfig {
    pub enabled: bool,
    /// Search provider.
    pub provider: SearchProvider,
    /// Brave Search API key (overrides `BRAVE_API_KEY` env var).
    #[serde(
        default,
        serialize_with = "serialize_option_secret",
        skip_serializing_if = "Option::is_none"
    )]
    pub api_key: Option<Secret<String>>,
    /// Maximum number of results to return (1-10).
    pub max_results: u8,
    /// HTTP request timeout in seconds.
    pub timeout_seconds: u64,
    /// In-memory cache TTL in minutes (0 to disable).
    pub cache_ttl_minutes: u64,
    /// Perplexity-specific settings.
    pub perplexity: PerplexityConfig,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: SearchProvider::default(),
            api_key: None,
            max_results: 5,
            timeout_seconds: 30,
            cache_ttl_minutes: 15,
            perplexity: PerplexityConfig::default(),
        }
    }
}

/// Perplexity search provider configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PerplexityConfig {
    /// API key (overrides `PERPLEXITY_API_KEY` / `OPENROUTER_API_KEY` env vars).
    #[serde(
        default,
        serialize_with = "serialize_option_secret",
        skip_serializing_if = "Option::is_none"
    )]
    pub api_key: Option<Secret<String>>,
    /// Base URL override. Auto-detected from key prefix if empty.
    pub base_url: Option<String>,
    /// Model to use.
    pub model: Option<String>,
}

/// Web fetch tool configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebFetchConfig {
    pub enabled: bool,
    /// Maximum characters to return from fetched content.
    pub max_chars: usize,
    /// HTTP request timeout in seconds.
    pub timeout_seconds: u64,
    /// In-memory cache TTL in minutes (0 to disable).
    pub cache_ttl_minutes: u64,
    /// Maximum number of HTTP redirects to follow.
    pub max_redirects: u8,
    /// Use readability extraction for HTML pages.
    pub readability: bool,
}

impl Default for WebFetchConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_chars: 50_000,
            timeout_seconds: 30,
            cache_ttl_minutes: 15,
            max_redirects: 3,
            readability: true,
        }
    }
}

/// Browser automation configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BrowserConfig {
    /// Whether browser support is enabled.
    pub enabled: bool,
    /// Path to Chrome/Chromium binary (auto-detected if not set).
    pub chrome_path: Option<String>,
    /// Whether to run in headless mode.
    pub headless: bool,
    /// Default viewport width.
    pub viewport_width: u32,
    /// Default viewport height.
    pub viewport_height: u32,
    /// Device scale factor for HiDPI/Retina displays.
    /// 1.0 = standard, 2.0 = Retina/HiDPI, 3.0 = 3x scaling.
    pub device_scale_factor: f64,
    /// Maximum concurrent browser instances (0 = unlimited, limited by memory).
    pub max_instances: usize,
    /// System memory usage threshold (0-100) above which new instances are blocked.
    /// Default is 90 (block new instances when memory > 90% used).
    pub memory_limit_percent: u8,
    /// Instance idle timeout in seconds before closing.
    pub idle_timeout_secs: u64,
    /// Default navigation timeout in milliseconds.
    pub navigation_timeout_ms: u64,
    /// User agent string (uses default if not set).
    pub user_agent: Option<String>,
    /// Additional Chrome arguments.
    #[serde(default)]
    pub chrome_args: Vec<String>,
    /// Docker image to use for sandboxed browser.
    /// Default: "browserless/chrome" - a purpose-built headless Chrome container.
    /// Sandbox mode is controlled per-session via the request, not globally.
    #[serde(default = "default_sandbox_image")]
    pub sandbox_image: String,
    /// Allowed domains for navigation. Empty list means all domains allowed.
    /// When set, the browser will refuse to navigate to non-matching domains.
    /// Supports wildcards: "*.example.com" matches subdomains.
    #[serde(default)]
    pub allowed_domains: Vec<String>,
}

fn default_sandbox_image() -> String {
    "browserless/chrome".to_string()
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            chrome_path: None,
            headless: true,
            viewport_width: 2560,
            viewport_height: 1440,
            device_scale_factor: 2.0,
            max_instances: 0, // 0 = unlimited, limited by memory
            memory_limit_percent: 90,
            idle_timeout_secs: 300,
            navigation_timeout_ms: 30000,
            user_agent: None,
            chrome_args: Vec::new(),
            sandbox_image: default_sandbox_image(),
            allowed_domains: Vec::new(),
        }
    }
}

/// Exec tool configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ExecConfig {
    pub default_timeout_secs: u64,
    pub max_output_bytes: usize,
    pub approval_mode: String,
    pub security_level: String,
    pub allowlist: Vec<String>,
    pub sandbox: SandboxConfig,
}

impl Default for ExecConfig {
    fn default() -> Self {
        Self {
            default_timeout_secs: 30,
            max_output_bytes: 200 * 1024,
            approval_mode: "on-miss".into(),
            security_level: "allowlist".into(),
            allowlist: Vec::new(),
            sandbox: SandboxConfig::default(),
        }
    }
}

/// Resource limits for sandboxed execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ResourceLimitsConfig {
    /// Memory limit (e.g. "512M", "1G").
    pub memory_limit: Option<String>,
    /// CPU quota as a fraction (e.g. 0.5 = half a core, 2.0 = two cores).
    pub cpu_quota: Option<f64>,
    /// Maximum number of PIDs.
    pub pids_max: Option<u32>,
}

/// Sandbox configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SandboxConfig {
    pub mode: String,
    pub scope: String,
    pub workspace_mount: String,
    pub image: Option<String>,
    pub container_prefix: Option<String>,
    pub no_network: bool,
    /// Backend: "auto" (default), "docker", or "apple-container".
    /// "auto" prefers Apple Container on macOS when available, falls back to Docker.
    pub backend: String,
    pub resource_limits: ResourceLimitsConfig,
    /// Packages to install via `apt-get` in the sandbox image.
    /// Set to an empty list to skip provisioning.
    #[serde(default = "default_sandbox_packages")]
    pub packages: Vec<String>,
}

/// Default packages installed in sandbox containers.
/// Inspired by GitHub Actions runner images — covers commonly needed
/// CLI tools, language runtimes, and utilities for LLM-driven tasks.
fn default_sandbox_packages() -> Vec<String> {
    [
        // Networking & HTTP
        "curl",
        "wget",
        "ca-certificates",
        "dnsutils",
        "netcat-openbsd",
        "openssh-client",
        "iproute2",
        "net-tools",
        // Language runtimes
        "python3",
        "python3-dev",
        "python3-pip",
        "python3-venv",
        "python-is-python3",
        "nodejs",
        "npm",
        "ruby",
        "ruby-dev",
        // Build toolchain & native deps
        "build-essential",
        "clang",
        "libclang-dev",
        "llvm-dev",
        "pkg-config",
        "libssl-dev",
        "libsqlite3-dev",
        "libyaml-dev",
        "liblzma-dev",
        "autoconf",
        "automake",
        "libtool",
        "bison",
        "flex",
        "dpkg-dev",
        "fakeroot",
        // Compression & archiving
        "zip",
        "unzip",
        "bzip2",
        "xz-utils",
        "p7zip-full",
        "tar",
        "zstd",
        "lz4",
        "pigz",
        // Common CLI utilities (mirrors GitHub runner image)
        "git",
        "gnupg2",
        "jq",
        "rsync",
        "file",
        "tree",
        "sqlite3",
        "sudo",
        "locales",
        "tzdata",
        "shellcheck",
        "patchelf",
        // Text processing & search
        "ripgrep",
        "fd-find",
        "yq",
        // Terminal multiplexer (useful for capturing ncurses apps)
        "tmux",
        // Browser automation (for browser tool)
        "chromium",
        "libxss1",
        "libnss3",
        "libnspr4",
        "libasound2t64",
        "libatk1.0-0t64",
        "libatk-bridge2.0-0t64",
        "libcups2t64",
        "libdrm2",
        "libgbm1",
        "libgtk-3-0t64",
        "libxcomposite1",
        "libxdamage1",
        "libxfixes3",
        "libxrandr2",
        "libxkbcommon0",
        "fonts-liberation",
        // Image processing (headless)
        "imagemagick",
        "graphicsmagick",
        "libvips-tools",
        "pngquant",
        "optipng",
        "jpegoptim",
        "webp",
        "libimage-exiftool-perl",
        "libheif-dev",
        // Audio / video / media
        "ffmpeg",
        "sox",
        "lame",
        "flac",
        "vorbis-tools",
        "opus-tools",
        "mediainfo",
        // Document & office conversion
        "pandoc",
        "poppler-utils",
        "ghostscript",
        "texlive-latex-base",
        "texlive-latex-extra",
        "texlive-fonts-recommended",
        "antiword",
        "catdoc",
        "unrtf",
        "libreoffice-core",
        "libreoffice-writer",
        // Data processing & conversion
        "csvtool",
        "xmlstarlet",
        "html2text",
        "dos2unix",
        "miller",
        "datamash",
        // GIS / OpenStreetMap / map generation
        "gdal-bin",
        "mapnik-utils",
        "osm2pgsql",
        "osmium-tool",
        "osmctools",
        "python3-mapnik",
        "libgdal-dev",
        // CalDAV / CardDAV
        "vdirsyncer",
        "khal",
        "python3-caldav",
        // Email (IMAP sync, indexing, CLI clients)
        "isync",
        "offlineimap3",
        "notmuch",
        "notmuch-mutt",
        "aerc",
        "mutt",
        "neomutt",
        // Newsgroups (NNTP)
        "tin",
        "slrn",
        // Messaging APIs
        "python3-discord",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect()
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            mode: "all".into(),
            scope: "session".into(),
            workspace_mount: "ro".into(),
            image: None,
            container_prefix: None,
            no_network: true,
            backend: "auto".into(),
            resource_limits: ResourceLimitsConfig::default(),
            packages: default_sandbox_packages(),
        }
    }
}

/// Tool policy configuration (allow/deny lists).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolPolicyConfig {
    pub allow: Vec<String>,
    pub deny: Vec<String>,
    pub profile: Option<String>,
}

/// OAuth provider configuration (e.g. openai-codex).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthProviderConfig {
    pub client_id: String,
    pub auth_url: String,
    pub token_url: String,
    pub redirect_uri: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub callback_port: u16,
}

/// LLM provider configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProvidersConfig {
    /// Optional allowlist of providers offered in web UI pickers (onboarding and
    /// "add provider" modal). Empty means show all known providers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub offered: Vec<String>,

    /// Provider-specific settings keyed by provider name.
    /// Known keys: "anthropic", "openai", "gemini", "groq", "xai", "deepseek"
    #[serde(flatten)]
    pub providers: HashMap<String, ProviderEntry>,

    /// Additional local model IDs to register (from local-llm.json).
    /// This is populated at runtime by the gateway and not persisted.
    #[serde(skip)]
    pub local_models: Vec<String>,
}

/// Configuration for a single LLM provider.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderEntry {
    /// Whether this provider is enabled. Defaults to true.
    pub enabled: bool,

    /// Override the API key (optional; env var still takes precedence if set).
    #[serde(
        default,
        serialize_with = "serialize_option_secret",
        skip_serializing_if = "Option::is_none"
    )]
    pub api_key: Option<Secret<String>>,

    /// Override the base URL.
    pub base_url: Option<String>,

    /// Default model ID for this provider.
    pub model: Option<String>,

    /// Optional alias for this provider instance.
    ///
    /// When set, this alias is used in metrics labels instead of the provider name.
    /// Useful when configuring multiple instances of the same provider type
    /// (e.g., "anthropic-work", "anthropic-personal").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

impl std::fmt::Debug for ProviderEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderEntry")
            .field("enabled", &self.enabled)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("alias", &self.alias)
            .finish()
    }
}

impl Default for ProviderEntry {
    fn default() -> Self {
        Self {
            enabled: true,
            api_key: None,
            base_url: None,
            model: None,
            alias: None,
        }
    }
}

// ── Serde helpers for Secret<String> ────────────────────────────────────────

fn serialize_option_secret<S: serde::Serializer>(
    secret: &Option<Secret<String>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match secret {
        Some(s) => serializer.serialize_some(s.expose_secret()),
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

impl ProvidersConfig {
    /// Check if a provider is enabled (defaults to true if not configured).
    pub fn is_enabled(&self, name: &str) -> bool {
        self.providers.get(name).is_none_or(|e| e.enabled)
    }

    /// Get the configured entry for a provider, if any.
    pub fn get(&self, name: &str) -> Option<&ProviderEntry> {
        self.providers.get(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geolocation_display_with_place() {
        let loc = GeoLocation {
            latitude: 37.759,
            longitude: -122.433,
            place: Some("Noe Valley, San Francisco, CA".to_string()),
            updated_at: None,
        };
        assert_eq!(loc.to_string(), "Noe Valley, San Francisco, CA");
    }

    #[test]
    fn geolocation_display_without_place() {
        let loc = GeoLocation {
            latitude: 37.759,
            longitude: -122.433,
            place: None,
            updated_at: None,
        };
        assert_eq!(loc.to_string(), "37.759,-122.433");
    }

    #[test]
    fn geolocation_serde_backward_compat() {
        // Old JSON without `place` field should deserialize fine.
        let json = r#"{"latitude":48.8566,"longitude":2.3522,"updated_at":1700000000}"#;
        let loc: GeoLocation = serde_json::from_str(json).unwrap();
        assert!((loc.latitude - 48.8566).abs() < 1e-6);
        assert!(loc.place.is_none());
    }

    #[test]
    fn geolocation_serde_with_place() {
        let loc = GeoLocation {
            latitude: 48.8566,
            longitude: 2.3522,
            place: Some("Paris, France".to_string()),
            updated_at: Some(1_700_000_000),
        };
        let json = serde_json::to_string(&loc).unwrap();
        let parsed: GeoLocation = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.place.as_deref(), Some("Paris, France"));
    }

    #[test]
    fn geolocation_now_stores_place() {
        let loc = GeoLocation::now(37.0, -122.0, Some("San Francisco".to_string()));
        assert_eq!(loc.place.as_deref(), Some("San Francisco"));
        assert!(loc.updated_at.is_some());
    }
}
