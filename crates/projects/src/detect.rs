use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use tracing::debug;

use crate::types::Project;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Derive a human-friendly project label from a directory.
///
/// Resolution order:
/// 1. `Cargo.toml` package name
/// 2. `package.json` name
/// 3. Git remote origin name (last path segment)
/// 4. Directory name
pub fn derive_label(dir: &Path) -> String {
    // Try Cargo.toml
    if let Some(name) = cargo_name(dir) {
        return name;
    }
    // Try package.json
    if let Some(name) = package_json_name(dir) {
        return name;
    }
    // Try git remote
    if let Some(name) = git_remote_name(dir) {
        return name;
    }
    // Fallback to directory name
    dir.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

fn cargo_name(dir: &Path) -> Option<String> {
    let path = dir.join("Cargo.toml");
    let content = fs::read_to_string(path).ok()?;
    let table: toml::Table = toml::from_str(&content).ok()?;
    table
        .get("package")?
        .get("name")?
        .as_str()
        .map(String::from)
}

fn package_json_name(dir: &Path) -> Option<String> {
    let path = dir.join("package.json");
    let content = fs::read_to_string(path).ok()?;
    let val: serde_json::Value = serde_json::from_str(&content).ok()?;
    val.get("name")?.as_str().map(String::from)
}

fn git_remote_name(dir: &Path) -> Option<String> {
    let config_path = dir.join(".git").join("config");
    let content = fs::read_to_string(config_path).ok()?;
    // Simple parse: find url = ... line under [remote "origin"]
    let mut in_origin = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("[remote") && trimmed.contains("\"origin\"") {
            in_origin = true;
            continue;
        }
        if trimmed.starts_with('[') {
            in_origin = false;
            continue;
        }
        if in_origin
            && trimmed.starts_with("url")
            && let Some(url) = trimmed.split('=').nth(1)
        {
            let url = url.trim();
            // Extract repo name from URL like git@...:user/repo.git or https://.../repo.git
            let name = url
                .rsplit('/')
                .next()
                .or_else(|| url.rsplit(':').next())
                .unwrap_or(url)
                .trim_end_matches(".git");
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// Derive a slug-friendly ID from a directory path.
pub fn derive_id(dir: &Path) -> String {
    dir.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != '-', "-")
}

/// Check if a directory is a git repository.
pub fn is_git_repo(dir: &Path) -> bool {
    dir.join(".git").exists()
}

/// Detect a project from a directory. Returns `None` if directory doesn't exist.
pub fn detect_project(dir: &Path) -> Option<Project> {
    if !dir.is_dir() {
        return None;
    }
    let now = now_ms();
    let label = derive_label(dir);
    let id = derive_id(dir);
    debug!(dir = %dir.display(), label = %label, "detected project");
    Some(Project {
        id,
        label,
        directory: dir.to_path_buf(),
        system_prompt: None,
        auto_worktree: is_git_repo(dir),
        setup_command: None,
        teardown_command: None,
        branch_prefix: None,
        sandbox_image: None,
        detected: true,
        created_at: now,
        updated_at: now,
    })
}

/// Well-known parent directories where projects typically live.
const SCAN_PARENTS: &[&str] = &[
    "Projects",
    "Developer",
    "src",
    "code",
    "repos",
    "workspace",
    "dev",
    "git",
];

/// Return the immediate children of well-known project parent directories
/// under the user's home folder. Only existing directories are included.
pub fn default_scan_dirs() -> Vec<PathBuf> {
    let Ok(home) = std::env::var("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    scan_dirs_from_home(&home)
}

/// Scan well-known project parent directories under the given home path.
///
/// Extracted from [`default_scan_dirs`] so tests can pass a custom home
/// directory without mutating the process environment.
fn scan_dirs_from_home(home: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for parent_name in SCAN_PARENTS {
        let parent = home.join(parent_name);
        if let Ok(entries) = fs::read_dir(&parent) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    out.push(path);
                }
            }
        }
    }
    // Superset worktrees: ~/.superset/worktrees/<project>/<branch>/
    let superset_wt = home.join(".superset").join("worktrees");
    if let Ok(projects) = fs::read_dir(&superset_wt) {
        for project in projects.flatten() {
            if project.path().is_dir()
                && let Ok(branches) = fs::read_dir(project.path())
            {
                for branch in branches.flatten() {
                    let path = branch.path();
                    if path.is_dir() {
                        out.push(path);
                    }
                }
            }
        }
    }
    out
}

/// Scan a list of directories and detect projects from git repos.
/// Returns new projects not already in `known_ids`.
pub fn auto_detect(dirs: &[&Path], known_ids: &[String]) -> Vec<Project> {
    let mut detected = Vec::new();
    for dir in dirs {
        if !is_git_repo(dir) {
            continue;
        }
        if let Some(project) = detect_project(dir)
            && !known_ids.contains(&project.id)
        {
            detected.push(project);
        }
    }
    detected
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_label_cargo() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"my-crate\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        assert_eq!(derive_label(dir.path()), "my-crate");
    }

    #[test]
    fn test_derive_label_package_json() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("package.json"), r#"{"name": "my-app"}"#).unwrap();
        assert_eq!(derive_label(dir.path()), "my-app");
    }

    #[test]
    fn test_derive_label_fallback_to_dir_name() {
        let dir = tempfile::tempdir().unwrap();
        let label = derive_label(dir.path());
        // tempdir names are random but non-empty
        assert!(!label.is_empty());
    }

    #[test]
    fn test_is_git_repo() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_git_repo(dir.path()));
        fs::create_dir(dir.path().join(".git")).unwrap();
        assert!(is_git_repo(dir.path()));
    }

    #[test]
    fn test_detect_project() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"cool-proj\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let p = detect_project(dir.path()).unwrap();
        assert_eq!(p.label, "cool-proj");
        assert!(p.auto_worktree);
        assert!(p.detected);
    }

    #[test]
    fn test_derive_id() {
        let id = derive_id(Path::new("/home/user/My Project!"));
        assert_eq!(id, "my-project-");
    }

    #[test]
    fn test_default_scan_dirs_with_projects_folder() {
        let home = tempfile::tempdir().unwrap();
        let projects = home.path().join("Projects");
        fs::create_dir(&projects).unwrap();
        let repo_a = projects.join("repo-a");
        fs::create_dir(&repo_a).unwrap();
        let repo_b = projects.join("repo-b");
        fs::create_dir(&repo_b).unwrap();
        // Also create a file — should be ignored.
        fs::write(projects.join("not-a-dir.txt"), "hi").unwrap();

        let dirs = scan_dirs_from_home(home.path());
        assert!(dirs.contains(&repo_a));
        assert!(dirs.contains(&repo_b));
        assert!(!dirs.iter().any(|p| p.ends_with("not-a-dir.txt")));
    }

    #[test]
    fn test_default_scan_dirs_no_matching_parents() {
        let home = tempfile::tempdir().unwrap();
        let dirs = scan_dirs_from_home(home.path());
        assert!(dirs.is_empty());
    }
}
