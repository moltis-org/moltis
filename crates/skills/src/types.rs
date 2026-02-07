use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::formats::PluginFormat;

// ── Skills manifest ──────────────────────────────────────────────────────────

/// Top-level manifest tracking installed repos and per-skill enabled state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsManifest {
    pub version: u32,
    #[serde(default)]
    pub repos: Vec<RepoEntry>,
}

impl Default for SkillsManifest {
    fn default() -> Self {
        Self {
            version: 1,
            repos: Vec::new(),
        }
    }
}

impl SkillsManifest {
    pub fn add_repo(&mut self, entry: RepoEntry) {
        self.repos.push(entry);
    }

    pub fn remove_repo(&mut self, source: &str) {
        self.repos.retain(|r| r.source != source);
    }

    pub fn find_repo(&self, source: &str) -> Option<&RepoEntry> {
        self.repos.iter().find(|r| r.source == source)
    }

    pub fn find_repo_mut(&mut self, source: &str) -> Option<&mut RepoEntry> {
        self.repos.iter_mut().find(|r| r.source == source)
    }

    pub fn set_skill_enabled(&mut self, source: &str, skill_name: &str, enabled: bool) -> bool {
        if let Some(repo) = self.find_repo_mut(source)
            && let Some(skill) = repo.skills.iter_mut().find(|s| s.name == skill_name)
        {
            skill.enabled = enabled;
            return true;
        }
        false
    }

    pub fn set_skill_trusted(&mut self, source: &str, skill_name: &str, trusted: bool) -> bool {
        if let Some(repo) = self.find_repo_mut(source)
            && let Some(skill) = repo.skills.iter_mut().find(|s| s.name == skill_name)
        {
            skill.trusted = trusted;
            return true;
        }
        false
    }
}

/// A single cloned repository with its discovered skills.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoEntry {
    pub source: String,
    pub repo_name: String,
    pub installed_at_ms: u64,
    #[serde(default)]
    pub commit_sha: Option<String>,
    #[serde(default)]
    pub format: PluginFormat,
    pub skills: Vec<SkillState>,
}

/// Per-skill enabled state within a repo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillState {
    pub name: String,
    pub relative_path: String,
    #[serde(default = "default_trusted")]
    pub trusted: bool,
    pub enabled: bool,
}

fn default_trusted() -> bool {
    // Backward compatibility: manifests created before trust-gating should
    // continue to work without immediately disabling all installed skills.
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_state_defaults_trusted_for_backward_compat() {
        let parsed: SkillState = serde_json::from_str(
            r#"{"name":"demo","relative_path":"repo/skills/demo","enabled":true}"#,
        )
        .unwrap();
        assert!(parsed.trusted);
    }
}

// ── Skill metadata ───────────────────────────────────────────────────────────

/// Where a skill was discovered from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillSource {
    /// Project-local: `<cwd>/.moltis/skills/`
    Project,
    /// Personal: `~/.moltis/skills/`
    Personal,
    /// Bundled inside a plugin directory.
    Plugin,
    /// Installed from a registry (e.g. skills.sh).
    Registry,
}

/// Lightweight metadata parsed from SKILL.md frontmatter.
/// Loaded at startup for all discovered skills (cheap).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMetadata {
    /// Skill name — lowercase, hyphens allowed, 1-64 chars.
    pub name: String,
    /// Short human-readable description.
    #[serde(default)]
    pub description: String,
    /// Homepage URL.
    #[serde(default)]
    pub homepage: Option<String>,
    /// SPDX license identifier.
    #[serde(default)]
    pub license: Option<String>,
    /// Environment requirements (intended product, system packages, network access, etc.).
    #[serde(default)]
    pub compatibility: Option<String>,
    /// Tools this skill is allowed to use (space-delimited in spec, parsed as list).
    #[serde(default, alias = "allowed-tools")]
    pub allowed_tools: Vec<String>,
    /// Optional Dockerfile (relative to skill directory) for sandbox environment.
    #[serde(default)]
    pub dockerfile: Option<String>,
    /// Binary/tool requirements for this skill.
    #[serde(default)]
    pub requires: SkillRequirements,
    /// Filesystem path to the skill directory.
    #[serde(skip)]
    pub path: PathBuf,
    /// Where this skill was discovered.
    #[serde(skip)]
    pub source: Option<SkillSource>,
}

// ── Skill requirements ──────────────────────────────────────────────────────

/// Binary and tool requirements declared in SKILL.md frontmatter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillRequirements {
    /// All of these binaries must be found in PATH.
    #[serde(default)]
    pub bins: Vec<String>,
    /// At least one of these binaries must be found (openclaw `anyBins`).
    #[serde(default)]
    pub any_bins: Vec<String>,
    /// Install instructions for missing binaries.
    #[serde(default)]
    pub install: Vec<InstallSpec>,
}

/// How to install a missing binary dependency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallSpec {
    pub kind: InstallKind,
    #[serde(default)]
    pub formula: Option<String>,
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub module: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    /// Which binaries this install step provides.
    #[serde(default)]
    pub bins: Vec<String>,
    /// Platform filter (e.g. `["darwin"]`, `["linux"]`). Empty = all platforms.
    #[serde(default)]
    pub os: Vec<String>,
    #[serde(default)]
    pub label: Option<String>,
}

/// Install method kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallKind {
    Brew,
    Npm,
    Go,
    Cargo,
    Uv,
    Download,
}

/// Result of checking whether a skill's requirements are met.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillEligibility {
    pub eligible: bool,
    pub missing_bins: Vec<String>,
    /// Install options filtered to the current OS.
    pub install_options: Vec<InstallSpec>,
}

/// Full skill content: metadata + markdown body.
/// Loaded on demand when a skill is activated.
#[derive(Debug, Clone)]
pub struct SkillContent {
    pub metadata: SkillMetadata,
    pub body: String,
}
