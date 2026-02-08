//! Repo format detection and adapters.
//!
//! Different AI coding tools use different layouts for their plugin/skill repos.
//! This module detects the format and normalizes repo contents into
//! `SkillMetadata` + body pairs that feed into the skills system.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::types::{SkillMetadata, SkillRequirements, SkillSource};

// ── Plugin format enum ──────────────────────────────────────────────────────

/// Detected format of a plugin/skill repository.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginFormat {
    /// Native `SKILL.md` format (single or multi-skill repo).
    #[default]
    Skill,
    /// Claude Code plugin: `.claude-plugin/plugin.json` + `agents/`, `commands/`, `skills/` dirs.
    ClaudeCode,
    /// Codex plugin: `codex-plugin.json` or `.codex/plugin.json` (future).
    Codex,
    /// Fallback: `.md` files treated as generic skill prompts.
    Generic,
}

impl std::fmt::Display for PluginFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Skill => write!(f, "skill"),
            Self::ClaudeCode => write!(f, "claude_code"),
            Self::Codex => write!(f, "codex"),
            Self::Generic => write!(f, "generic"),
        }
    }
}

// ── Plugin skill entry ──────────────────────────────────────────────────────

/// A single skill entry scanned from a non-SKILL.md repo, with extra metadata
/// beyond what `SkillMetadata` carries.
#[derive(Debug, Clone, Serialize)]
pub struct PluginSkillEntry {
    pub metadata: SkillMetadata,
    pub body: String,
    /// Human-friendly display name (e.g. "Code Reviewer" for `code-reviewer`).
    pub display_name: Option<String>,
    /// Plugin author (from plugin.json).
    pub author: Option<String>,
    /// Relative path of the source `.md` file within the repo (e.g. `agents/code-reviewer.md`).
    pub source_file: Option<String>,
}

// ── Format adapter trait ────────────────────────────────────────────────────

/// A format adapter normalizes a non-SKILL.md repo into skill entries.
pub trait FormatAdapter: Send + Sync {
    /// Check whether the given repo directory matches this format.
    fn detect(&self, repo_dir: &Path) -> bool;

    /// Scan the repo and return enriched entries for each skill found.
    fn scan_skills(&self, repo_dir: &Path) -> anyhow::Result<Vec<PluginSkillEntry>>;
}

// ── Claude Code adapter ─────────────────────────────────────────────────────

/// Claude Code plugin metadata from `.claude-plugin/plugin.json`.
#[derive(Debug, Deserialize)]
struct ClaudePluginJson {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    author: Option<PluginAuthor>,
}

/// Author field can be a string or an object with `name` (and optionally `email`).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PluginAuthor {
    Simple(String),
    Object { name: String },
}

impl PluginAuthor {
    fn name(&self) -> &str {
        match self {
            Self::Simple(s) => s,
            Self::Object { name } => name,
        }
    }
}

/// Adapter for Claude Code plugin repos.
pub struct ClaudeCodeAdapter;

impl ClaudeCodeAdapter {
    /// Scan a single plugin directory (one that has `.claude-plugin/plugin.json`).
    /// `repo_root` is the top-level repo directory; `source_file` paths are
    /// computed relative to it so that GitHub URLs work for marketplace repos.
    fn scan_single_plugin(
        &self,
        plugin_dir: &Path,
        repo_root: &Path,
    ) -> anyhow::Result<Vec<PluginSkillEntry>> {
        let plugin_json_path = plugin_dir.join(".claude-plugin/plugin.json");
        let plugin_json: ClaudePluginJson =
            serde_json::from_str(&std::fs::read_to_string(&plugin_json_path)?)?;

        let plugin_name = &plugin_json.name;
        let author = plugin_json.author.as_ref().map(|a| a.name().to_string());
        let mut results = Vec::new();

        // Scan agents/, commands/, skills/ directories for .md files.
        for subdir in &["agents", "commands", "skills"] {
            let dir = plugin_dir.join(subdir);
            if !dir.is_dir() {
                continue;
            }
            let entries = match std::fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let ext = path.extension().and_then(|e| e.to_str());
                if ext != Some("md") {
                    continue;
                }
                let stem = match path.file_stem().and_then(|s| s.to_str()) {
                    Some(s) => s.to_string(),
                    None => continue,
                };

                let body = match std::fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(?path, %e, "failed to read plugin skill file");
                        continue;
                    },
                };

                // Extract description from first non-empty line of body.
                let description = body
                    .lines()
                    .find(|l| {
                        let trimmed = l.trim();
                        !trimmed.is_empty() && !trimmed.starts_with('#')
                    })
                    .unwrap_or("")
                    .trim()
                    .chars()
                    .take(120)
                    .collect::<String>();

                let namespaced_name = format!("{plugin_name}:{stem}");

                // Build display name from stem: "code-reviewer" → "Code Reviewer"
                let display_name = stem
                    .split('-')
                    .map(|w| {
                        let mut c = w.chars();
                        match c.next() {
                            Some(first) => first.to_uppercase().to_string() + c.as_str(),
                            None => String::new(),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");

                // Relative path within repo root (e.g. "plugins/pr-review-toolkit/agents/code-reviewer.md")
                let source_file = path
                    .strip_prefix(repo_root)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string());

                let meta = SkillMetadata {
                    name: namespaced_name,
                    description: if description.is_empty() {
                        plugin_json.description.clone().unwrap_or_default()
                    } else {
                        description
                    },
                    homepage: author.as_ref().map(|a| format!("https://github.com/{a}")),
                    license: None,
                    compatibility: None,
                    allowed_tools: Vec::new(),
                    requires: SkillRequirements::default(),
                    path: path.parent().unwrap_or(plugin_dir).to_path_buf(),
                    source: Some(SkillSource::Plugin),
                    dockerfile: None,
                };

                results.push(PluginSkillEntry {
                    metadata: meta,
                    body,
                    display_name: Some(display_name),
                    author: author.clone(),
                    source_file,
                });
            }
        }

        Ok(results)
    }
}

impl FormatAdapter for ClaudeCodeAdapter {
    fn detect(&self, repo_dir: &Path) -> bool {
        // Single plugin: .claude-plugin/plugin.json at root
        // Marketplace repo: .claude-plugin/marketplace.json at root
        repo_dir.join(".claude-plugin/plugin.json").is_file()
            || repo_dir.join(".claude-plugin/marketplace.json").is_file()
    }

    fn scan_skills(&self, repo_dir: &Path) -> anyhow::Result<Vec<PluginSkillEntry>> {
        // Single plugin case
        if repo_dir.join(".claude-plugin/plugin.json").is_file() {
            return self.scan_single_plugin(repo_dir, repo_dir);
        }

        // Marketplace repo: scan plugins/ and external_plugins/ subdirs
        let mut results = Vec::new();
        for container in &["plugins", "external_plugins"] {
            let container_dir = repo_dir.join(container);
            if !container_dir.is_dir() {
                continue;
            }
            let entries = match std::fs::read_dir(&container_dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                if !path.join(".claude-plugin/plugin.json").is_file() {
                    continue;
                }
                match self.scan_single_plugin(&path, repo_dir) {
                    Ok(skills) => results.extend(skills),
                    Err(e) => {
                        tracing::warn!(?path, %e, "failed to scan sub-plugin");
                    },
                }
            }
        }

        Ok(results)
    }
}

// ── Format detection ────────────────────────────────────────────────────────

/// All known format adapters, in detection priority order.
fn adapters() -> Vec<(PluginFormat, Box<dyn FormatAdapter>)> {
    vec![(PluginFormat::ClaudeCode, Box::new(ClaudeCodeAdapter))]
}

/// Detect the format of a repository.
pub fn detect_format(repo_dir: &Path) -> PluginFormat {
    for (format, adapter) in adapters() {
        if adapter.detect(repo_dir) {
            return format;
        }
    }

    // Check for native SKILL.md.
    if repo_dir.join("SKILL.md").is_file() || has_skill_md_recursive(repo_dir) {
        return PluginFormat::Skill;
    }

    PluginFormat::Generic
}

/// Scan a repo using the detected format adapter.
/// Returns `None` for `Skill` format (caller should use existing SKILL.md scanning).
pub fn scan_with_adapter(
    repo_dir: &Path,
    format: PluginFormat,
) -> Option<anyhow::Result<Vec<PluginSkillEntry>>> {
    match format {
        PluginFormat::Skill => None, // handled by existing scan_repo_skills
        PluginFormat::ClaudeCode => Some(ClaudeCodeAdapter.scan_skills(repo_dir)),
        PluginFormat::Codex => None, // not yet implemented
        PluginFormat::Generic => None,
    }
}

/// Check if there's at least one SKILL.md in subdirectories.
fn has_skill_md_recursive(dir: &Path) -> bool {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return false,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.join("SKILL.md").is_file() {
                return true;
            }
            if has_skill_md_recursive(&path) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_format_display() {
        assert_eq!(PluginFormat::Skill.to_string(), "skill");
        assert_eq!(PluginFormat::ClaudeCode.to_string(), "claude_code");
        assert_eq!(PluginFormat::Codex.to_string(), "codex");
        assert_eq!(PluginFormat::Generic.to_string(), "generic");
    }

    #[test]
    fn test_detect_skill_format_root() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("SKILL.md"), "---\nname: test\n---\nbody").unwrap();
        assert_eq!(detect_format(tmp.path()), PluginFormat::Skill);
    }

    #[test]
    fn test_detect_skill_format_subdir() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("mysub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("SKILL.md"), "---\nname: test\n---\nbody").unwrap();
        assert_eq!(detect_format(tmp.path()), PluginFormat::Skill);
    }

    #[test]
    fn test_detect_claude_code_format() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = tmp.path().join(".claude-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.json"),
            r#"{"name":"test-plugin","description":"A test"}"#,
        )
        .unwrap();
        assert_eq!(detect_format(tmp.path()), PluginFormat::ClaudeCode);
    }

    #[test]
    fn test_detect_generic_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("README.md"), "hello").unwrap();
        assert_eq!(detect_format(tmp.path()), PluginFormat::Generic);
    }

    #[test]
    fn test_claude_code_adapter_scan() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Create plugin structure
        std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
        std::fs::write(
            root.join(".claude-plugin/plugin.json"),
            r#"{"name":"pr-review-toolkit","description":"PR review tools","author":"anthropics"}"#,
        )
        .unwrap();

        std::fs::create_dir_all(root.join("agents")).unwrap();
        std::fs::write(
            root.join("agents/code-reviewer.md"),
            "Use this agent when you need to review code.\n\nDetailed instructions here.",
        )
        .unwrap();

        std::fs::create_dir_all(root.join("commands")).unwrap();
        std::fs::write(
            root.join("commands/review-pr.md"),
            "# Review PR\n\nReview the current pull request.",
        )
        .unwrap();

        let adapter = ClaudeCodeAdapter;
        assert!(adapter.detect(root));

        let results = adapter.scan_skills(root).unwrap();
        assert_eq!(results.len(), 2);

        let names: Vec<&str> = results.iter().map(|e| e.metadata.name.as_str()).collect();
        assert!(names.contains(&"pr-review-toolkit:code-reviewer"));
        assert!(names.contains(&"pr-review-toolkit:review-pr"));

        // Check source is Plugin
        for entry in &results {
            assert_eq!(entry.metadata.source, Some(SkillSource::Plugin));
            assert_eq!(entry.author.as_deref(), Some("anthropics"));
        }

        // Check display_name and source_file
        let reviewer = results
            .iter()
            .find(|e| e.metadata.name == "pr-review-toolkit:code-reviewer")
            .unwrap();
        assert_eq!(reviewer.display_name.as_deref(), Some("Code Reviewer"));
        assert_eq!(
            reviewer.source_file.as_deref(),
            Some("agents/code-reviewer.md")
        );

        let review_pr = results
            .iter()
            .find(|e| e.metadata.name == "pr-review-toolkit:review-pr")
            .unwrap();
        assert_eq!(review_pr.display_name.as_deref(), Some("Review Pr"));
        assert_eq!(
            review_pr.source_file.as_deref(),
            Some("commands/review-pr.md")
        );
    }

    #[test]
    fn test_claude_code_adapter_empty_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
        std::fs::write(
            root.join(".claude-plugin/plugin.json"),
            r#"{"name":"empty-plugin"}"#,
        )
        .unwrap();

        let adapter = ClaudeCodeAdapter;
        let results = adapter.scan_skills(root).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_claude_code_adapter_skips_non_md() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
        std::fs::write(
            root.join(".claude-plugin/plugin.json"),
            r#"{"name":"test-plugin"}"#,
        )
        .unwrap();

        std::fs::create_dir_all(root.join("agents")).unwrap();
        std::fs::write(root.join("agents/readme.txt"), "not a skill").unwrap();
        std::fs::write(root.join("agents/real.md"), "A real skill agent.").unwrap();

        let adapter = ClaudeCodeAdapter;
        let results = adapter.scan_skills(root).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].metadata.name, "test-plugin:real");
    }

    #[test]
    fn test_detect_claude_code_marketplace_format() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin_dir = tmp.path().join(".claude-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("marketplace.json"),
            r#"{"name":"marketplace","plugins":[]}"#,
        )
        .unwrap();
        assert_eq!(detect_format(tmp.path()), PluginFormat::ClaudeCode);
    }

    #[test]
    fn test_claude_code_marketplace_scan() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // marketplace.json at root
        std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
        std::fs::write(
            root.join(".claude-plugin/marketplace.json"),
            r#"{"name":"marketplace"}"#,
        )
        .unwrap();

        // Sub-plugin in plugins/
        let p1 = root.join("plugins/my-plugin");
        std::fs::create_dir_all(p1.join(".claude-plugin")).unwrap();
        std::fs::write(
            p1.join(".claude-plugin/plugin.json"),
            r#"{"name":"my-plugin","description":"A plugin"}"#,
        )
        .unwrap();
        std::fs::create_dir_all(p1.join("commands")).unwrap();
        std::fs::write(p1.join("commands/do-thing.md"), "Do the thing.").unwrap();

        // Sub-plugin in external_plugins/
        let p2 = root.join("external_plugins/ext-plugin");
        std::fs::create_dir_all(p2.join(".claude-plugin")).unwrap();
        std::fs::write(
            p2.join(".claude-plugin/plugin.json"),
            r#"{"name":"ext-plugin"}"#,
        )
        .unwrap();
        std::fs::create_dir_all(p2.join("agents")).unwrap();
        std::fs::write(p2.join("agents/helper.md"), "A helper agent.").unwrap();

        // Dir without plugin.json should be skipped
        let p3 = root.join("plugins/no-plugin");
        std::fs::create_dir_all(&p3).unwrap();
        std::fs::write(p3.join("README.md"), "no plugin").unwrap();

        let adapter = ClaudeCodeAdapter;
        assert!(adapter.detect(root));

        let results = adapter.scan_skills(root).unwrap();
        assert_eq!(results.len(), 2);

        let names: Vec<&str> = results.iter().map(|e| e.metadata.name.as_str()).collect();
        assert!(names.contains(&"my-plugin:do-thing"));
        assert!(names.contains(&"ext-plugin:helper"));

        // source_file should be relative to repo root, not the sub-plugin dir
        let do_thing = results
            .iter()
            .find(|e| e.metadata.name == "my-plugin:do-thing")
            .unwrap();
        assert_eq!(
            do_thing.source_file.as_deref(),
            Some("plugins/my-plugin/commands/do-thing.md")
        );

        let helper = results
            .iter()
            .find(|e| e.metadata.name == "ext-plugin:helper")
            .unwrap();
        assert_eq!(
            helper.source_file.as_deref(),
            Some("external_plugins/ext-plugin/agents/helper.md")
        );
    }

    #[test]
    fn test_scan_with_adapter_skill_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(scan_with_adapter(tmp.path(), PluginFormat::Skill).is_none());
    }

    #[test]
    fn test_scan_with_adapter_claude_code() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
        std::fs::write(
            root.join(".claude-plugin/plugin.json"),
            r#"{"name":"my-plugin"}"#,
        )
        .unwrap();
        std::fs::create_dir_all(root.join("skills")).unwrap();
        std::fs::write(root.join("skills/do-thing.md"), "Do the thing.").unwrap();

        let result = scan_with_adapter(root, PluginFormat::ClaudeCode);
        assert!(result.is_some());
        let skills = result.unwrap().unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].metadata.name, "my-plugin:do-thing");
    }
}
