//! sherpa-onnx STT provider implementation.
//!
//! sherpa-onnx provides offline speech recognition using ONNX runtime
//! with support for various ASR models (Whisper, Zipformer, etc.).
//!
//! Installation: https://k2-fsa.github.io/sherpa/onnx/install/index.html
//! Models: https://github.com/k2-fsa/sherpa-onnx/releases

use {
    anyhow::{Context, Result, anyhow},
    async_trait::async_trait,
    std::{path::PathBuf, process::Stdio},
    tokio::process::Command,
};

use super::{SttProvider, TranscribeRequest, Transcript, cli_utils};

/// Binary name for sherpa-onnx offline.
const BINARY_NAME: &str = "sherpa-onnx-offline";

/// sherpa-onnx STT provider for offline speech recognition.
#[derive(Clone, Debug)]
pub struct SherpaOnnxStt {
    binary_path: Option<String>,
    model_dir: Option<String>,
    language: Option<String>,
}

impl Default for SherpaOnnxStt {
    fn default() -> Self {
        Self::new()
    }
}

impl SherpaOnnxStt {
    /// Create a new sherpa-onnx STT provider.
    #[must_use]
    pub fn new() -> Self {
        Self {
            binary_path: None,
            model_dir: None,
            language: None,
        }
    }

    /// Create with custom options.
    #[must_use]
    pub fn with_options(
        binary_path: Option<String>,
        model_dir: Option<String>,
        language: Option<String>,
    ) -> Self {
        Self {
            binary_path,
            model_dir,
            language,
        }
    }

    /// Find the sherpa-onnx-offline binary.
    fn find_binary(&self) -> Option<PathBuf> {
        cli_utils::find_binary(BINARY_NAME, self.binary_path.as_deref())
    }

    /// Get the model directory, returning an error if not configured.
    fn get_model_dir(&self) -> Result<PathBuf> {
        self.model_dir
            .as_ref()
            .map(|p| cli_utils::expand_tilde(p))
            .filter(|p| p.exists() && p.is_dir())
            .ok_or_else(|| anyhow!("sherpa-onnx model directory not configured or not found"))
    }

    /// Detect model files in the model directory.
    /// sherpa-onnx models typically have: tokens.txt, encoder.onnx, decoder.onnx
    fn detect_model_files(&self, model_dir: &PathBuf) -> Result<ModelFiles> {
        let tokens = model_dir.join("tokens.txt");
        if !tokens.exists() {
            return Err(anyhow!("tokens.txt not found in model directory"));
        }

        // Look for encoder/decoder patterns (varies by model type)
        let encoder = find_file_with_prefix(model_dir, "encoder")
            .or_else(|| find_file_with_prefix(model_dir, "model"))
            .ok_or_else(|| anyhow!("encoder model not found in model directory"))?;

        let decoder = find_file_with_prefix(model_dir, "decoder");

        // Some models use a single model file, others have encoder+decoder
        Ok(ModelFiles {
            tokens,
            encoder,
            decoder,
        })
    }
}

/// Model files detected in the model directory.
struct ModelFiles {
    tokens: PathBuf,
    encoder: PathBuf,
    decoder: Option<PathBuf>,
}

/// Find a file in a directory that starts with a given prefix and ends with .onnx
fn find_file_with_prefix(dir: &PathBuf, prefix: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(prefix) && n.ends_with(".onnx"))
        })
}

#[async_trait]
impl SttProvider for SherpaOnnxStt {
    fn id(&self) -> &'static str {
        "sherpa-onnx"
    }

    fn name(&self) -> &'static str {
        "sherpa-onnx"
    }

    fn is_configured(&self) -> bool {
        // Need both binary and model directory with valid files
        if self.find_binary().is_none() {
            return false;
        }

        let Ok(model_dir) = self.get_model_dir() else {
            return false;
        };

        self.detect_model_files(&model_dir).is_ok()
    }

    async fn transcribe(&self, request: TranscribeRequest) -> Result<Transcript> {
        let binary = self
            .find_binary()
            .ok_or_else(|| anyhow!("sherpa-onnx-offline binary not found in PATH"))?;

        let model_dir = self.get_model_dir()?;
        let model_files = self.detect_model_files(&model_dir)?;

        // Write audio to temp file
        // sherpa-onnx prefers WAV but can handle other formats
        let (_temp_file, audio_path) = cli_utils::write_temp_audio(&request.audio, request.format)?;

        // Build command
        // sherpa-onnx-offline requires explicit model file paths
        let mut cmd = Command::new(&binary);

        cmd.arg(format!("--tokens={}", model_files.tokens.display()));
        cmd.arg(format!("--encoder={}", model_files.encoder.display()));

        if let Some(ref decoder) = model_files.decoder {
            cmd.arg(format!("--decoder={}", decoder.display()));
        }

        // Add audio file as positional argument
        cmd.arg(&audio_path);

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let output = cmd
            .output()
            .await
            .context("failed to execute sherpa-onnx-offline")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("sherpa-onnx-offline failed: {}", stderr));
        }

        // sherpa-onnx outputs plain text transcription to stdout
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Parse the output - sherpa-onnx typically outputs:
        // filename
        // transcription text
        // (possibly more lines for timestamps)
        let text = stdout
            .lines()
            .skip(1) // Skip filename line
            .filter(|l| !l.is_empty() && !l.starts_with("progress"))
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string();

        Ok(Transcript {
            text,
            language: self.language.clone(),
            confidence: None,
            duration_seconds: None,
            words: None, // Basic output doesn't include word timestamps
        })
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, crate::tts::AudioFormat, bytes::Bytes};

    #[test]
    fn test_provider_metadata() {
        let provider = SherpaOnnxStt::new();
        assert_eq!(provider.id(), "sherpa-onnx");
        assert_eq!(provider.name(), "sherpa-onnx");
        // Not configured without model directory
        assert!(!provider.is_configured());
    }

    #[test]
    fn test_with_options() {
        let provider = SherpaOnnxStt::with_options(
            Some("/usr/local/bin/sherpa-onnx-offline".into()),
            Some("~/.moltis/models/sherpa-onnx-whisper-tiny.en".into()),
            Some("en".into()),
        );
        assert_eq!(
            provider.binary_path,
            Some("/usr/local/bin/sherpa-onnx-offline".into())
        );
        assert_eq!(
            provider.model_dir,
            Some("~/.moltis/models/sherpa-onnx-whisper-tiny.en".into())
        );
        assert_eq!(provider.language, Some("en".into()));
    }

    #[tokio::test]
    async fn test_transcribe_without_config() {
        let provider = SherpaOnnxStt::new();
        let request = TranscribeRequest {
            audio: Bytes::from_static(b"fake audio"),
            format: AudioFormat::Mp3,
            language: None,
            prompt: None,
        };

        let result = provider.transcribe(request).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found") || err.contains("not configured"));
    }

    #[test]
    fn test_find_file_with_prefix() {
        // Test with a temp directory
        let temp_dir = tempfile::tempdir().unwrap();
        let dir_path = temp_dir.path().to_path_buf();

        // No files yet
        assert!(find_file_with_prefix(&dir_path, "encoder").is_none());

        // Create a matching file
        std::fs::write(dir_path.join("encoder.onnx"), b"test").unwrap();
        let found = find_file_with_prefix(&dir_path, "encoder");
        assert!(found.is_some());
        assert!(found.unwrap().ends_with("encoder.onnx"));

        // Non-matching prefix
        assert!(find_file_with_prefix(&dir_path, "decoder").is_none());
    }
}
