//! Detection of a Hermes installation.

use std::path::{Path, PathBuf};

use tracing::info;

/// Result of scanning for a Hermes installation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HermesDetection {
    /// Root directory (`~/.hermes/` or `HERMES_HOME`).
    pub home_dir: PathBuf,
    /// Whether `config.yaml` exists.
    pub config_path: Option<PathBuf>,
    /// Whether `.env` exists (credentials).
    pub env_path: Option<PathBuf>,
    /// Skills directory.
    pub skills_dir: Option<PathBuf>,
    /// Memory file paths.
    pub soul_path: Option<PathBuf>,
    pub agents_path: Option<PathBuf>,
    pub memory_path: Option<PathBuf>,
    pub user_path: Option<PathBuf>,
    /// Whether anything meaningful was found.
    pub has_data: bool,
}

/// Detect a Hermes installation.
///
/// Checks `HERMES_HOME` env var first, then `~/.hermes/`.
pub fn detect() -> Option<HermesDetection> {
    let home = if let Ok(env_home) = std::env::var("HERMES_HOME") {
        PathBuf::from(env_home)
    } else {
        dirs_next::home_dir()?.join(".hermes")
    };

    detect_at(&home)
}

/// Detect Hermes at a specific directory (for testing).
pub fn detect_at(home_dir: &Path) -> Option<HermesDetection> {
    if !home_dir.is_dir() {
        info!(path = %home_dir.display(), "hermes detect: directory does not exist");
        return None;
    }

    let config_path = check_file(&home_dir.join("config.yaml"));
    let env_path = check_file(&home_dir.join(".env"));
    let skills_dir = check_dir(&home_dir.join("skills"));
    let soul_path = check_file(&home_dir.join("SOUL.md"));
    let agents_path = check_file(&home_dir.join("AGENTS.md"));
    let memory_path = check_file(&home_dir.join("memories").join("MEMORY.md"))
        .or_else(|| check_file(&home_dir.join("MEMORY.md")));
    let user_path = check_file(&home_dir.join("memories").join("USER.md"))
        .or_else(|| check_file(&home_dir.join("USER.md")));

    let has_data = config_path.is_some()
        || env_path.is_some()
        || skills_dir.is_some()
        || soul_path.is_some()
        || memory_path.is_some();

    if !has_data {
        info!(path = %home_dir.display(), "hermes detect: no Hermes data found");
        return None;
    }

    info!(
        path = %home_dir.display(),
        has_config = config_path.is_some(),
        has_env = env_path.is_some(),
        has_skills = skills_dir.is_some(),
        has_memory = memory_path.is_some(),
        "hermes detect: installation found"
    );

    Some(HermesDetection {
        home_dir: home_dir.to_path_buf(),
        config_path,
        env_path,
        skills_dir,
        soul_path,
        agents_path,
        memory_path,
        user_path,
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

fn check_dir(path: &Path) -> Option<PathBuf> {
    if path.is_dir() {
        Some(path.to_path_buf())
    } else {
        None
    }
}

/// Summary of what data is available for import from Hermes.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HermesScan {
    pub has_config: bool,
    pub has_credentials: bool,
    pub skills_count: usize,
    pub memory_files_count: usize,
    pub memory_files: Vec<String>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn detect_at_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let hermes = tmp.path().join(".hermes");
        // Directory doesn't exist
        assert!(detect_at(&hermes).is_none());
    }

    #[test]
    fn detect_at_empty_hermes_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let hermes = tmp.path().join(".hermes");
        std::fs::create_dir_all(&hermes).unwrap();
        // Empty directory — no data
        assert!(detect_at(&hermes).is_none());
    }

    #[test]
    fn detect_at_with_env() {
        let tmp = tempfile::tempdir().unwrap();
        let hermes = tmp.path().join(".hermes");
        std::fs::create_dir_all(&hermes).unwrap();
        std::fs::write(hermes.join(".env"), "OPENAI_API_KEY=sk-test").unwrap();

        let detection = detect_at(&hermes).unwrap();
        assert!(detection.env_path.is_some());
        assert!(detection.has_data);
    }

    #[test]
    fn detect_at_with_config_and_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let hermes = tmp.path().join(".hermes");
        std::fs::create_dir_all(hermes.join("skills").join("my-skill")).unwrap();
        std::fs::write(hermes.join("config.yaml"), "providers: {}").unwrap();

        let detection = detect_at(&hermes).unwrap();
        assert!(detection.config_path.is_some());
        assert!(detection.skills_dir.is_some());
    }

    #[test]
    fn detect_at_with_memory_files() {
        let tmp = tempfile::tempdir().unwrap();
        let hermes = tmp.path().join(".hermes");
        std::fs::create_dir_all(hermes.join("memories")).unwrap();
        std::fs::write(hermes.join("SOUL.md"), "# Soul").unwrap();
        std::fs::write(hermes.join("memories").join("MEMORY.md"), "# Memory").unwrap();
        std::fs::write(hermes.join("memories").join("USER.md"), "# User").unwrap();

        let detection = detect_at(&hermes).unwrap();
        assert!(detection.soul_path.is_some());
        assert!(detection.memory_path.is_some());
        assert!(detection.user_path.is_some());
    }
}
