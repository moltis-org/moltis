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

/// Walk `assets/<category>/<skill>/SKILL.md` on the filesystem.
fn discover_from_fs(assets_dir: &Path) -> Vec<SkillMetadata> {
    let mut skills = Vec::new();
    let Ok(categories) = std::fs::read_dir(assets_dir) else {
        return skills;
    };
    for cat_entry in categories.flatten() {
        if !cat_entry.path().is_dir() {
            continue;
        }
        let Ok(skill_dirs) = std::fs::read_dir(cat_entry.path()) else {
            continue;
        };
        for skill_entry in skill_dirs.flatten() {
            let skill_dir = skill_entry.path();
            if !skill_dir.is_dir() {
                continue;
            }
            let skill_md = skill_dir.join("SKILL.md");
            let Ok(content) = std::fs::read_to_string(&skill_md) else {
                continue;
            };
            match parse::parse_metadata(&content, &skill_dir) {
                Ok(mut meta) => {
                    meta.source = Some(SkillSource::Bundled);
                    skills.push(meta);
                },
                Err(e) => {
                    tracing::warn!(path = %skill_md.display(), %e, "failed to parse bundled SKILL.md");
                },
            }
        }
    }
    skills
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

/// Find a skill directory by name under the two-level `<category>/<skill>/` layout.
fn find_skill_dir_fs(assets_dir: &Path, name: &str) -> Option<PathBuf> {
    let categories = std::fs::read_dir(assets_dir).ok()?;
    for cat_entry in categories.flatten() {
        if !cat_entry.path().is_dir() {
            continue;
        }
        let candidate = cat_entry.path().join(name);
        if candidate.is_dir() && candidate.join("SKILL.md").is_file() {
            return Some(candidate);
        }
    }
    None
}

// ── Embedded (release mode) ─────────────────────────────────────────────────

/// Walk the embedded `include_dir!` tree for SKILL.md files.
fn discover_from_embedded() -> Vec<SkillMetadata> {
    let mut skills = Vec::new();
    for category_dir in BUNDLED_ASSETS.dirs() {
        for skill_dir in category_dir.dirs() {
            let Some(skill_md) = skill_dir.get_file("SKILL.md") else {
                continue;
            };
            let Ok(content) = std::str::from_utf8(skill_md.contents()) else {
                continue;
            };
            // Use a synthetic path for the skill directory (never hits filesystem).
            let synthetic_path =
                PathBuf::from("__bundled__").join(skill_dir.path().to_string_lossy().as_ref());
            match parse::parse_metadata(content, &synthetic_path) {
                Ok(mut meta) => {
                    meta.source = Some(SkillSource::Bundled);
                    skills.push(meta);
                },
                Err(e) => {
                    tracing::warn!(
                        path = %skill_dir.path().display(),
                        %e,
                        "failed to parse embedded bundled SKILL.md"
                    );
                },
            }
        }
    }
    skills
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

/// Find a skill subdirectory by name in the embedded two-level layout.
fn find_skill_dir_embedded(name: &str) -> Option<&'static include_dir::Dir<'static>> {
    for category_dir in BUNDLED_ASSETS.dirs() {
        for skill_dir in category_dir.dirs() {
            // Match on the directory name (last path component).
            let dir_name = skill_dir
                .path()
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            if dir_name == name && skill_dir.get_file("SKILL.md").is_some() {
                return Some(skill_dir);
            }
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
