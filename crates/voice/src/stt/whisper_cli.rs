//! whisper-cli (whisper.cpp) STT provider implementation.
//!
//! whisper.cpp is a port of OpenAI's Whisper model to C/C++, offering
//! fast local inference on CPU or GPU. This provider wraps the CLI tool.
//!
//! Installation:
//! - macOS: `brew install whisper-cpp`
//! - From source: https://github.com/ggerganov/whisper.cpp
//!
//! Models can be downloaded from:
//! https://huggingface.co/ggerganov/whisper.cpp

use {
    anyhow::{Context, Result, anyhow},
    async_trait::async_trait,
    serde::Deserialize,
    std::process::Stdio,
    tokio::process::Command,
};

use super::{SttProvider, TranscribeRequest, Transcript, Word, cli_utils};

/// Binary name for whisper.cpp CLI.
const BINARY_NAME: &str = "whisper-cli";

/// Alternative binary name (some installations use this).
const ALT_BINARY_NAME: &str = "whisper";

/// whisper-cli (whisper.cpp) STT provider.
#[derive(Clone, Debug)]
pub struct WhisperCliStt {
    binary_path: Option<String>,
    model_path: Option<String>,
    language: Option<String>,
}

impl Default for WhisperCliStt {
    fn default() -> Self {
        Self::new()
    }
}

impl WhisperCliStt {
    /// Create a new whisper-cli STT provider.
    #[must_use]
    pub fn new() -> Self {
        Self {
            binary_path: None,
            model_path: None,
            language: None,
        }
    }

    /// Create with custom options.
    #[must_use]
    pub fn with_options(
        binary_path: Option<String>,
        model_path: Option<String>,
        language: Option<String>,
    ) -> Self {
        Self {
            binary_path,
            model_path,
            language,
        }
    }

    /// Find the whisper-cli binary.
    fn find_binary(&self) -> Option<std::path::PathBuf> {
        cli_utils::find_binary(BINARY_NAME, self.binary_path.as_deref())
            .or_else(|| cli_utils::find_binary(ALT_BINARY_NAME, None))
    }

    /// Get the model path, returning an error if not configured.
    fn get_model_path(&self) -> Result<std::path::PathBuf> {
        self.model_path
            .as_ref()
            .map(|p| cli_utils::expand_tilde(p))
            .filter(|p| p.exists())
            .ok_or_else(|| anyhow!("whisper-cli model path not configured or file not found"))
    }
}

#[async_trait]
impl SttProvider for WhisperCliStt {
    fn id(&self) -> &'static str {
        "whisper-cli"
    }

    fn name(&self) -> &'static str {
        "whisper.cpp"
    }

    fn is_configured(&self) -> bool {
        // Need both binary and model to be configured
        self.find_binary().is_some()
            && self
                .model_path
                .as_ref()
                .is_some_and(|p| cli_utils::expand_tilde(p).exists())
    }

    async fn transcribe(&self, request: TranscribeRequest) -> Result<Transcript> {
        let binary = self
            .find_binary()
            .ok_or_else(|| anyhow!("whisper-cli binary not found in PATH"))?;

        let model_path = self.get_model_path()?;

        // Write audio to temp file (whisper-cli needs a file path)
        // Note: whisper-cli prefers WAV format, but handles others via ffmpeg
        let (_temp_file, audio_path) = cli_utils::write_temp_audio(&request.audio, request.format)?;

        // Build command
        let mut cmd = Command::new(&binary);
        cmd.arg("-m").arg(&model_path);
        cmd.arg("-f").arg(&audio_path);
        cmd.arg("-oj"); // Output JSON
        cmd.arg("--no-prints"); // Suppress progress output

        // Language hint
        if let Some(ref lang) = request.language.as_ref().or(self.language.as_ref()) {
            cmd.arg("-l").arg(lang);
        }

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let output = cmd
            .output()
            .await
            .context("failed to execute whisper-cli")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("whisper-cli failed: {}", stderr));
        }

        // Parse JSON output
        let stdout = String::from_utf8_lossy(&output.stdout);

        // whisper-cli outputs JSON with transcription data
        let response: WhisperCliResponse =
            serde_json::from_str(&stdout).context("failed to parse whisper-cli JSON output")?;

        // Combine all segment texts
        let text = response
            .transcription
            .iter()
            .map(|seg| seg.text.trim())
            .collect::<Vec<_>>()
            .join(" ");

        // Extract words if available (from timestamps)
        let words: Option<Vec<Word>> = if response
            .transcription
            .iter()
            .any(|s| s.timestamps.is_some())
        {
            Some(
                response
                    .transcription
                    .iter()
                    .filter_map(|seg| seg.timestamps.as_ref())
                    .flatten()
                    .map(|ts| Word {
                        word: ts.text.trim().to_string(),
                        start: ts.offsets.from as f32 / 1000.0,
                        end: ts.offsets.to as f32 / 1000.0,
                    })
                    .collect(),
            )
        } else {
            None
        };

        Ok(Transcript {
            text,
            language: response.result.language.clone(),
            confidence: None, // whisper-cli doesn't provide confidence
            duration_seconds: None,
            words,
        })
    }
}

// ── CLI Output Types ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct WhisperCliResponse {
    #[serde(default)]
    result: WhisperCliResult,
    #[serde(default)]
    transcription: Vec<WhisperCliSegment>,
}

#[derive(Debug, Default, Deserialize)]
struct WhisperCliResult {
    #[serde(default)]
    language: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WhisperCliSegment {
    #[serde(default)]
    text: String,
    #[serde(default)]
    timestamps: Option<Vec<WhisperCliTimestamp>>,
}

#[derive(Debug, Deserialize)]
struct WhisperCliTimestamp {
    text: String,
    offsets: WhisperCliOffsets,
}

#[derive(Debug, Deserialize)]
struct WhisperCliOffsets {
    from: u64,
    to: u64,
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, crate::tts::AudioFormat, bytes::Bytes};

    #[test]
    fn test_provider_metadata() {
        let provider = WhisperCliStt::new();
        assert_eq!(provider.id(), "whisper-cli");
        assert_eq!(provider.name(), "whisper.cpp");
        // Not configured without model path
        assert!(!provider.is_configured());
    }

    #[test]
    fn test_with_options() {
        let provider = WhisperCliStt::with_options(
            Some("/usr/local/bin/whisper-cli".into()),
            Some("~/.moltis/models/ggml-base.en.bin".into()),
            Some("en".into()),
        );
        assert_eq!(
            provider.binary_path,
            Some("/usr/local/bin/whisper-cli".into())
        );
        assert_eq!(
            provider.model_path,
            Some("~/.moltis/models/ggml-base.en.bin".into())
        );
        assert_eq!(provider.language, Some("en".into()));
    }

    #[test]
    fn test_whisper_cli_response_parsing() {
        let json = r#"{
            "result": {
                "language": "en"
            },
            "transcription": [
                {
                    "text": " Hello, how are you?",
                    "timestamps": [
                        {"text": " Hello", "offsets": {"from": 0, "to": 500}},
                        {"text": " how", "offsets": {"from": 600, "to": 800}}
                    ]
                }
            ]
        }"#;

        let response: WhisperCliResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.result.language, Some("en".into()));
        assert_eq!(response.transcription.len(), 1);
        assert_eq!(response.transcription[0].text, " Hello, how are you?");
        assert_eq!(
            response.transcription[0].timestamps.as_ref().unwrap().len(),
            2
        );
    }

    #[test]
    fn test_whisper_cli_response_minimal() {
        let json = r#"{
            "result": {},
            "transcription": [
                {"text": "Hello"}
            ]
        }"#;
        let response: WhisperCliResponse = serde_json::from_str(json).unwrap();
        assert!(response.result.language.is_none());
        assert_eq!(response.transcription[0].text, "Hello");
        assert!(response.transcription[0].timestamps.is_none());
    }

    #[tokio::test]
    async fn test_transcribe_without_config() {
        let provider = WhisperCliStt::new();
        let request = TranscribeRequest {
            audio: Bytes::from_static(b"fake audio"),
            format: AudioFormat::Mp3,
            language: None,
            prompt: None,
        };

        let result = provider.transcribe(request).await;
        assert!(result.is_err());
        // Either binary not found or model not configured
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found") || err.contains("not configured"));
    }
}
