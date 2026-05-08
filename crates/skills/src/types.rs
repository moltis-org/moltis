use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize};

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
    #[serde(default)]
    pub quarantined: bool,
    #[serde(default)]
    pub quarantine_reason: Option<String>,
    #[serde(default)]
    pub provenance: Option<RepoProvenance>,
    pub skills: Vec<SkillState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoProvenance {
    pub original_source: String,
    #[serde(default)]
    pub original_commit_sha: Option<String>,
    #[serde(default)]
    pub imported_from: Option<String>,
    #[serde(default)]
    pub exported_at_ms: Option<u64>,
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

#[allow(clippy::unwrap_used, clippy::expect_used)]
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
    /// Project-local: `<data_dir>/.moltis/skills/`
    Project,
    /// Personal: `<data_dir>/skills/`
    Personal,
    /// Bundled inside a plugin directory.
    Plugin,
    /// Installed from a registry (e.g. skills.sh).
    Registry,
    /// Embedded in the binary at compile time from `crates/skills/src/assets/`.
    Bundled,
}

/// Lightweight metadata parsed from SKILL.md frontmatter.
/// Loaded at startup for all discovered skills (cheap).
///
/// `Default::default()` leaves `name` as `""` (invalid per `validate_name`).
/// Always initialise `name` explicitly, e.g.
/// `SkillMetadata { name: "my-skill".into(), ..Default::default() }`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillMetadata {
    /// Internal skill name — lowercase, hyphens allowed, 1-64 chars.
    /// When frontmatter `name` is human-readable (e.g. "SEO (Audit + Writer)"),
    /// this is populated from `slug` instead, and the original is stored in `display_name`.
    pub name: String,
    /// Optional slug from frontmatter; used as internal name when `name` fails validation.
    #[serde(default)]
    pub slug: Option<String>,
    /// Human-readable display name, set when `name` was swapped with `slug`.
    #[serde(default)]
    pub display_name: Option<String>,
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
    /// Tools this skill is allowed to use.
    ///
    /// Accepts either a YAML sequence or a space-separated string
    /// (Claude Code format: `"Bash(gh:*) Bash(git:*) Read"`).
    #[serde(
        default,
        alias = "allowed-tools",
        deserialize_with = "deserialize_string_or_seq"
    )]
    pub allowed_tools: Vec<String>,
    /// Optional Dockerfile (relative to skill directory) for sandbox environment.
    #[serde(default)]
    pub dockerfile: Option<String>,
    /// Binary/tool requirements for this skill.
    #[serde(default)]
    pub requires: SkillRequirements,
    /// Provenance of a bundled or imported skill (upstream repo, commit, date).
    #[serde(default)]
    pub origin: Option<SkillOrigin>,
    /// Category for grouping in the UI (e.g. "research", "creative", "mlops").
    /// Derived from the parent directory name for bundled skills.
    #[serde(skip)]
    pub category: Option<String>,
    /// Filesystem path to the skill directory.
    #[serde(skip)]
    pub path: PathBuf,
    /// Where this skill was discovered.
    #[serde(skip)]
    pub source: Option<SkillSource>,
}

// ── Skill origin ────────────────────────────────────────────────────────────

/// Provenance information for a skill copied from an external source.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillOrigin {
    /// Upstream project name (e.g. `"hermes-agent"`).
    #[serde(default)]
    pub source: Option<String>,
    /// URL of the upstream repository.
    #[serde(default)]
    pub url: Option<String>,
    /// Commit SHA or version tag at which the skill was copied.
    #[serde(default)]
    pub version: Option<String>,
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

/// Deserialize a value that can be either a YAML sequence of strings or a
/// single space-separated string (Claude Code format).
fn deserialize_string_or_seq<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrSeq {
        Seq(Vec<String>),
        Str(String),
    }

    match StringOrSeq::deserialize(deserializer)? {
        StringOrSeq::Seq(v) => Ok(v),
        StringOrSeq::Str(s) => Ok(s.split_whitespace().map(String::from).collect()),
    }
}

/// Full skill content: metadata + markdown body.
/// Loaded on demand when a skill is activated.
#[derive(Debug, Clone)]
pub struct SkillContent {
    pub metadata: SkillMetadata,
    pub body: String,
}
