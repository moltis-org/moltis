//! Detection of an OpenAI Codex CLI installation.

use std::path::{Path, PathBuf};

use tracing::info;

/// Result of scanning for a Codex CLI installation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CodexDetection {
    /// Codex home directory (`~/.codex/`).
    pub home_dir: PathBuf,
    /// Config file (`~/.codex/config.toml`).
    pub config_path: Option<PathBuf>,
    /// Global instructions (`~/.codex/instructions.md`).
    pub instructions_path: Option<PathBuf>,
    /// Whether anything meaningful was found.
    pub has_data: bool,
}

/// Detect a Codex CLI installation.
///
/// Checks `CODEX_HOME` env var first, then `~/.codex/`.
pub fn detect() -> Option<CodexDetection> {
    let home = if let Ok(env_home) = std::env::var("CODEX_HOME") {
        PathBuf::from(env_home)
    } else {
        dirs_next::home_dir()?.join(".codex")
    };

    detect_at(&home)
}

/// Detect Codex at a specific directory (for testing).
pub fn detect_at(home_dir: &Path) -> Option<CodexDetection> {
    if !home_dir.is_dir() {
        info!(path = %home_dir.display(), "codex detect: directory does not exist");
        return None;
    }

    let config_path = check_file(&home_dir.join("config.toml"));
    let instructions_path = check_file(&home_dir.join("instructions.md"));

    let has_data = config_path.is_some() || instructions_path.is_some();

    if !has_data {
        info!(path = %home_dir.display(), "codex detect: no Codex data found");
        return None;
    }

    info!(
        path = %home_dir.display(),
        has_config = config_path.is_some(),
        has_instructions = instructions_path.is_some(),
        "codex detect: installation found"
    );

    Some(CodexDetection {
        home_dir: home_dir.to_path_buf(),
        config_path,
        instructions_path,
        has_data,
    })
}

fn check_file(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        Some(path.to_path_buf())
    } else {
        None
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn detect_at_nonexistent_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let codex = tmp.path().join(".codex");
        assert!(detect_at(&codex).is_none());
    }

    #[test]
    fn detect_at_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let codex = tmp.path().join(".codex");
        std::fs::create_dir_all(&codex).unwrap();
        assert!(detect_at(&codex).is_none());
    }

    #[test]
    fn detect_at_with_config() {
        let tmp = tempfile::tempdir().unwrap();
        let codex = tmp.path().join(".codex");
        std::fs::create_dir_all(&codex).unwrap();
        std::fs::write(codex.join("config.toml"), "[model]\nname = \"o3\"").unwrap();

        let detection = detect_at(&codex).unwrap();
        assert!(detection.config_path.is_some());
        assert!(detection.has_data);
    }

    #[test]
    fn detect_at_with_instructions() {
        let tmp = tempfile::tempdir().unwrap();
        let codex = tmp.path().join(".codex");
        std::fs::create_dir_all(&codex).unwrap();
        std::fs::write(codex.join("instructions.md"), "# Instructions\nBe helpful.").unwrap();

        let detection = detect_at(&codex).unwrap();
        assert!(detection.instructions_path.is_some());
        assert!(detection.has_data);
    }

    #[test]
    fn detect_at_with_both() {
        let tmp = tempfile::tempdir().unwrap();
        let codex = tmp.path().join(".codex");
        std::fs::create_dir_all(&codex).unwrap();
        std::fs::write(codex.join("config.toml"), "[model]\nname = \"o3\"").unwrap();
        std::fs::write(codex.join("instructions.md"), "# Instructions").unwrap();

        let detection = detect_at(&codex).unwrap();
        assert!(detection.config_path.is_some());
        assert!(detection.instructions_path.is_some());
    }
}
