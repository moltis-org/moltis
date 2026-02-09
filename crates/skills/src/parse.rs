use std::path::Path;

use {
    anyhow::{Context, bail},
    serde::Deserialize,
};

use crate::types::{InstallKind, InstallSpec, SkillContent, SkillMetadata};

/// Validate a skill name: lowercase ASCII, hyphens, 1-64 chars.
pub fn validate_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == ':')
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.starts_with(':')
        && !name.ends_with(':')
        && !name.contains("--")
        && !name.contains("::")
}

/// Parse a SKILL.md file into metadata only (frontmatter).
pub fn parse_metadata(content: &str, skill_dir: &Path) -> anyhow::Result<SkillMetadata> {
    let (frontmatter, _body) = split_frontmatter(content)?;
    let mut meta: SkillMetadata =
        serde_yaml::from_str(&frontmatter).context("invalid SKILL.md frontmatter")?;

    if !validate_name(&meta.name) {
        bail!(
            "invalid skill name '{}': must be 1-64 lowercase alphanumeric/hyphen chars",
            meta.name
        );
    }

    merge_openclaw_requires(&frontmatter, &mut meta);
    meta.path = skill_dir.to_path_buf();
    Ok(meta)
}

/// Parse a SKILL.md file into full content (metadata + body).
pub fn parse_skill(content: &str, skill_dir: &Path) -> anyhow::Result<SkillContent> {
    let (frontmatter, body) = split_frontmatter(content)?;
    let mut meta: SkillMetadata =
        serde_yaml::from_str(&frontmatter).context("invalid SKILL.md frontmatter")?;

    if !validate_name(&meta.name) {
        bail!(
            "invalid skill name '{}': must be 1-64 lowercase alphanumeric/hyphen chars",
            meta.name
        );
    }

    merge_openclaw_requires(&frontmatter, &mut meta);
    meta.path = skill_dir.to_path_buf();
    Ok(SkillContent {
        metadata: meta,
        body: body.to_string(),
    })
}

// ── OpenClaw metadata extraction ────────────────────────────────────────────

/// Helper struct to extract `metadata.openclaw.requires` and `metadata.openclaw.install`.
#[derive(Deserialize, Default)]
struct OpenClawRoot {
    #[serde(default)]
    metadata: Option<OpenClawMetadataWrap>,
}

#[derive(Deserialize, Default)]
struct OpenClawMetadataWrap {
    /// Our own namespace.
    #[serde(default)]
    openclaw: Option<OpenClawMeta>,
    /// Original openclaw/clawdbot namespace.
    #[serde(default)]
    clawdbot: Option<OpenClawMeta>,
    /// Moltbot namespace (some openclaw skills use this).
    #[serde(default)]
    moltbot: Option<OpenClawMeta>,
}

#[derive(Deserialize, Default)]
struct OpenClawMeta {
    #[serde(default)]
    requires: Option<OpenClawRequires>,
    #[serde(default)]
    install: Vec<OpenClawInstallSpec>,
}

#[derive(Deserialize, Default)]
struct OpenClawRequires {
    #[serde(default)]
    bins: Vec<String>,
    #[serde(default, rename = "anyBins")]
    any_bins: Vec<String>,
}

#[derive(Deserialize)]
struct OpenClawInstallSpec {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    formula: Option<String>,
    #[serde(default)]
    package: Option<String>,
    /// openclaw uses `pkg` for go/cargo installs.
    #[serde(default)]
    pkg: Option<String>,
    #[serde(default, rename = "module")]
    module_path: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    bins: Vec<String>,
    #[serde(default)]
    os: Vec<String>,
    #[serde(default)]
    label: Option<String>,
}

/// If the top-level `requires` is empty but `metadata.openclaw.requires`/`install` exist,
/// merge them into `SkillMetadata.requires`.
fn merge_openclaw_requires(frontmatter: &str, meta: &mut SkillMetadata) {
    // Only merge if top-level requires is empty
    if !meta.requires.bins.is_empty()
        || !meta.requires.any_bins.is_empty()
        || !meta.requires.install.is_empty()
    {
        return;
    }

    let root: OpenClawRoot = match serde_yaml::from_str(frontmatter) {
        Ok(r) => r,
        Err(_) => return,
    };

    let oc = match root
        .metadata
        .and_then(|m| m.openclaw.or(m.clawdbot).or(m.moltbot))
    {
        Some(oc) => oc,
        None => return,
    };

    if let Some(req) = oc.requires {
        meta.requires.bins = req.bins;
        meta.requires.any_bins = req.any_bins;
    }

    fn parse_kind(s: &str) -> Option<InstallKind> {
        match s {
            "brew" => Some(InstallKind::Brew),
            "npm" => Some(InstallKind::Npm),
            "go" => Some(InstallKind::Go),
            "cargo" => Some(InstallKind::Cargo),
            "uv" => Some(InstallKind::Uv),
            "download" => Some(InstallKind::Download),
            _ => None,
        }
    }

    for spec in oc.install {
        if let Some(kind) = parse_kind(&spec.kind) {
            meta.requires.install.push(InstallSpec {
                kind: kind.clone(),
                formula: spec.formula,
                package: spec.package.or_else(|| {
                    if kind == InstallKind::Npm || kind == InstallKind::Cargo {
                        spec.pkg.clone()
                    } else {
                        None
                    }
                }),
                module: spec.module_path.or_else(|| {
                    if kind == InstallKind::Go {
                        spec.pkg.clone()
                    } else {
                        None
                    }
                }),
                url: spec.url,
                bins: spec.bins,
                os: spec.os,
                label: spec.label,
            });
        }
    }
}

// ── _meta.json support (openclaw) ───────────────────────────────────────────

/// Metadata from an openclaw `_meta.json` file (sibling to SKILL.md).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SkillMetaJson {
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default, rename = "displayName")]
    pub display_name: Option<String>,
    #[serde(default)]
    pub latest: Option<SkillMetaVersion>,
}

/// Version info from `_meta.json`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SkillMetaVersion {
    #[serde(default)]
    pub version: Option<String>,
}

/// Try to read and parse `_meta.json` from a skill directory.
/// Returns `None` if the file doesn't exist or can't be parsed.
pub fn read_meta_json(skill_dir: &Path) -> Option<SkillMetaJson> {
    let path = skill_dir.join("_meta.json");
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Split SKILL.md content at `---` delimiters into (frontmatter, body).
fn split_frontmatter(content: &str) -> anyhow::Result<(String, String)> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        bail!("SKILL.md must start with YAML frontmatter delimited by ---");
    }

    // Skip the opening ---
    let after_open = &trimmed[3..];
    let close_pos = after_open
        .find("\n---")
        .context("SKILL.md missing closing --- for frontmatter")?;

    let frontmatter = after_open[..close_pos].trim().to_string();
    let body = after_open[close_pos + 4..].trim().to_string();
    Ok((frontmatter, body))
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_name() {
        assert!(validate_name("my-skill"));
        assert!(validate_name("a"));
        assert!(validate_name("skill123"));
        assert!(!validate_name(""));
        assert!(!validate_name("-bad"));
        assert!(!validate_name("bad-"));
        assert!(!validate_name("Bad"));
        assert!(!validate_name("has space"));
        assert!(!validate_name("has--double"));
        assert!(!validate_name(&"a".repeat(65)));
        // Colons allowed for namespaced plugin names
        assert!(validate_name("plugin:skill"));
        assert!(validate_name("pr-review-toolkit:code-reviewer"));
        assert!(!validate_name(":bad"));
        assert!(!validate_name("bad:"));
        assert!(!validate_name("bad::double"));
    }

    #[test]
    fn test_parse_metadata() {
        let content = r#"---
name: my-skill
description: A test skill
license: MIT
allowed_tools:
  - exec
  - read
---

# My Skill

Instructions here.
"#;
        let meta = parse_metadata(content, Path::new("/tmp/my-skill")).unwrap();
        assert_eq!(meta.name, "my-skill");
        assert_eq!(meta.description, "A test skill");
        assert_eq!(meta.license, Some("MIT".into()));
        assert_eq!(meta.allowed_tools, vec!["exec", "read"]);
        assert_eq!(meta.path, Path::new("/tmp/my-skill"));
    }

    #[test]
    fn test_parse_skill_full() {
        let content = r#"---
name: commit
description: Create git commits
---

When asked to commit, run `git add` then `git commit`.
"#;
        let skill = parse_skill(content, Path::new("/skills/commit")).unwrap();
        assert_eq!(skill.metadata.name, "commit");
        assert!(skill.body.contains("git add"));
    }

    #[test]
    fn test_invalid_name_rejected() {
        let content = "---\nname: Bad-Name\n---\nbody\n";
        assert!(parse_metadata(content, Path::new("/tmp")).is_err());
    }

    #[test]
    fn test_missing_frontmatter() {
        let content = "# No frontmatter\nJust markdown.";
        assert!(parse_metadata(content, Path::new("/tmp")).is_err());
    }

    #[test]
    fn test_missing_closing_delimiter() {
        let content = "---\nname: test\nno closing\n";
        assert!(parse_metadata(content, Path::new("/tmp")).is_err());
    }

    #[test]
    fn test_top_level_requires() {
        let content = r#"---
name: songsee
description: Generate spectrograms
requires:
  bins: [songsee]
  install:
    - kind: brew
      formula: songsee
      os: [darwin]
---

Instructions.
"#;
        let meta = parse_metadata(content, Path::new("/tmp/songsee")).unwrap();
        assert_eq!(meta.requires.bins, vec!["songsee"]);
        assert_eq!(meta.requires.install.len(), 1);
        assert_eq!(meta.requires.install[0].kind, InstallKind::Brew);
        assert_eq!(meta.requires.install[0].formula.as_deref(), Some("songsee"));
        assert_eq!(meta.requires.install[0].os, vec!["darwin"]);
    }

    #[test]
    fn test_openclaw_metadata_requires() {
        let content = r#"---
name: himalaya
description: CLI email client
metadata:
  openclaw:
    requires:
      bins: [himalaya]
    install:
      - kind: brew
        formula: himalaya
        bins: [himalaya]
        label: "Install Himalaya (brew)"
---

Instructions.
"#;
        let meta = parse_metadata(content, Path::new("/tmp/himalaya")).unwrap();
        assert_eq!(meta.requires.bins, vec!["himalaya"]);
        assert_eq!(meta.requires.install.len(), 1);
        assert_eq!(meta.requires.install[0].kind, InstallKind::Brew);
        assert_eq!(
            meta.requires.install[0].label.as_deref(),
            Some("Install Himalaya (brew)")
        );
    }

    #[test]
    fn test_top_level_requires_takes_precedence_over_openclaw() {
        let content = r#"---
name: test-skill
description: test
requires:
  bins: [mytool]
metadata:
  openclaw:
    requires:
      bins: [othertool]
---

Body.
"#;
        let meta = parse_metadata(content, Path::new("/tmp/test")).unwrap();
        // Top-level requires should be kept, openclaw not merged
        assert_eq!(meta.requires.bins, vec!["mytool"]);
    }

    #[test]
    fn test_clawdbot_metadata_requires() {
        // Real openclaw format: metadata is single-line JSON with "clawdbot" key
        let content = r#"---
name: beeper
description: Search and browse local Beeper chat history
metadata: {"clawdbot":{"requires":{"bins":["beeper-cli"]},"install":[{"id":"go","kind":"go","pkg":"github.com/krausefx/beeper-cli/cmd/beeper-cli","bins":["beeper-cli"],"label":"Install beeper-cli (go install)"}]}}
---

Instructions.
"#;
        let meta = parse_metadata(content, Path::new("/tmp/beeper")).unwrap();
        assert_eq!(meta.requires.bins, vec!["beeper-cli"]);
        assert_eq!(meta.requires.install.len(), 1);
        assert_eq!(meta.requires.install[0].kind, InstallKind::Go);
        // pkg should be mapped to module for go installs
        assert_eq!(
            meta.requires.install[0].module.as_deref(),
            Some("github.com/krausefx/beeper-cli/cmd/beeper-cli")
        );
        assert_eq!(
            meta.requires.install[0].label.as_deref(),
            Some("Install beeper-cli (go install)")
        );
    }

    #[test]
    fn test_compatibility_field() {
        let content = r#"---
name: docker-skill
description: Runs containers
compatibility: Requires docker and network access
---

Body.
"#;
        let meta = parse_metadata(content, Path::new("/tmp/docker-skill")).unwrap();
        assert_eq!(
            meta.compatibility.as_deref(),
            Some("Requires docker and network access")
        );
    }

    #[test]
    fn test_allowed_tools_hyphenated() {
        let content = "---\nname: git-skill\ndescription: Git helper\nallowed-tools:\n  - Bash(git:*)\n  - Read\n---\nBody.\n";
        let meta = parse_metadata(content, Path::new("/tmp/git-skill")).unwrap();
        assert_eq!(meta.allowed_tools, vec!["Bash(git:*)", "Read"]);
    }

    #[test]
    fn test_dockerfile_field() {
        let content = r#"---
name: docker-skill
description: Needs a custom image
dockerfile: Dockerfile
---

Body.
"#;
        let meta = parse_metadata(content, Path::new("/tmp/docker-skill")).unwrap();
        assert_eq!(meta.dockerfile.as_deref(), Some("Dockerfile"));
    }

    #[test]
    fn test_dockerfile_field_absent() {
        let content = "---\nname: simple\ndescription: no docker\n---\nBody.\n";
        let meta = parse_metadata(content, Path::new("/tmp/simple")).unwrap();
        assert!(meta.dockerfile.is_none());
    }

    #[test]
    fn test_no_requires_is_default() {
        let content = "---\nname: simple\ndescription: no deps\n---\nBody.\n";
        let meta = parse_metadata(content, Path::new("/tmp/simple")).unwrap();
        assert!(meta.requires.bins.is_empty());
        assert!(meta.requires.any_bins.is_empty());
        assert!(meta.requires.install.is_empty());
    }
}
