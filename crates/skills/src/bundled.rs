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
        if let Some(skill_md) = sub_dir.get_file("SKILL.md") {
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
    let skill_md = skill_dir.get_file("SKILL.md")?;
    let content = std::str::from_utf8(skill_md.contents()).ok()?;
    let synthetic_path =
        PathBuf::from("__bundled__").join(skill_dir.path().to_string_lossy().as_ref());
    let skill = parse::parse_skill(content, &synthetic_path).ok()?;
    Some(skill.body)
}

fn read_sidecar_embedded(name: &str, rel_path: &str) -> Option<(Vec<u8>, bool)> {
    let skill_dir = find_skill_dir_embedded(name)?;
    let file = skill_dir.get_file(rel_path)?;
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
        let Some(sub_dir) = skill_dir.get_dir(sub) else {
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
        if dir_name == name && sub_dir.get_file("SKILL.md").is_some() {
            return Some(sub_dir);
        }
        if let Some(found) = find_skill_dir_embedded_recursive(sub_dir, name) {
            return Some(found);
        }
    }
    None
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_skills_are_discovered() {
        let store = BundledSkillStore::new();
        let skills = store.discover();
        assert!(
            !skills.is_empty(),
            "bundled skills directory should contain at least one skill"
        );
        for skill in &skills {
            assert_eq!(skill.source, Some(SkillSource::Bundled));
            assert!(!skill.name.is_empty());
            assert!(!skill.description.is_empty());
        }
    }

    #[test]
    fn bundled_skill_content_readable() {
        let store = BundledSkillStore::new();
        let skills = store.discover();
        let first = skills.first().expect("need at least one bundled skill");
        let body = store.read_skill(&first.name);
        assert!(body.is_some(), "should be able to read skill body");
        assert!(
            !body.as_ref().map_or(true, String::is_empty),
            "skill body should not be empty"
        );
    }

    #[test]
    fn bundled_skill_origin_deserialized() {
        let store = BundledSkillStore::new();
        let skills = store.discover();
        // At least one bundled skill should have origin metadata.
        let has_origin = skills.iter().any(|s| s.origin.is_some());
        assert!(
            has_origin,
            "at least one bundled skill should have origin metadata"
        );
    }

    #[test]
    fn missing_skill_returns_none() {
        let store = BundledSkillStore::new();
        assert!(store.read_skill("nonexistent-skill-xyz").is_none());
    }
}
