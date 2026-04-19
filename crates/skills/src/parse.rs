use std::path::Path;

use serde::Deserialize;

use crate::error::{Error, Result};

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

/// When `name` fails validation, try to use `slug` (from frontmatter or `_meta.json`)
/// as the internal name, storing the original `name` as `display_name`.
fn resolve_name_or_slug(meta: &mut SkillMetadata, skill_dir: &Path) -> Result<()> {
    if validate_name(&meta.name) {
        return Ok(());
    }

    // Try slug from frontmatter first.
    let slug = meta.slug.clone().or_else(|| {
        // Fall back to slug from _meta.json.
        read_meta_json(skill_dir).and_then(|m| m.slug)
    });

    match slug {
        Some(ref s) if validate_name(s) => {
            tracing::debug!(
                name = %meta.name,
                slug = %s,
                "skill name invalid, using slug as internal name"
            );
            meta.display_name = Some(std::mem::take(&mut meta.name));
            meta.name = s.clone();
            // slug is intentionally left populated so callers can inspect what was in the frontmatter.
            Ok(())
        },
        Some(ref s) => {
            let source = if meta.slug.is_some() {
                "frontmatter"
            } else {
                "_meta.json"
            };
            return Err(Error::Validation(format!(
                "skill name '{}' is invalid and slug '{}' (from {}) is also invalid: \
                 must be 1-64 lowercase alphanumeric, hyphen, or colon chars \
                 (e.g. 'my-skill' or 'ns:skill')",
                meta.name, s, source
            )));
        },
        None => {
            return Err(Error::Validation(format!(
                "skill name '{}' is invalid and no slug provided: \
                 must be 1-64 lowercase alphanumeric, hyphen, or colon chars \
                 (e.g. 'my-skill' or 'ns:skill'), \
                 or provide a valid 'slug' field",
                meta.name
            )));
        },
    }
}

/// Parse a SKILL.md file into metadata only (frontmatter).
pub fn parse_metadata(content: &str, skill_dir: &Path) -> Result<SkillMetadata> {
    let (frontmatter, _body) = split_frontmatter(content)?;
    let mut meta: SkillMetadata = serde_yaml::from_str(&frontmatter)
        .map_err(|e| Error::Parse(format!("invalid SKILL.md frontmatter: {e}")))?;

    resolve_name_or_slug(&mut meta, skill_dir)?;

    merge_openclaw_requires(&frontmatter, &mut meta);
    meta.path = skill_dir.to_path_buf();
    Ok(meta)
}

/// Parse a SKILL.md file into full content (metadata + body).
pub fn parse_skill(content: &str, skill_dir: &Path) -> Result<SkillContent> {
    let (frontmatter, body) = split_frontmatter(content)?;
    let mut meta: SkillMetadata = serde_yaml::from_str(&frontmatter)
        .map_err(|e| Error::Parse(format!("invalid SKILL.md frontmatter: {e}")))?;

    resolve_name_or_slug(&mut meta, skill_dir)?;

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
fn split_frontmatter(content: &str) -> Result<(String, String)> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return Err(Error::Parse(
            "SKILL.md must start with YAML frontmatter delimited by ---".into(),
        ));
    }

    // Skip the opening ---
    let after_open = &trimmed[3..];
    let close_pos = after_open
        .find("\n---")
        .ok_or_else(|| Error::Parse("SKILL.md missing closing --- for frontmatter".into()))?;

    let frontmatter = after_open[..close_pos].trim().to_string();
    let body = after_open[close_pos + 4..].trim().to_string();
    Ok((frontmatter, body))
}

/// Tolerant variant of [`split_frontmatter`]: returns only the body after an
/// optional YAML frontmatter block. If the content doesn't start with a
/// `---` fence or the closing `---` is missing, returns the original content
/// unchanged.
///
/// Unlike `parse_skill`, this helper never errors and never validates the
/// frontmatter's schema — it's intended for consumers that just want a clean
/// markdown body without a full schema check (e.g. reading plugin-backed
/// skills whose frontmatter may follow a non-SKILL.md convention).
#[must_use]
pub fn strip_optional_frontmatter(content: &str) -> &str {
    let trimmed_start = content.trim_start();
    let Some(after_open) = trimmed_start.strip_prefix("---") else {
        return content;
    };
    let Some(close_pos) = after_open.find("\n---") else {
        return content;
    };
    // Advance past "\n---" and any trailing newline so the caller sees
    // clean markdown starting at the first real content line.
    let rest = &after_open[close_pos + 4..];
    rest.trim_start_matches(['\r', '\n'])
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    use rstest::rstest;

    #[test]
    fn strip_optional_frontmatter_removes_yaml_block() {
        let input = "---\nname: foo\ndescription: bar\n---\n\n# Body\n\nHello.\n";
        assert_eq!(strip_optional_frontmatter(input), "# Body\n\nHello.\n");
    }

    #[test]
    fn strip_optional_frontmatter_passes_through_plain_markdown() {
        let input = "# No frontmatter here\n\nJust body.\n";
        assert_eq!(strip_optional_frontmatter(input), input);
    }

    #[test]
    fn strip_optional_frontmatter_passes_through_unterminated_fence() {
        // Missing closing fence — don't eat the body silently, return as-is.
        let input = "---\nname: broken\nno closing fence here\n\n# Body that survives\n";
        assert_eq!(strip_optional_frontmatter(input), input);
    }

    #[test]
    fn strip_optional_frontmatter_handles_leading_whitespace() {
        let input = "\n\n---\nname: foo\n---\n# Body\n";
        assert_eq!(strip_optional_frontmatter(input), "# Body\n");
    }

    #[test]
    fn strip_optional_frontmatter_handles_empty_body() {
        let input = "---\nname: foo\n---\n";
        assert_eq!(strip_optional_frontmatter(input), "");
    }

    #[rstest]
    #[case("my-skill", true)]
    #[case("a", true)]
    #[case("skill123", true)]
    #[case("plugin:skill", true)]
    #[case("pr-review-toolkit:code-reviewer", true)]
    #[case("", false)]
    #[case("-bad", false)]
    #[case("bad-", false)]
    #[case("Bad", false)]
    #[case("has space", false)]
    #[case("has--double", false)]
    #[case(":bad", false)]
    #[case("bad:", false)]
    #[case("bad::double", false)]
    fn test_validate_name(#[case] name: &str, #[case] expected: bool) {
        assert_eq!(validate_name(name), expected, "validate_name({name:?})");
    }

    #[test]
    fn test_validate_name_too_long() {
        assert!(!validate_name(&"a".repeat(65)));
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
    fn test_invalid_name_no_slug_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let content = "---\nname: Bad-Name\n---\nbody\n";
        let err = parse_metadata(content, tmp.path()).unwrap_err();
        assert!(
            err.to_string().contains("no slug provided"),
            "error should mention missing slug: {err}"
        );
    }

    #[test]
    fn test_invalid_name_with_valid_slug_uses_slug() {
        let tmp = tempfile::tempdir().unwrap();
        let content = r#"---
name: "SEO (Site Audit + Content Writer + Competitor Analysis)"
slug: seo
description: SEO tools
---

Body.
"#;
        let meta = parse_metadata(content, tmp.path()).unwrap();
        assert_eq!(meta.name, "seo");
        assert_eq!(
            meta.display_name.as_deref(),
            Some("SEO (Site Audit + Content Writer + Competitor Analysis)")
        );
        assert_eq!(meta.description, "SEO tools");
    }

    #[test]
    fn test_invalid_name_with_invalid_slug_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let content = "---\nname: Bad Name\nslug: Also Bad\n---\nbody\n";
        let err = parse_metadata(content, tmp.path()).unwrap_err();
        assert!(
            err.to_string().contains("also invalid"),
            "error should mention both are invalid: {err}"
        );
    }

    #[test]
    fn test_valid_name_ignores_slug() {
        let content = "---\nname: my-skill\nslug: other\ndescription: test\n---\nbody\n";
        let meta = parse_metadata(content, Path::new("/tmp/my-skill")).unwrap();
        assert_eq!(meta.name, "my-skill");
        assert!(meta.display_name.is_none());
        assert_eq!(meta.slug, Some("other".into()));
    }

    #[test]
    fn test_parse_skill_slug_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let content = r#"---
name: "My Fancy Skill (v2)"
slug: fancy-skill
description: A fancy skill
---

Do fancy things.
"#;
        let skill = parse_skill(content, tmp.path()).unwrap();
        assert_eq!(skill.metadata.name, "fancy-skill");
        assert_eq!(
            skill.metadata.display_name.as_deref(),
            Some("My Fancy Skill (v2)")
        );
        assert!(skill.body.contains("fancy things"));
    }

    #[test]
    fn test_slug_fallback_from_meta_json() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("seo");
        std::fs::create_dir_all(&skill_dir).unwrap();

        // Write a _meta.json with slug
        std::fs::write(
            skill_dir.join("_meta.json"),
            r#"{"slug": "seo", "displayName": "SEO Tools", "owner": "test"}"#,
        )
        .unwrap();

        let content = r#"---
name: "SEO (Audit + Writer)"
description: SEO toolkit
---

Body.
"#;
        let meta = parse_metadata(content, &skill_dir).unwrap();
        assert_eq!(meta.name, "seo");
        assert_eq!(meta.display_name.as_deref(), Some("SEO (Audit + Writer)"));
        // Slug came from _meta.json, not frontmatter, so meta.slug stays None.
        assert!(
            meta.slug.is_none(),
            "slug comes from _meta.json, not frontmatter, so meta.slug should remain None"
        );
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
