//! Bundled skills embedded in the binary at compile time.
//!
//! Skills live in `crates/skills/src/assets/<category>/<name>/SKILL.md` and are
//! committed to the repository. In dev mode (`cargo run`) the module reads
//! directly from the filesystem for instant iteration; in release builds it
//! serves from the [`include_dir!`] embedded copy.
//!
//! This mirrors the three-tier asset strategy in `crates/web/src/assets.rs`.

use std::path::{Path, PathBuf};

use crate::{
    parse,
    types::{SkillMetadata, SkillSource},
};

// ── Embedded assets ─────────────────────────────────────────────────────────

static BUNDLED_ASSETS: include_dir::Dir<'static> =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/src/assets");

// ── Asset source resolution ─────────────────────────────────────────────────

enum AssetSource {
    /// Read from the filesystem (dev mode: `cargo run`).
    Filesystem(PathBuf),
    /// Read from the compile-time embedded directory.
    Embedded,
}

/// Store for bundled skills. Shared (via `Arc`) between the composite
/// discoverer and the `ReadSkillTool`.
pub struct BundledSkillStore {
    source: AssetSource,
}

impl BundledSkillStore {
    /// Create a new store, preferring the filesystem in dev mode.
    #[must_use]
    pub fn new() -> Self {
        let cargo_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/assets");
        let source = if cargo_dir.is_dir() {
            tracing::debug!(path = %cargo_dir.display(), "bundled skills: using filesystem (dev mode)");
            AssetSource::Filesystem(cargo_dir)
        } else {
            tracing::debug!("bundled skills: using embedded assets");
            AssetSource::Embedded
        };
        Self { source }
    }

    /// Discover metadata for all bundled skills.
    ///
    /// Walks the assets directory two levels deep (`<category>/<skill>/SKILL.md`),
    /// parses frontmatter, and tags each with [`SkillSource::Bundled`].
    pub fn discover(&self) -> Vec<SkillMetadata> {
        match &self.source {
            AssetSource::Filesystem(dir) => discover_from_fs(dir),
            AssetSource::Embedded => discover_from_embedded(),
        }
    }

    /// Read the full body of a bundled skill by name.
    pub fn read_skill(&self, name: &str) -> Option<String> {
        match &self.source {
            AssetSource::Filesystem(dir) => read_skill_body_fs(dir, name),
            AssetSource::Embedded => read_skill_body_embedded(name),
        }
    }

    /// Read a sidecar file from a bundled skill directory.
    ///
    /// Returns `Some((bytes, is_utf8))` or `None` if the file does not exist.
    pub fn read_sidecar(&self, name: &str, rel_path: &str) -> Option<(Vec<u8>, bool)> {
        match &self.source {
            AssetSource::Filesystem(dir) => read_sidecar_fs(dir, name, rel_path),
            AssetSource::Embedded => read_sidecar_embedded(name, rel_path),
        }
    }

    /// List sidecar files for a bundled skill (references/, templates/, etc.).
    pub fn list_sidecars(&self, name: &str) -> Vec<(String, u64)> {
        match &self.source {
            AssetSource::Filesystem(dir) => list_sidecars_fs(dir, name),
            AssetSource::Embedded => list_sidecars_embedded(name),
        }
    }
}

impl Default for BundledSkillStore {
    fn default() -> Self {
        Self::new()
    }
}

// ── Filesystem (dev mode) ───────────────────────────────────────────────────

/// Recursively walk the assets directory for SKILL.md files on the filesystem.
/// Supports arbitrary nesting (e.g. `mlops/training/axolotl/SKILL.md`).
fn discover_from_fs(assets_dir: &Path) -> Vec<SkillMetadata> {
    let mut skills = Vec::new();
    discover_from_fs_recursive(assets_dir, assets_dir, &mut skills);
    skills
}

fn discover_from_fs_recursive(assets_root: &Path, dir: &Path, skills: &mut Vec<SkillMetadata>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let skill_md = path.join("SKILL.md");
        if skill_md.is_file() {
            let Ok(content) = std::fs::read_to_string(&skill_md) else {
                continue;
            };
            match parse::parse_metadata(&content, &path) {
                Ok(mut meta) => {
                    meta.source = Some(SkillSource::Bundled);
                    meta.category = category_from_path(assets_root, &path);
                    skills.push(meta);
                },
                Err(e) => {
                    tracing::warn!(path = %skill_md.display(), %e, "failed to parse bundled SKILL.md");
                },
            }
        } else {
            // No SKILL.md here — recurse into subdirectories (category nesting).
            discover_from_fs_recursive(assets_root, &path, skills);
        }
    }
}

/// Extract the top-level category from a skill's path relative to the assets root.
/// e.g. `assets/research/arxiv` → `"research"`, `assets/mlops/training/axolotl` → `"mlops"`.
fn category_from_path(assets_root: &Path, skill_dir: &Path) -> Option<String> {
    let rel = skill_dir.strip_prefix(assets_root).ok()?;
    let first_component = rel.components().next()?;
    Some(first_component.as_os_str().to_string_lossy().into_owned())
}

/// Read SKILL.md body from the filesystem.
fn read_skill_body_fs(assets_dir: &Path, name: &str) -> Option<String> {
    let skill_dir = find_skill_dir_fs(assets_dir, name)?;
    let content = std::fs::read_to_string(skill_dir.join("SKILL.md")).ok()?;
    let skill = parse::parse_skill(&content, &skill_dir).ok()?;
    Some(skill.body)
}

fn read_sidecar_fs(assets_dir: &Path, name: &str, rel_path: &str) -> Option<(Vec<u8>, bool)> {
    let skill_dir = find_skill_dir_fs(assets_dir, name)?;
    let target = skill_dir.join(rel_path);
    // Basic traversal check.
    if !target.starts_with(&skill_dir) {
        return None;
    }
    let bytes = std::fs::read(&target).ok()?;
    let is_utf8 = std::str::from_utf8(&bytes).is_ok();
    Some((bytes, is_utf8))
}

fn list_sidecars_fs(assets_dir: &Path, name: &str) -> Vec<(String, u64)> {
    let Some(skill_dir) = find_skill_dir_fs(assets_dir, name) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for sub in crate::SIDECAR_SUBDIRS {
        let dir = skill_dir.join(sub);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.path().is_file() {
                let file_name = entry.file_name().to_string_lossy().to_string();
                let bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
                out.push((format!("{sub}/{file_name}"), bytes));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Recursively find a skill directory by name under the assets tree.
fn find_skill_dir_fs(dir: &Path, name: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if dir_name == name && path.join("SKILL.md").is_file() {
            return Some(path);
        }
        // Recurse into subdirectories (category nesting).
        if let Some(found) = find_skill_dir_fs(&path, name) {
            return Some(found);
        }
    }
    None
}

// ── Embedded (release mode) ─────────────────────────────────────────────────

/// Find a file by name among the *direct* files of an embedded directory.
///
/// `include_dir` stores file paths relative to the root of the `include_dir!`
/// tree, not relative to the containing `Dir`. So `dir.get_file("SKILL.md")`
/// fails on subdirectories because it compares `"SKILL.md"` against full paths
/// like `"research/arxiv/SKILL.md"`. This helper matches on the file-name
/// component instead.
fn dir_file_by_name<'a>(
    dir: &'a include_dir::Dir<'static>,
    name: &str,
) -> Option<&'a include_dir::File<'static>> {
    dir.files()
        .find(|f| f.path().file_name().and_then(|n| n.to_str()) == Some(name))
}

/// Recursively walk the embedded `include_dir!` tree for SKILL.md files.
fn discover_from_embedded() -> Vec<SkillMetadata> {
    let mut skills = Vec::new();
    discover_from_embedded_recursive(&BUNDLED_ASSETS, &mut skills);
    skills
}

fn discover_from_embedded_recursive(
    dir: &include_dir::Dir<'static>,
    skills: &mut Vec<SkillMetadata>,
) {
    for sub_dir in dir.dirs() {
        if let Some(skill_md) = dir_file_by_name(sub_dir, "SKILL.md") {
            let Ok(content) = std::str::from_utf8(skill_md.contents()) else {
                continue;
            };
            let synthetic_path =
                PathBuf::from("__bundled__").join(sub_dir.path().to_string_lossy().as_ref());
            match parse::parse_metadata(content, &synthetic_path) {
                Ok(mut meta) => {
                    meta.source = Some(SkillSource::Bundled);
                    // Extract category from first path component (e.g. "research/arxiv" → "research").
                    meta.category = sub_dir
                        .path()
                        .components()
                        .next()
                        .and_then(|c| c.as_os_str().to_str())
                        .map(String::from);
                    skills.push(meta);
                },
                Err(e) => {
                    tracing::warn!(
                        path = %sub_dir.path().display(),
                        %e,
                        "failed to parse embedded bundled SKILL.md"
                    );
                },
            }
        } else {
            // No SKILL.md here — recurse into subdirectories.
            discover_from_embedded_recursive(sub_dir, skills);
        }
    }
}

/// Read SKILL.md body from the embedded directory.
fn read_skill_body_embedded(name: &str) -> Option<String> {
    let skill_dir = find_skill_dir_embedded(name)?;
    let skill_md = dir_file_by_name(skill_dir, "SKILL.md")?;
    let content = std::str::from_utf8(skill_md.contents()).ok()?;
    let synthetic_path =
        PathBuf::from("__bundled__").join(skill_dir.path().to_string_lossy().as_ref());
    let skill = parse::parse_skill(content, &synthetic_path).ok()?;
    Some(skill.body)
}

fn read_sidecar_embedded(name: &str, rel_path: &str) -> Option<(Vec<u8>, bool)> {
    let skill_dir = find_skill_dir_embedded(name)?;
    // Sidecar paths like "references/foo.md" — match by the full relative tail.
    let file = skill_dir.files().find(|f| {
        f.path()
            .strip_prefix(skill_dir.path())
            .is_ok_and(|rel| rel == Path::new(rel_path))
    })?;
    let bytes = file.contents().to_vec();
    let is_utf8 = std::str::from_utf8(&bytes).is_ok();
    Some((bytes, is_utf8))
}

fn list_sidecars_embedded(name: &str) -> Vec<(String, u64)> {
    let Some(skill_dir) = find_skill_dir_embedded(name) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for sub in crate::SIDECAR_SUBDIRS {
        // Use get_dir with the full path since include_dir stores full paths.
        let sub_path = skill_dir.path().join(sub);
        let Some(sub_dir) = skill_dir.dirs().find(|d| d.path() == sub_path) else {
            continue;
        };
        for file in sub_dir.files() {
            let file_name = file
                .path()
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            out.push((format!("{sub}/{file_name}"), file.contents().len() as u64));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Recursively find a skill subdirectory by name in the embedded tree.
fn find_skill_dir_embedded(name: &str) -> Option<&'static include_dir::Dir<'static>> {
    find_skill_dir_embedded_recursive(&BUNDLED_ASSETS, name)
}

fn find_skill_dir_embedded_recursive(
    dir: &'static include_dir::Dir<'static>,
    name: &str,
) -> Option<&'static include_dir::Dir<'static>> {
    for sub_dir in dir.dirs() {
        let dir_name = sub_dir
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if dir_name == name && dir_file_by_name(sub_dir, "SKILL.md").is_some() {
            return Some(sub_dir);
        }
        if let Some(found) = find_skill_dir_embedded_recursive(sub_dir, name) {
            return Some(found);
        }
    }
    None
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> BundledSkillStore {
        BundledSkillStore::new()
    }

    // ── Embedded assets ─────────────────────────────────────────────────

    #[test]
    fn embedded_assets_have_skills() {
        // Verify the include_dir! embedded data has skills, independent of
        // the filesystem fallback. This catches build issues where the
        // embedded assets are silently empty at runtime.
        let skills = discover_from_embedded();
        assert!(
            skills.len() >= 90,
            "expected ≥90 embedded skills, got {}",
            skills.len()
        );
        for skill in &skills {
            assert!(
                skill.category.is_some(),
                "embedded skill '{}' has no category",
                skill.name
            );
        }
    }

    #[test]
    fn embedded_skill_body_is_readable() {
        let skills = discover_from_embedded();
        assert!(!skills.is_empty(), "no embedded skills to test");
        let first = &skills[0];
        let body = read_skill_body_embedded(&first.name);
        assert!(
            body.is_some(),
            "embedded skill '{}' body not readable via embedded path",
            first.name
        );
    }

    // ── Discovery ───────────────────────────────────────────────────────

    #[test]
    fn bundled_skills_are_discovered() {
        let skills = store().discover();
        assert!(
            skills.len() >= 90,
            "expected ≥90 bundled skills, got {}",
            skills.len()
        );
        for skill in &skills {
            assert_eq!(skill.source, Some(SkillSource::Bundled));
            assert!(!skill.name.is_empty(), "skill has empty name");
            assert!(
                !skill.description.is_empty(),
                "skill {} has empty description",
                skill.name
            );
        }
    }

    #[test]
    fn no_duplicate_skill_names() {
        let skills = store().discover();
        let mut seen = std::collections::HashSet::new();
        for skill in &skills {
            assert!(
                seen.insert(&skill.name),
                "duplicate bundled skill name: {}",
                skill.name
            );
        }
    }

    #[test]
    fn all_names_pass_validation() {
        let skills = store().discover();
        for skill in &skills {
            assert!(
                parse::validate_name(&skill.name),
                "skill name '{}' fails validation",
                skill.name
            );
        }
    }

    // ── Category ────────────────────────────────────────────────────────

    #[test]
    fn every_bundled_skill_has_category() {
        let skills = store().discover();
        for skill in &skills {
            assert!(
                skill.category.is_some(),
                "skill '{}' has no category",
                skill.name
            );
            assert!(
                !skill.category.as_ref().is_none_or(String::is_empty),
                "skill '{}' has empty category",
                skill.name
            );
        }
    }

    #[test]
    fn known_categories_present() {
        let skills = store().discover();
        let cats: std::collections::HashSet<String> =
            skills.iter().filter_map(|s| s.category.clone()).collect();
        // These categories must exist (from both hermes and openclaw copies).
        for expected in [
            "research",
            "creative",
            "mlops",
            "software-development",
            "productivity",
        ] {
            assert!(
                cats.contains(expected),
                "expected category '{}' not found in {:?}",
                expected,
                cats
            );
        }
    }

    #[test]
    fn category_derived_from_top_level_directory() {
        let skills = store().discover();
        // axolotl lives at mlops/training/axolotl — category should be "mlops"
        let axolotl = skills.iter().find(|s| s.name == "axolotl");
        if let Some(skill) = axolotl {
            assert_eq!(skill.category.as_deref(), Some("mlops"));
        }
        // arxiv lives at research/arxiv — category should be "research"
        let arxiv = skills.iter().find(|s| s.name == "arxiv");
        if let Some(skill) = arxiv {
            assert_eq!(skill.category.as_deref(), Some("research"));
        }
    }

    // ── Origin ──────────────────────────────────────────────────────────

    #[test]
    fn all_bundled_skills_have_origin() {
        let skills = store().discover();
        for skill in &skills {
            assert!(
                skill.origin.is_some(),
                "skill '{}' has no origin metadata",
                skill.name
            );
        }
    }

    #[test]
    fn origin_sources_are_known() {
        let skills = store().discover();
        let sources: std::collections::HashSet<String> = skills
            .iter()
            .filter_map(|s| s.origin.as_ref()?.source.clone())
            .collect();
        // All skills should come from one of our vetted sources.
        for source in &sources {
            assert!(
                source == "hermes-agent" || source == "openclaw",
                "unexpected origin source: '{}'",
                source
            );
        }
    }

    // ── Content reading ─────────────────────────────────────────────────

    #[test]
    fn every_bundled_skill_body_is_readable() {
        let s = store();
        let skills = s.discover();
        for skill in &skills {
            let body = s.read_skill(&skill.name);
            assert!(body.is_some(), "skill '{}' body not readable", skill.name);
            assert!(
                !body.as_ref().is_none_or(String::is_empty),
                "skill '{}' has empty body",
                skill.name
            );
        }
    }

    #[test]
    fn missing_skill_returns_none() {
        assert!(store().read_skill("nonexistent-skill-xyz").is_none());
    }

    #[test]
    fn missing_sidecar_returns_none() {
        assert!(store().read_sidecar("arxiv", "nonexistent.md").is_none());
    }

    // ── Specific skills smoke tests ─────────────────────────────────────

    #[test]
    fn arxiv_skill_metadata() {
        let skills = store().discover();
        let arxiv = skills
            .iter()
            .find(|s| s.name == "arxiv")
            .expect("arxiv should be bundled");
        assert_eq!(arxiv.category.as_deref(), Some("research"));
        assert!(arxiv.description.contains("arXiv"));
        assert_eq!(
            arxiv.origin.as_ref().and_then(|o| o.source.as_deref()),
            Some("hermes-agent")
        );
    }

    #[test]
    fn weather_skill_metadata() {
        let skills = store().discover();
        let weather = skills
            .iter()
            .find(|s| s.name == "weather")
            .expect("weather should be bundled");
        assert_eq!(weather.category.as_deref(), Some("smart-home"));
        assert_eq!(
            weather.origin.as_ref().and_then(|o| o.source.as_deref()),
            Some("openclaw")
        );
    }

    #[test]
    fn himalaya_has_requires() {
        let skills = store().discover();
        let himalaya = skills
            .iter()
            .find(|s| s.name == "himalaya")
            .expect("himalaya should be bundled");
        assert!(
            himalaya.requires.bins.contains(&"himalaya".to_string()),
            "himalaya should require the himalaya binary"
        );
        assert!(
            !himalaya.requires.install.is_empty(),
            "himalaya should have install instructions"
        );
    }

    #[test]
    fn webhook_subscriptions_is_moltis_native() {
        let s = store();
        let body = s.read_skill("webhook-subscriptions").expect("should exist");
        // The rewritten skill should reference Moltis RPC, not Hermes CLI.
        assert!(
            body.contains("webhooks.create"),
            "webhook skill should reference Moltis RPC API"
        );
        assert!(
            !body.contains("hermes webhook"),
            "webhook skill should not reference Hermes CLI"
        );
    }
}
