//! Shared utilities for CLI-based STT providers.
//!
//! Provides common functionality for local STT tools like whisper.cpp
//! and sherpa-onnx that run as command-line processes.

use {
    anyhow::{Context, Result},
    std::path::PathBuf,
    tempfile::NamedTempFile,
};

use crate::tts::AudioFormat;

/// Find a binary in PATH or at a specific path.
///
/// If `config_path` is Some, it's checked first. If None or not found,
/// searches the system PATH.
pub fn find_binary(name: &str, config_path: Option<&str>) -> Option<PathBuf> {
    // Check explicit config path first
    if let Some(path_str) = config_path {
        let path = expand_tilde(path_str);
        if path.exists() && path.is_file() {
            return Some(path);
        }
    }

    // Search in PATH
    which::which(name).ok()
}

/// Expand `~` to the user's home directory.
pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(stripped);
    }
    PathBuf::from(path)
}

/// Write audio data to a temporary file for CLI processing.
///
/// Returns the temp file handle (keeps file alive) and its path.
pub fn write_temp_audio(audio: &[u8], format: AudioFormat) -> Result<(NamedTempFile, PathBuf)> {
    let ext = format.extension();
    let temp_file = NamedTempFile::with_suffix(format!(".{}", ext))
        .context("failed to create temp audio file")?;

    std::fs::write(temp_file.path(), audio).context("failed to write audio to temp file")?;

    let path = temp_file.path().to_path_buf();
    Ok((temp_file, path))
}

/// Check if a model file or directory exists.
#[allow(dead_code)]
pub fn model_exists(path_str: Option<&str>) -> bool {
    path_str.map(expand_tilde).is_some_and(|p| p.exists())
}

/// Verify a binary is executable.
#[allow(dead_code)]
pub fn is_binary_configured(binary_name: &str, config_path: Option<&str>) -> bool {
    find_binary(binary_name, config_path).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_tilde() {
        // Non-tilde paths should be unchanged
        assert_eq!(
            expand_tilde("/usr/bin/test"),
            PathBuf::from("/usr/bin/test")
        );
        assert_eq!(
            expand_tilde("relative/path"),
            PathBuf::from("relative/path")
        );

        // Tilde paths should expand (if home dir exists)
        if let Some(home) = dirs::home_dir() {
            assert_eq!(expand_tilde("~/test"), home.join("test"));
            assert_eq!(expand_tilde("~/.config/app"), home.join(".config/app"));
        }
    }

    #[test]
    fn test_find_binary_in_path() {
        // Common binaries that should exist
        assert!(find_binary("ls", None).is_some());

        // Non-existent binary
        assert!(find_binary("definitely-not-a-real-binary-xyz123", None).is_none());
    }

    #[test]
    fn test_write_temp_audio() {
        let audio = b"fake audio data";
        let result = write_temp_audio(audio, AudioFormat::Mp3);
        assert!(result.is_ok());

        let (_temp_file, path) = result.unwrap();
        assert!(path.exists());
        assert!(path.extension().is_some_and(|e| e == "mp3"));

        let contents = std::fs::read(&path).unwrap();
        assert_eq!(contents, audio);
    }

    #[test]
    fn test_model_exists() {
        // None should return false
        assert!(!model_exists(None));

        // Non-existent path should return false
        assert!(!model_exists(Some("/definitely/not/a/real/path")));

        // Existing path (use temp dir as a known path)
        let temp_dir = std::env::temp_dir();
        assert!(model_exists(Some(temp_dir.to_str().unwrap())));
    }
}
