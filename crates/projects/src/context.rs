use std::{
    fs,
    path::{Path, PathBuf},
};

use {anyhow::Result, tracing::info};

use crate::types::ContextFile;

/// Names of context files to collect when walking the directory hierarchy.
const CONTEXT_FILE_NAMES: &[&str] = &["CLAUDE.md", "CLAUDE.local.md", "AGENTS.md"];

/// Load all context files for a project directory.
///
/// Walks upward from `project_dir` to the filesystem root, collecting
/// `CLAUDE.md`, `CLAUDE.local.md`, and `AGENTS.md` at each level.
/// Also loads `.claude/rules/*.md` from `project_dir`.
///
/// Files are returned ordered outermost (root) first, innermost (project dir)
/// last, so that project-level files take highest priority when appended.
pub fn load_context_files(project_dir: &Path) -> Result<Vec<ContextFile>> {
    let project_dir = project_dir.canonicalize()?;
    let mut layers: Vec<Vec<ContextFile>> = Vec::new();

    // Walk upward from project dir to root
    let mut current = Some(project_dir.as_path());
    while let Some(dir) = current {
        let mut layer = Vec::new();
        for name in CONTEXT_FILE_NAMES {
            let file_path = dir.join(name);
            if file_path.is_file()
                && let Ok(content) = fs::read_to_string(&file_path)
                && !content.trim().is_empty()
            {
                info!(path = %file_path.display(), "loaded context file");
                layer.push(ContextFile {
                    path: file_path,
                    content,
                });
            }
        }
        if !layer.is_empty() {
            layers.push(layer);
        }
        current = dir.parent();
    }

    // Reverse so outermost comes first, innermost (project dir) last
    layers.reverse();
    let mut files: Vec<ContextFile> = layers.into_iter().flatten().collect();

    // Load .claude/rules/*.md from project root
    let rules_dir = project_dir.join(".claude").join("rules");
    if rules_dir.is_dir() {
        let mut rule_files: Vec<PathBuf> = fs::read_dir(&rules_dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "md"))
            .collect();
        rule_files.sort();
        for path in rule_files {
            if let Ok(content) = fs::read_to_string(&path)
                && !content.trim().is_empty()
            {
                info!(path = %path.display(), "loaded rule file");
                files.push(ContextFile { path, content });
            }
        }
    }

    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_context_files_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let files = load_context_files(dir.path()).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn test_load_claude_md() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("CLAUDE.md"), "# Project rules").unwrap();
        let files = load_context_files(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].path.ends_with("CLAUDE.md"));
        assert_eq!(files[0].content, "# Project rules");
    }

    #[test]
    fn test_load_agents_md() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("AGENTS.md"), "# Agents").unwrap();
        let files = load_context_files(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].path.ends_with("AGENTS.md"));
    }

    #[test]
    fn test_load_multiple_context_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("CLAUDE.md"), "claude").unwrap();
        fs::write(dir.path().join("CLAUDE.local.md"), "local").unwrap();
        fs::write(dir.path().join("AGENTS.md"), "agents").unwrap();
        let files = load_context_files(dir.path()).unwrap();
        assert_eq!(files.len(), 3);
    }

    #[test]
    fn test_load_rules_dir() {
        let dir = tempfile::tempdir().unwrap();
        let rules = dir.path().join(".claude").join("rules");
        fs::create_dir_all(&rules).unwrap();
        fs::write(rules.join("style.md"), "# Style guide").unwrap();
        fs::write(rules.join("security.md"), "# Security rules").unwrap();
        let files = load_context_files(dir.path()).unwrap();
        assert_eq!(files.len(), 2);
        // Should be sorted alphabetically
        assert!(files[0].path.ends_with("security.md"));
        assert!(files[1].path.ends_with("style.md"));
    }

    #[test]
    fn test_ignores_empty_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("CLAUDE.md"), "   \n  ").unwrap();
        let files = load_context_files(dir.path()).unwrap();
        assert!(files.is_empty());
    }
}
