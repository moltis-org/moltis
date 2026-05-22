//! Piper TTS (local) provider.
//!
//! Piper is a fast, local neural text-to-speech system.
//! Install: pip install piper-tts
//! Or download pre-built binaries from: https://github.com/rhasspy/piper/releases

use {
    crate::{
        config::PiperTtsConfig,
        tts::{AudioFormat, AudioOutput, SynthesizeRequest, TtsProvider, Voice, audio},
    },
    anyhow::{Result, anyhow},
    async_trait::async_trait,
    bytes::Bytes,
    std::{path::Path, process::Stdio},
    tokio::{io::AsyncWriteExt, process::Command},
};

const DEFAULT_SAMPLE_RATE_HZ: u32 = 22_050;

/// Piper TTS (local) provider.
pub struct PiperTts {
    binary_path: Option<String>,
    model_path: Option<String>,
    config_path: Option<String>,
    speaker_id: Option<u32>,
    length_scale: f32,
}

impl PiperTts {
    /// Create a new Piper TTS provider from config.
    #[must_use]
    pub fn new(config: &PiperTtsConfig) -> Self {
        Self {
            binary_path: config.binary_path.clone(),
            model_path: config.model_path.clone(),
            config_path: config.config_path.clone(),
            speaker_id: config.speaker_id,
            length_scale: config.length_scale.unwrap_or(1.0),
        }
    }

    fn get_binary(&self) -> &str {
        self.binary_path.as_deref().unwrap_or("piper")
    }

    fn expand_path(path: &str) -> String {
        if let Some(stripped) = path.strip_prefix("~/")
            && let Some(home) = dirs::home_dir()
        {
            return home.join(stripped).to_string_lossy().into_owned();
        }
        path.to_string()
    }

    async fn sample_rate_hz(&self, model_path: &str) -> u32 {
        let config_path = self
            .config_path
            .as_deref()
            .map(Self::expand_path)
            .unwrap_or_else(|| format!("{model_path}.json"));

        Self::read_sample_rate_hz(Path::new(&config_path))
            .await
            .unwrap_or(DEFAULT_SAMPLE_RATE_HZ)
    }

    async fn read_sample_rate_hz(path: &Path) -> Option<u32> {
        let config = tokio::fs::read(path).await.ok()?;
        let value: serde_json::Value = serde_json::from_slice(&config).ok()?;
        let sample_rate = value.pointer("/audio/sample_rate")?.as_u64()?;
        u32::try_from(sample_rate).ok()
    }
}

#[async_trait]
impl TtsProvider for PiperTts {
    fn id(&self) -> &'static str {
        "piper"
    }

    fn name(&self) -> &'static str {
        "Piper"
    }

    fn is_configured(&self) -> bool {
        self.model_path.is_some()
    }

    async fn voices(&self) -> Result<Vec<Voice>> {
        // Piper doesn't have a dynamic voice list - the voice is determined by the model file
        // Return a single voice representing the configured model
        if let Some(model_path) = &self.model_path {
            let model_name = Path::new(model_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("piper-voice");

            Ok(vec![Voice {
                id: "default".into(),
                name: model_name.into(),
                description: Some(format!("Model: {}", model_path)),
                preview_url: None,
            }])
        } else {
            Ok(vec![])
        }
    }

    async fn synthesize(&self, request: SynthesizeRequest) -> Result<AudioOutput> {
        let model_path = self
            .model_path
            .as_ref()
            .ok_or_else(|| anyhow!("Piper model path not configured"))?;

        let model_path = Self::expand_path(model_path);

        let mut cmd = Command::new(self.get_binary());
        cmd.arg("--model").arg(&model_path);

        // Add config path if specified
        if let Some(config_path) = &self.config_path {
            cmd.arg("--config").arg(Self::expand_path(config_path));
        }

        // Add speaker ID for multi-speaker models
        if let Some(speaker_id) = self.speaker_id {
            cmd.arg("--speaker").arg(speaker_id.to_string());
        }

        // Set length scale (speaking rate)
        let length_scale = request.speed.map(|s| 1.0 / s).unwrap_or(self.length_scale);
        cmd.arg("--length-scale").arg(length_scale.to_string());

        // Output format - Piper outputs raw PCM by default, we can pipe to ffmpeg for conversion
        // For simplicity, output WAV which is PCM with headers
        cmd.arg("--output-raw");

        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            anyhow!(
                "Failed to spawn piper binary '{}': {}. Install with: pip install piper-tts",
                self.get_binary(),
                e
            )
        })?;

        // Write text to stdin
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(request.text.as_bytes()).await?;
            stdin.shutdown().await?;
        }

        let output = child.wait_with_output().await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("Piper failed: {}", stderr));
        }

        // Piper outputs raw 16-bit mono PCM; wrap it as WAV unless raw PCM was requested.
        let (data, format) = match request.output_format {
            AudioFormat::Pcm => (Bytes::from(output.stdout), AudioFormat::Pcm),
            AudioFormat::Mp3
            | AudioFormat::Opus
            | AudioFormat::Aac
            | AudioFormat::Wav
            | AudioFormat::Webm => {
                let sample_rate_hz = self.sample_rate_hz(&model_path).await;
                (
                    audio::wav_from_s16le_mono(&output.stdout, sample_rate_hz)?,
                    AudioFormat::Wav,
                )
            },
        };

        Ok(AudioOutput {
            data,
            format,
            duration_ms: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_piper_not_configured_without_model() {
        let config = PiperTtsConfig::default();
        let tts = PiperTts::new(&config);
        assert!(!tts.is_configured());
    }

    #[test]
    fn test_piper_configured_with_model() {
        let config = PiperTtsConfig {
            model_path: Some("~/.moltis/models/en_US-lessac-medium.onnx".into()),
            ..Default::default()
        };
        let tts = PiperTts::new(&config);
        assert!(tts.is_configured());
    }

    #[test]
    fn test_piper_id_and_name() {
        let config = PiperTtsConfig::default();
        let tts = PiperTts::new(&config);
        assert_eq!(tts.id(), "piper");
        assert_eq!(tts.name(), "Piper");
        assert!(!tts.supports_ssml());
    }

    #[test]
    fn test_expand_path() {
        let expanded = PiperTts::expand_path("~/test/path");
        assert!(!expanded.starts_with("~/"));
    }

    #[tokio::test]
    async fn test_read_sample_rate_hz_from_piper_config() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("voice.onnx.json");
        std::fs::write(&path, r#"{"audio":{"sample_rate":16000}}"#)?;

        assert_eq!(PiperTts::read_sample_rate_hz(&path).await, Some(16_000));
        Ok(())
    }
}
