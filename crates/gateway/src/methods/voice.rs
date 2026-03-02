use std::time::Duration;

use moltis_config::VoiceSttProvider;

/// Check if Python 3.10+ is available.
pub(super) async fn check_python_version() -> serde_json::Value {
    // Try python3 first, then python
    for cmd in &["python3", "python"] {
        if let Ok(output) = tokio::process::Command::new(cmd)
            .arg("--version")
            .output()
            .await
            && output.status.success()
        {
            let version_str = String::from_utf8_lossy(&output.stdout);
            // Parse "Python 3.11.0" format
            if let Some(version) = version_str.strip_prefix("Python ") {
                let version = version.trim();
                // Check if version is 3.10+
                let parts: Vec<&str> = version.split('.').collect();
                if parts.len() >= 2
                    && let (Ok(major), Ok(minor)) =
                        (parts[0].parse::<u32>(), parts[1].parse::<u32>())
                {
                    let sufficient = major > 3 || (major == 3 && minor >= 10);
                    return serde_json::json!({
                        "available": true,
                        "version": version,
                        "sufficient": sufficient,
                    });
                }
                return serde_json::json!({
                    "available": true,
                    "version": version,
                    "sufficient": false,
                });
            }
        }
    }
    serde_json::json!({
        "available": false,
        "version": null,
        "sufficient": false,
    })
}

/// Check CUDA availability via nvidia-smi.
pub(super) async fn check_cuda_availability() -> serde_json::Value {
    // Check if nvidia-smi is available
    if let Ok(output) = tokio::process::Command::new("nvidia-smi")
        .arg("--query-gpu=name,memory.total")
        .arg("--format=csv,noheader,nounits")
        .output()
        .await
        && output.status.success()
    {
        let info = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = info.trim().lines().collect();
        if let Some(first_gpu) = lines.first() {
            let parts: Vec<&str> = first_gpu.split(", ").collect();
            if parts.len() >= 2 {
                let gpu_name = parts[0].trim();
                let memory_mb: u64 = parts[1].trim().parse().unwrap_or(0);
                // vLLM needs ~9.5GB, recommend 10GB minimum
                let sufficient = memory_mb >= 10000;
                return serde_json::json!({
                    "available": true,
                    "gpu_name": gpu_name,
                    "memory_mb": memory_mb,
                    "sufficient": sufficient,
                });
            }
        }
        return serde_json::json!({
            "available": true,
            "gpu_name": null,
            "memory_mb": null,
            "sufficient": false,
        });
    }
    serde_json::json!({
        "available": false,
        "gpu_name": null,
        "memory_mb": null,
        "sufficient": false,
    })
}

/// Check if the system meets Voxtral Local requirements.
pub(super) fn check_voxtral_compatibility(
    os: &str,
    arch: &str,
    python: &serde_json::Value,
    cuda: &serde_json::Value,
) -> (bool, Vec<String>) {
    let mut reasons = Vec::new();

    // vLLM primarily supports Linux
    let os_ok = os == "linux";
    if !os_ok {
        if os == "macos" {
            reasons.push("vLLM has limited macOS support. Linux is recommended.".into());
        } else if os == "windows" {
            reasons.push("vLLM requires WSL2 on Windows.".into());
        }
    }

    // Architecture check
    let arch_ok = arch == "x86_64";
    if !arch_ok && arch == "aarch64" {
        reasons.push("ARM64 has limited CUDA/vLLM support.".into());
    }

    // Python check
    let python_ok = python["sufficient"].as_bool().unwrap_or(false);
    if !python["available"].as_bool().unwrap_or(false) {
        reasons.push("Python is not installed. Install Python 3.10+.".into());
    } else if !python_ok {
        let ver = python["version"].as_str().unwrap_or("unknown");
        reasons.push(format!("Python {} is too old. Python 3.10+ required.", ver));
    }

    // CUDA check
    let cuda_ok = cuda["sufficient"].as_bool().unwrap_or(false);
    if !cuda["available"].as_bool().unwrap_or(false) {
        reasons.push("No NVIDIA GPU detected. CUDA GPU with 10GB+ VRAM required.".into());
    } else if !cuda_ok {
        let mem = cuda["memory_mb"].as_u64().unwrap_or(0);
        reasons.push(format!(
            "GPU has {}MB VRAM. 10GB+ recommended for Voxtral.",
            mem
        ));
    }

    // Overall compatibility
    let compatible = os_ok && arch_ok && python_ok && cuda_ok;

    (compatible, reasons)
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum VoiceProviderId {
    Elevenlabs,
    OpenaiTts,
    GoogleTts,
    Piper,
    Coqui,
    Whisper,
    Groq,
    Deepgram,
    Google,
    Mistral,
    ElevenlabsStt,
    VoxtralLocal,
    WhisperCli,
    SherpaOnnx,
}

/// Static UI metadata for a voice provider (description, key hints, URLs).
struct VoiceProviderMeta {
    description: &'static str,
    key_placeholder: Option<&'static str>,
    key_url: Option<&'static str>,
    key_url_label: Option<&'static str>,
    hint: Option<&'static str>,
}

impl VoiceProviderId {
    fn meta(self) -> VoiceProviderMeta {
        match self {
            // TTS Cloud
            Self::Elevenlabs => VoiceProviderMeta {
                description: "Lowest latency (~75ms), natural voices. Same key enables Scribe STT",
                key_placeholder: Some("API key"),
                key_url: Some("https://elevenlabs.io/app/settings/api-keys"),
                key_url_label: Some("elevenlabs.io"),
                hint: Some("This API key also enables ElevenLabs Scribe for speech-to-text."),
            },
            Self::OpenaiTts => VoiceProviderMeta {
                description: "Good quality, shares API key with Whisper STT",
                key_placeholder: Some("sk-..."),
                key_url: Some("https://platform.openai.com/api-keys"),
                key_url_label: Some("platform.openai.com/api-keys"),
                hint: None,
            },
            Self::GoogleTts => VoiceProviderMeta {
                description: "220+ voices, 40+ languages, WaveNet and Neural2 voices",
                key_placeholder: Some("API key"),
                key_url: Some("https://console.cloud.google.com/apis/credentials"),
                key_url_label: Some("console.cloud.google.com"),
                hint: None,
            },
            Self::Piper => VoiceProviderMeta {
                description: "Fast local TTS, commonly used in Home Assistant",
                key_placeholder: None,
                key_url: None,
                key_url_label: None,
                hint: None,
            },
            Self::Coqui => VoiceProviderMeta {
                description: "Open-source deep learning TTS with many voice models",
                key_placeholder: None,
                key_url: None,
                key_url_label: None,
                hint: None,
            },
            // STT Cloud
            Self::Whisper => VoiceProviderMeta {
                description: "Best accuracy, handles accents and background noise",
                key_placeholder: Some("sk-..."),
                key_url: Some("https://platform.openai.com/api-keys"),
                key_url_label: Some("platform.openai.com/api-keys"),
                hint: None,
            },
            Self::Groq => VoiceProviderMeta {
                description: "Ultra-fast Whisper inference on Groq hardware",
                key_placeholder: Some("gsk_..."),
                key_url: Some("https://console.groq.com/keys"),
                key_url_label: Some("console.groq.com/keys"),
                hint: None,
            },
            Self::Deepgram => VoiceProviderMeta {
                description: "Fast and accurate with Nova-3 model",
                key_placeholder: Some("API key"),
                key_url: Some("https://console.deepgram.com/api-keys"),
                key_url_label: Some("console.deepgram.com"),
                hint: None,
            },
            Self::Google => VoiceProviderMeta {
                description: "Supports 125+ languages with Google Speech-to-Text",
                key_placeholder: Some("API key"),
                key_url: Some("https://console.cloud.google.com/apis/credentials"),
                key_url_label: Some("console.cloud.google.com"),
                hint: None,
            },
            Self::Mistral => VoiceProviderMeta {
                description: "Fast Voxtral transcription with 13 language support",
                key_placeholder: Some("API key"),
                key_url: Some("https://console.mistral.ai/api-keys"),
                key_url_label: Some("console.mistral.ai"),
                hint: None,
            },
            Self::ElevenlabsStt => VoiceProviderMeta {
                description: "90+ languages, word timestamps. Same API key as ElevenLabs TTS",
                key_placeholder: Some("API key"),
                key_url: Some("https://elevenlabs.io/app/settings/api-keys"),
                key_url_label: Some("elevenlabs.io"),
                hint: Some(
                    "If you already have ElevenLabs TTS configured, use the same API key here.",
                ),
            },
            // STT Local
            Self::WhisperCli => VoiceProviderMeta {
                description: "Local Whisper inference via whisper-cli",
                key_placeholder: None,
                key_url: None,
                key_url_label: None,
                hint: None,
            },
            Self::SherpaOnnx => VoiceProviderMeta {
                description: "Local offline speech recognition via ONNX runtime",
                key_placeholder: None,
                key_url: None,
                key_url_label: None,
                hint: None,
            },
            Self::VoxtralLocal => VoiceProviderMeta {
                description: "Run Mistral's Voxtral model locally via vLLM server",
                key_placeholder: None,
                key_url: None,
                key_url_label: None,
                hint: None,
            },
        }
    }

    fn parse_tts_list_id(id: &str) -> Option<Self> {
        match id {
            "elevenlabs" => Some(Self::Elevenlabs),
            "openai" | "openai-tts" => Some(Self::OpenaiTts),
            "google" | "google-tts" => Some(Self::GoogleTts),
            "piper" => Some(Self::Piper),
            "coqui" => Some(Self::Coqui),
            _ => None,
        }
    }

    fn parse_stt_list_id(id: &str) -> Option<Self> {
        match id {
            "whisper" => Some(Self::Whisper),
            "groq" => Some(Self::Groq),
            "deepgram" => Some(Self::Deepgram),
            "google" => Some(Self::Google),
            "mistral" => Some(Self::Mistral),
            "elevenlabs" | "elevenlabs-stt" => Some(Self::ElevenlabsStt),
            "voxtral-local" => Some(Self::VoxtralLocal),
            "whisper-cli" => Some(Self::WhisperCli),
            "sherpa-onnx" => Some(Self::SherpaOnnx),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct VoiceProviderInfo {
    id: VoiceProviderId,
    name: String,
    #[serde(rename = "type")]
    provider_type: String,
    category: String,
    description: String,
    available: bool,
    enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    key_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key_placeholder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key_url_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    binary_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status_message: Option<String>,
    capabilities: serde_json::Value,
    settings: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    settings_summary: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(super) struct VoiceProvidersResponse {
    tts: Vec<VoiceProviderInfo>,
    stt: Vec<VoiceProviderInfo>,
}

/// Detect all available voice providers with their availability status.
pub(super) async fn detect_voice_providers(
    config: &moltis_config::MoltisConfig,
) -> serde_json::Value {
    use secrecy::ExposeSecret;

    // Check for API keys from environment variables
    let env_openai_key = std::env::var("OPENAI_API_KEY").ok();
    let env_elevenlabs_key = std::env::var("ELEVENLABS_API_KEY").ok();
    let env_google_key = std::env::var("GOOGLE_API_KEY")
        .or_else(|_| std::env::var("GOOGLE_CLOUD_API_KEY"))
        .ok();
    let env_groq_key = std::env::var("GROQ_API_KEY").ok();
    let env_deepgram_key = std::env::var("DEEPGRAM_API_KEY").ok();
    let env_mistral_key = std::env::var("MISTRAL_API_KEY").ok();

    // Check for API keys from LLM providers config
    let llm_openai_key = config
        .providers
        .get("openai")
        .and_then(|p| p.api_key.as_ref())
        .map(|k| k.expose_secret().to_string());
    let llm_groq_key = config
        .providers
        .get("groq")
        .and_then(|p| p.api_key.as_ref())
        .map(|k| k.expose_secret().to_string());
    let _llm_deepseek_key = config
        .providers
        .get("deepseek")
        .and_then(|p| p.api_key.as_ref())
        .map(|k| k.expose_secret().to_string());

    // Check for local binaries
    let whisper_cli_available = check_binary_available("whisper-cpp")
        .await
        .or(check_binary_available("whisper").await);
    let piper_available = check_binary_available("piper").await;
    let sherpa_onnx_available = check_binary_available("sherpa-onnx-offline").await;
    let coqui_server_running = check_coqui_server(&config.voice.tts.coqui.endpoint).await;
    let tts_server_binary = check_binary_available("tts-server").await;

    // Build TTS providers list
    let tts_providers = vec![
        build_provider_info(
            VoiceProviderId::Elevenlabs,
            "ElevenLabs",
            "tts",
            "cloud",
            config.voice.tts.elevenlabs.api_key.is_some() || env_elevenlabs_key.is_some(),
            config.voice.tts.provider == "elevenlabs" && config.voice.tts.enabled,
            key_source(
                config.voice.tts.elevenlabs.api_key.is_some(),
                env_elevenlabs_key.is_some(),
                false,
            ),
            None,
            None,
        ),
        build_provider_info(
            VoiceProviderId::OpenaiTts,
            "OpenAI TTS",
            "tts",
            "cloud",
            config.voice.tts.openai.api_key.is_some()
                || env_openai_key.is_some()
                || llm_openai_key.is_some(),
            config.voice.tts.provider == "openai" && config.voice.tts.enabled,
            key_source(
                config.voice.tts.openai.api_key.is_some(),
                env_openai_key.is_some(),
                llm_openai_key.is_some(),
            ),
            None,
            None,
        ),
        build_provider_info(
            VoiceProviderId::GoogleTts,
            "Google Cloud TTS",
            "tts",
            "cloud",
            config.voice.tts.google.api_key.is_some() || env_google_key.is_some(),
            config.voice.tts.provider == "google" && config.voice.tts.enabled,
            key_source(
                config.voice.tts.google.api_key.is_some(),
                env_google_key.is_some(),
                false,
            ),
            None,
            None,
        ),
        build_provider_info(
            VoiceProviderId::Piper,
            "Piper",
            "tts",
            "local",
            piper_available.is_some() && config.voice.tts.piper.model_path.is_some(),
            config.voice.tts.provider == "piper" && config.voice.tts.enabled,
            None,
            piper_available.clone(),
            if piper_available.is_none() {
                Some(
                    "piper binary not found. Install from https://github.com/rhasspy/piper/releases",
                )
            } else if config.voice.tts.piper.model_path.is_none() {
                Some(
                    "model not configured - download voice models from https://rhasspy.github.io/piper-samples/",
                )
            } else {
                None
            },
        ),
        build_provider_info(
            VoiceProviderId::Coqui,
            "Coqui TTS",
            "tts",
            "local",
            coqui_server_running,
            config.voice.tts.provider == "coqui" && config.voice.tts.enabled,
            None,
            tts_server_binary,
            if !coqui_server_running {
                Some("server not running")
            } else {
                None
            },
        ),
    ];

    // Check voxtral local server
    let voxtral_server_running = check_vllm_server(&config.voice.stt.voxtral_local.endpoint).await;

    // Build STT providers list
    let stt_providers = vec![
        build_provider_info(
            VoiceProviderId::Whisper,
            "OpenAI Whisper",
            "stt",
            "cloud",
            config.voice.stt.whisper.api_key.is_some()
                || env_openai_key.is_some()
                || llm_openai_key.is_some(),
            config.voice.stt.provider == Some(VoiceSttProvider::Whisper)
                && config.voice.stt.enabled,
            key_source(
                config.voice.stt.whisper.api_key.is_some(),
                env_openai_key.is_some(),
                llm_openai_key.is_some(),
            ),
            None,
            None,
        ),
        build_provider_info(
            VoiceProviderId::Groq,
            "Groq",
            "stt",
            "cloud",
            config.voice.stt.groq.api_key.is_some()
                || env_groq_key.is_some()
                || llm_groq_key.is_some(),
            config.voice.stt.provider == Some(VoiceSttProvider::Groq) && config.voice.stt.enabled,
            key_source(
                config.voice.stt.groq.api_key.is_some(),
                env_groq_key.is_some(),
                llm_groq_key.is_some(),
            ),
            None,
            None,
        ),
        build_provider_info(
            VoiceProviderId::Deepgram,
            "Deepgram",
            "stt",
            "cloud",
            config.voice.stt.deepgram.api_key.is_some() || env_deepgram_key.is_some(),
            config.voice.stt.provider == Some(VoiceSttProvider::Deepgram)
                && config.voice.stt.enabled,
            key_source(
                config.voice.stt.deepgram.api_key.is_some(),
                env_deepgram_key.is_some(),
                false,
            ),
            None,
            None,
        ),
        build_provider_info(
            VoiceProviderId::Google,
            "Google Cloud STT",
            "stt",
            "cloud",
            config.voice.stt.google.api_key.is_some() || env_google_key.is_some(),
            config.voice.stt.provider == Some(VoiceSttProvider::Google) && config.voice.stt.enabled,
            key_source(
                config.voice.stt.google.api_key.is_some(),
                env_google_key.is_some(),
                false,
            ),
            None,
            None,
        ),
        build_provider_info(
            VoiceProviderId::Mistral,
            "Mistral (Voxtral)",
            "stt",
            "cloud",
            config.voice.stt.mistral.api_key.is_some() || env_mistral_key.is_some(),
            config.voice.stt.provider == Some(VoiceSttProvider::Mistral)
                && config.voice.stt.enabled,
            key_source(
                config.voice.stt.mistral.api_key.is_some(),
                env_mistral_key.is_some(),
                false,
            ),
            None,
            None,
        ),
        build_provider_info(
            VoiceProviderId::ElevenlabsStt,
            "ElevenLabs Scribe",
            "stt",
            "cloud",
            config.voice.stt.elevenlabs.api_key.is_some()
                || config.voice.tts.elevenlabs.api_key.is_some()
                || env_elevenlabs_key.is_some(),
            config.voice.stt.provider == Some(VoiceSttProvider::ElevenLabs)
                && config.voice.stt.enabled,
            key_source(
                config.voice.stt.elevenlabs.api_key.is_some()
                    || config.voice.tts.elevenlabs.api_key.is_some(),
                env_elevenlabs_key.is_some(),
                false,
            ),
            None,
            None,
        ),
        build_provider_info(
            VoiceProviderId::VoxtralLocal,
            "Voxtral (Local)",
            "stt",
            "local",
            voxtral_server_running,
            config.voice.stt.provider == Some(VoiceSttProvider::VoxtralLocal)
                && config.voice.stt.enabled,
            None,
            None,
            if !voxtral_server_running {
                Some("server not running")
            } else {
                None
            },
        ),
        build_provider_info(
            VoiceProviderId::WhisperCli,
            "whisper.cpp",
            "stt",
            "local",
            whisper_cli_available.is_some() && config.voice.stt.whisper_cli.model_path.is_some(),
            config.voice.stt.provider == Some(VoiceSttProvider::WhisperCli)
                && config.voice.stt.enabled,
            None,
            whisper_cli_available.clone(),
            if whisper_cli_available.is_none() {
                Some(
                    "whisper-cpp binary not found. Install with: brew install whisper-cpp (macOS) or build from https://github.com/ggerganov/whisper.cpp",
                )
            } else if config.voice.stt.whisper_cli.model_path.is_none() {
                Some(
                    "model not configured - download a GGML model from https://huggingface.co/ggerganov/whisper.cpp",
                )
            } else {
                None
            },
        ),
        build_provider_info(
            VoiceProviderId::SherpaOnnx,
            "sherpa-onnx",
            "stt",
            "local",
            sherpa_onnx_available.is_some() && config.voice.stt.sherpa_onnx.model_dir.is_some(),
            config.voice.stt.provider == Some(VoiceSttProvider::SherpaOnnx)
                && config.voice.stt.enabled,
            None,
            sherpa_onnx_available.clone(),
            if sherpa_onnx_available.is_none() {
                Some(
                    "sherpa-onnx binary not found. Download from https://github.com/k2-fsa/sherpa-onnx/releases",
                )
            } else if config.voice.stt.sherpa_onnx.model_dir.is_none() {
                Some(
                    "model not configured - download models from https://github.com/k2-fsa/sherpa-onnx/releases",
                )
            } else {
                None
            },
        ),
    ];

    let tts_with_details = filter_listed_voice_providers(
        tts_providers
            .into_iter()
            .map(|provider| enrich_voice_provider(provider, config))
            .collect::<Vec<_>>(),
        &config.voice.tts.providers,
        VoiceProviderId::parse_tts_list_id,
    );
    let stt_with_details = filter_listed_voice_providers(
        stt_providers
            .into_iter()
            .map(|provider| enrich_voice_provider(provider, config))
            .collect::<Vec<_>>(),
        &config.voice.stt.providers,
        VoiceProviderId::parse_stt_list_id,
    );

    serde_json::to_value(VoiceProvidersResponse {
        tts: tts_with_details,
        stt: stt_with_details,
    })
    .unwrap_or_else(|_| serde_json::json!({ "tts": [], "stt": [] }))
}

fn filter_listed_voice_providers(
    providers: Vec<VoiceProviderInfo>,
    listed_provider_ids: &[String],
    parse_provider_id: fn(&str) -> Option<VoiceProviderId>,
) -> Vec<VoiceProviderInfo> {
    if listed_provider_ids.is_empty() {
        return providers;
    }

    let allowed_ids: Vec<_> = listed_provider_ids
        .iter()
        .filter_map(|id| parse_provider_id(id))
        .collect();

    providers
        .into_iter()
        .filter(|provider| allowed_ids.contains(&provider.id))
        .collect()
}

fn enrich_voice_provider(
    mut provider: VoiceProviderInfo,
    config: &moltis_config::MoltisConfig,
) -> VoiceProviderInfo {
    let (capabilities, settings, summary) = match provider.id {
        VoiceProviderId::OpenaiTts => (
            serde_json::json!({
                "voiceChoices": ["alloy", "echo", "fable", "onyx", "nova", "shimmer"],
                "modelChoices": ["tts-1", "tts-1-hd"],
                "customVoice": true,
                "customModel": true,
            }),
            serde_json::json!({
                "voice": config.voice.tts.openai.voice,
                "model": config.voice.tts.openai.model,
            }),
            format_voice_summary(
                config.voice.tts.openai.voice.clone(),
                config.voice.tts.openai.model.clone(),
            ),
        ),
        VoiceProviderId::Elevenlabs => (
            serde_json::json!({
                "voiceId": true,
                "modelChoices": ["eleven_flash_v2_5", "eleven_turbo_v2_5", "eleven_multilingual_v2"],
                "customVoice": true,
                "customModel": true,
            }),
            serde_json::json!({
                "voiceId": config.voice.tts.elevenlabs.voice_id,
                "model": config.voice.tts.elevenlabs.model,
            }),
            format_voice_summary(
                config.voice.tts.elevenlabs.voice_id.clone(),
                config.voice.tts.elevenlabs.model.clone(),
            ),
        ),
        VoiceProviderId::GoogleTts => (
            serde_json::json!({
                "languageChoices": ["en-US", "en-GB", "fr-FR", "de-DE", "es-ES", "it-IT", "pt-BR", "ja-JP"],
                "exampleVoices": [
                    "en-US-Neural2-A", "en-US-Neural2-C", "en-GB-Neural2-A", "en-GB-Neural2-B",
                    "fr-FR-Neural2-A", "de-DE-Neural2-B"
                ],
                "customVoice": true,
                "customLanguage": true,
            }),
            serde_json::json!({
                "voice": config.voice.tts.google.voice,
                "languageCode": config.voice.tts.google.language_code,
            }),
            format_voice_summary(
                config.voice.tts.google.voice.clone(),
                config.voice.tts.google.language_code.clone(),
            ),
        ),
        VoiceProviderId::Coqui => (
            serde_json::json!({
                "speaker": true,
                "language": true,
                "customSpeaker": true,
                "customLanguage": true,
            }),
            serde_json::json!({
                "speaker": config.voice.tts.coqui.speaker,
                "language": config.voice.tts.coqui.language,
                "model": config.voice.tts.coqui.model,
            }),
            format_voice_summary(
                config.voice.tts.coqui.speaker.clone(),
                config.voice.tts.coqui.language.clone(),
            ),
        ),
        VoiceProviderId::Piper => (
            serde_json::json!({
                "speakerId": true,
                "customModelPath": true,
            }),
            serde_json::json!({
                "speakerId": config.voice.tts.piper.speaker_id,
                "modelPath": config.voice.tts.piper.model_path,
            }),
            format_voice_summary(
                config
                    .voice
                    .tts
                    .piper
                    .speaker_id
                    .map(|s| format!("speaker {}", s)),
                None,
            ),
        ),
        _ => (serde_json::json!({}), serde_json::json!({}), None),
    };

    provider.capabilities = capabilities;
    provider.settings = settings;
    provider.settings_summary = summary;
    provider
}

fn format_voice_summary(primary: Option<String>, secondary: Option<String>) -> Option<String> {
    match (primary, secondary) {
        (Some(a), Some(b)) if !a.is_empty() && !b.is_empty() => Some(format!("{} · {}", a, b)),
        (Some(a), _) if !a.is_empty() => Some(a),
        (_, Some(b)) if !b.is_empty() => Some(b),
        _ => None,
    }
}

#[derive(Debug, serde::Deserialize)]
struct ElevenLabsVoiceListResponse {
    voices: Vec<ElevenLabsVoice>,
}

#[derive(Debug, serde::Deserialize)]
struct ElevenLabsVoice {
    voice_id: String,
    name: String,
}

#[derive(Debug, serde::Deserialize)]
struct ElevenLabsModel {
    model_id: String,
    name: String,
    #[serde(default)]
    can_do_text_to_speech: Option<bool>,
}

pub(super) async fn fetch_elevenlabs_catalog(
    config: &moltis_config::MoltisConfig,
) -> serde_json::Value {
    use secrecy::ExposeSecret;

    let fallback_models = vec![
        serde_json::json!({ "id": "eleven_flash_v2_5", "name": "Eleven Flash v2.5" }),
        serde_json::json!({ "id": "eleven_turbo_v2_5", "name": "Eleven Turbo v2.5" }),
        serde_json::json!({ "id": "eleven_multilingual_v2", "name": "Eleven Multilingual v2" }),
        serde_json::json!({ "id": "eleven_monolingual_v1", "name": "Eleven Monolingual v1" }),
    ];

    let api_key = config
        .voice
        .tts
        .elevenlabs
        .api_key
        .as_ref()
        .or(config.voice.stt.elevenlabs.api_key.as_ref())
        .map(|k| k.expose_secret().to_string())
        .or_else(|| std::env::var("ELEVENLABS_API_KEY").ok());

    let Some(api_key) = api_key else {
        return serde_json::json!({
            "voices": [],
            "models": fallback_models,
            "warning": "No ElevenLabs API key configured. Showing known model suggestions only.",
        });
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build();
    let Ok(client) = client else {
        return serde_json::json!({ "voices": [], "models": fallback_models });
    };

    let voices_req = client
        .get("https://api.elevenlabs.io/v1/voices")
        .header("xi-api-key", &api_key)
        .send();
    let models_req = client
        .get("https://api.elevenlabs.io/v1/models")
        .header("xi-api-key", &api_key)
        .send();

    let (voices_res, models_res) = tokio::join!(voices_req, models_req);

    let voices = match voices_res {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<ElevenLabsVoiceListResponse>().await {
                Ok(body) => body
                    .voices
                    .into_iter()
                    .map(|v| serde_json::json!({ "id": v.voice_id, "name": v.name }))
                    .collect::<Vec<_>>(),
                Err(_) => Vec::new(),
            }
        },
        _ => Vec::new(),
    };

    let models = match models_res {
        Ok(resp) if resp.status().is_success() => match resp.json::<Vec<ElevenLabsModel>>().await {
            Ok(body) => {
                let parsed: Vec<_> = body
                    .into_iter()
                    .filter(|m| m.can_do_text_to_speech.unwrap_or(true))
                    .map(|m| serde_json::json!({ "id": m.model_id, "name": m.name }))
                    .collect();
                if parsed.is_empty() {
                    fallback_models.clone()
                } else {
                    parsed
                }
            },
            Err(_) => fallback_models.clone(),
        },
        _ => fallback_models.clone(),
    };

    serde_json::json!({
        "voices": voices,
        "models": models,
    })
}

fn build_provider_info(
    id: VoiceProviderId,
    name: &str,
    provider_type: &str,
    category: &str,
    available: bool,
    enabled: bool,
    key_source: Option<&str>,
    binary_path: Option<String>,
    status_message: Option<&str>,
) -> VoiceProviderInfo {
    let meta = id.meta();
    VoiceProviderInfo {
        id,
        name: name.to_string(),
        provider_type: provider_type.to_string(),
        category: category.to_string(),
        description: meta.description.to_string(),
        available,
        enabled,
        key_source: key_source.map(str::to_string),
        key_placeholder: meta.key_placeholder.map(str::to_string),
        key_url: meta.key_url.map(str::to_string),
        key_url_label: meta.key_url_label.map(str::to_string),
        hint: meta.hint.map(str::to_string),
        binary_path,
        status_message: status_message.map(str::to_string),
        capabilities: serde_json::json!({}),
        settings: serde_json::json!({}),
        settings_summary: None,
    }
}

fn key_source(in_config: bool, in_env: bool, in_llm_provider: bool) -> Option<&'static str> {
    if in_config {
        Some("config")
    } else if in_env {
        Some("env")
    } else if in_llm_provider {
        Some("llm_provider")
    } else {
        None
    }
}

pub(super) fn apply_voice_provider_settings(
    cfg: &mut moltis_config::MoltisConfig,
    provider: &str,
    params: &serde_json::Value,
) {
    let get_string = |key: &str| -> Option<String> {
        params
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
    };

    match provider {
        "openai" | "openai-tts" => {
            if let Some(voice) = get_string("voice") {
                cfg.voice.tts.openai.voice = Some(voice);
            }
            if let Some(model) = get_string("model") {
                cfg.voice.tts.openai.model = Some(model);
            }
        },
        "elevenlabs" => {
            if let Some(voice_id) = get_string("voiceId") {
                cfg.voice.tts.elevenlabs.voice_id = Some(voice_id);
            }
            if let Some(model) = get_string("model") {
                cfg.voice.tts.elevenlabs.model = Some(model);
            }
        },
        "google" | "google-tts" => {
            if let Some(voice) = get_string("voice") {
                cfg.voice.tts.google.voice = Some(voice);
            }
            if let Some(language_code) = get_string("languageCode") {
                cfg.voice.tts.google.language_code = Some(language_code);
            }
        },
        "coqui" => {
            if let Some(speaker) = get_string("speaker") {
                cfg.voice.tts.coqui.speaker = Some(speaker);
            }
            if let Some(language) = get_string("language") {
                cfg.voice.tts.coqui.language = Some(language);
            }
            if let Some(model) = get_string("model") {
                cfg.voice.tts.coqui.model = Some(model);
            }
        },
        "piper" => {
            if let Some(model_path) = get_string("modelPath") {
                cfg.voice.tts.piper.model_path = Some(model_path);
            }
            if let Some(speaker_id) = params
                .get("speakerId")
                .and_then(serde_json::Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
            {
                cfg.voice.tts.piper.speaker_id = Some(speaker_id);
            }
        },
        _ => {},
    }
}

async fn check_binary_available(name: &str) -> Option<String> {
    // Try to find the binary in PATH
    if let Ok(output) = tokio::process::Command::new("which")
        .arg(name)
        .output()
        .await
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Some(path);
        }
    }
    None
}

/// Check if Coqui TTS server is running.
async fn check_coqui_server(endpoint: &str) -> bool {
    // Try to connect to the server's health endpoint
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap_or_default();

    // Coqui TTS server responds to GET /
    if let Ok(resp) = client.get(endpoint).send().await {
        return resp.status().is_success();
    }
    false
}

/// Check if vLLM server is running (for Voxtral local).
async fn check_vllm_server(endpoint: &str) -> bool {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap_or_default();

    // vLLM exposes /health endpoint
    let health_url = format!("{}/health", endpoint.trim_end_matches('/'));
    if let Ok(resp) = client.get(&health_url).send().await {
        return resp.status().is_success();
    }
    false
}

/// Toggle a voice provider on/off by updating the config file.
pub(super) fn toggle_voice_provider(
    provider: &str,
    enabled: bool,
    provider_type: &str,
) -> Result<(), anyhow::Error> {
    moltis_config::update_config(|cfg| {
        match provider_type {
            "tts" => {
                if enabled {
                    // Map provider id to config provider name
                    let config_provider = match provider {
                        "openai-tts" => "openai",
                        "google-tts" => "google",
                        other => other,
                    };
                    cfg.voice.tts.provider = config_provider.to_string();
                    cfg.voice.tts.enabled = true;
                } else if cfg.voice.tts.provider == provider
                    || (provider == "openai-tts" && cfg.voice.tts.provider == "openai")
                    || (provider == "google-tts" && cfg.voice.tts.provider == "google")
                {
                    cfg.voice.tts.enabled = false;
                }
            },
            "stt" => {
                let stt_provider = VoiceSttProvider::parse(provider);
                if enabled {
                    if let Some(provider_id) = stt_provider {
                        cfg.voice.stt.provider = Some(provider_id);
                        cfg.voice.stt.enabled = true;
                    }
                } else if stt_provider
                    .is_some_and(|provider_id| cfg.voice.stt.provider == Some(provider_id))
                {
                    cfg.voice.stt.enabled = false;
                }
            },
            _ => {},
        }
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use {super::*, secrecy::Secret};

    fn test_voice_provider(id: VoiceProviderId) -> VoiceProviderInfo {
        let meta = id.meta();
        VoiceProviderInfo {
            id,
            name: String::new(),
            provider_type: String::new(),
            category: String::new(),
            description: meta.description.to_string(),
            available: false,
            enabled: false,
            key_source: None,
            key_placeholder: meta.key_placeholder.map(str::to_string),
            key_url: meta.key_url.map(str::to_string),
            key_url_label: meta.key_url_label.map(str::to_string),
            hint: meta.hint.map(str::to_string),
            binary_path: None,
            status_message: None,
            capabilities: serde_json::json!({}),
            settings: serde_json::json!({}),
            settings_summary: None,
        }
    }

    #[test]
    fn parse_voice_provider_list_aliases() {
        assert_eq!(
            VoiceProviderId::parse_tts_list_id("openai"),
            Some(VoiceProviderId::OpenaiTts)
        );
        assert_eq!(
            VoiceProviderId::parse_tts_list_id("google-tts"),
            Some(VoiceProviderId::GoogleTts)
        );
        assert_eq!(
            VoiceProviderId::parse_stt_list_id("elevenlabs"),
            Some(VoiceProviderId::ElevenlabsStt)
        );
        assert_eq!(
            VoiceProviderId::parse_stt_list_id("sherpa-onnx"),
            Some(VoiceProviderId::SherpaOnnx)
        );
    }

    #[test]
    fn filter_listed_voice_providers_keeps_all_when_list_is_empty() {
        let filtered = filter_listed_voice_providers(
            vec![
                test_voice_provider(VoiceProviderId::OpenaiTts),
                test_voice_provider(VoiceProviderId::GoogleTts),
            ],
            &[],
            VoiceProviderId::parse_tts_list_id,
        );
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn filter_listed_voice_providers_filters_tts_ids() {
        let filtered = filter_listed_voice_providers(
            vec![
                test_voice_provider(VoiceProviderId::OpenaiTts),
                test_voice_provider(VoiceProviderId::GoogleTts),
                test_voice_provider(VoiceProviderId::Piper),
            ],
            &["openai".to_string(), "piper".to_string()],
            VoiceProviderId::parse_tts_list_id,
        );
        let ids: Vec<_> = filtered.into_iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![
            VoiceProviderId::OpenaiTts,
            VoiceProviderId::Piper
        ]);
    }

    #[tokio::test]
    async fn detect_voice_providers_marks_selected_stt_provider_when_some() {
        let mut config = moltis_config::MoltisConfig::default();
        config.voice.stt.enabled = true;
        config.voice.stt.provider = Some(VoiceSttProvider::Whisper);
        config.voice.stt.whisper.api_key = Some(Secret::new("test-whisper-key".to_string()));

        let detected = detect_voice_providers(&config).await;
        let Some(stt) = detected["stt"].as_array() else {
            panic!("stt list missing");
        };
        let Some(whisper) = stt.iter().find(|provider| provider["id"] == "whisper") else {
            panic!("whisper provider missing");
        };

        assert_eq!(whisper["enabled"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn detect_voice_providers_does_not_mark_stt_provider_when_none() {
        let mut config = moltis_config::MoltisConfig::default();
        config.voice.stt.enabled = true;
        config.voice.stt.provider = None;
        config.voice.stt.whisper.api_key = Some(Secret::new("test-whisper-key".to_string()));

        let detected = detect_voice_providers(&config).await;
        let Some(stt) = detected["stt"].as_array() else {
            panic!("stt list missing");
        };
        let enabled_count = stt
            .iter()
            .filter(|provider| provider["enabled"].as_bool() == Some(true))
            .count();

        assert_eq!(enabled_count, 0);
    }
}
