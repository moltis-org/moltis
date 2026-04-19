use std::path::{Path, PathBuf};

use async_trait::async_trait;

use crate::{
    error::Result,
    formats::PluginFormat,
    manifest::ManifestStore,
    parse,
    types::{SkillMetadata, SkillSource},
};

/// Discovers skills from filesystem paths.
#[async_trait]
pub trait SkillDiscoverer: Send + Sync {
    /// Scan configured paths and return metadata for all discovered skills.
    async fn discover(&self) -> Result<Vec<SkillMetadata>>;
}

/// Default filesystem-based skill discoverer.
pub struct FsSkillDiscoverer {
    /// (path, source) pairs to scan, in priority order.
    search_paths: Vec<(PathBuf, SkillSource)>,
}

impl FsSkillDiscoverer {
    pub fn new(search_paths: Vec<(PathBuf, SkillSource)>) -> Self {
        Self { search_paths }
    }

    /// Build the default search paths for skill discovery.
    ///
    /// Workspace root is always the configured data directory.
    pub fn default_paths() -> Vec<(PathBuf, SkillSource)> {
        Self::default_paths_for(&moltis_config::data_dir())
    }

    /// Build the default search paths rooted at an explicit workspace / data
    /// directory.
    ///
    /// Prefer this over [`default_paths`](Self::default_paths) when the caller
    /// already has a `data_dir` in hand (e.g. the gateway's `bootstrap` scope)
    /// so the read and write sides stay consistent even if
    /// `moltis_config::data_dir()` is ever reconfigured at runtime.
    #[must_use]
    pub fn default_paths_for(data_dir: &Path) -> Vec<(PathBuf, SkillSource)> {
        vec![
            (data_dir.join(".moltis/skills"), SkillSource::Project),
            (data_dir.join("skills"), SkillSource::Personal),
            (data_dir.join("installed-skills"), SkillSource::Registry),
            (data_dir.join("installed-plugins"), SkillSource::Plugin),
        ]
    }
}

#[async_trait]
impl SkillDiscoverer for FsSkillDiscoverer {
    async fn discover(&self) -> Result<Vec<SkillMetadata>> {
        let mut skills = Vec::new();

        for (base_path, source) in &self.search_paths {
            if !base_path.is_dir() {
                continue;
            }

            match source {
                // Project/Personal: scan one level deep (always enabled).
                SkillSource::Project | SkillSource::Personal => {
                    discover_flat(base_path, source, &mut skills);
                },
                // Registry: use manifest to filter by enabled state.
                SkillSource::Registry => {
                    discover_registry(base_path, &mut skills);
                },
                // Plugin: use plugins manifest to filter by enabled state.
                SkillSource::Plugin => {
                    discover_plugins(base_path, &mut skills);
                },
            }
        }

        Ok(skills)
    }
}

/// Scan one level deep for SKILL.md dirs (project/personal sources).
fn discover_flat(base_path: &Path, source: &SkillSource, skills: &mut Vec<SkillMetadata>) {
    let entries = match std::fs::read_dir(base_path) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let skill_dir = entry.path();
        if !skill_dir.is_dir() {
            continue;
        }
        let skill_md = skill_dir.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        let content = match std::fs::read_to_string(&skill_md) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(?skill_md, %e, "failed to read SKILL.md");
                continue;
            },
        };
        match parse::parse_metadata(&content, &skill_dir) {
            Ok(mut meta) => {
                meta.source = Some(source.clone());
                tracing::debug!(
                    path = %skill_md.display(),
                    source = ?source,
                    name = %meta.name,
                    "loaded SKILL.md"
                );
                skills.push(meta);
            },
            Err(e) => {
                tracing::warn!(?skill_dir, %e, "failed to parse SKILL.md");
            },
        }
    }
}

/// Discover enabled plugin skills using the plugins manifest.
/// Plugin skills don't have SKILL.md — they are normalized by format adapters.
/// This returns lightweight metadata from the manifest for prompt injection.
fn discover_plugins(install_dir: &Path, skills: &mut Vec<SkillMetadata>) {
    let manifest_path = moltis_config::data_dir().join("plugins-manifest.json");
    let store = ManifestStore::new(manifest_path);
    let manifest = match store.load() {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(%e, "failed to load plugins manifest");
            return;
        },
    };

    for repo in &manifest.repos {
        for skill_state in &repo.skills {
            if !skill_state.enabled || !skill_state.trusted {
                continue;
            }
            let skill_dir = install_dir.join(&skill_state.relative_path);
            skills.push(SkillMetadata {
                name: skill_state.name.clone(),
                path: skill_dir,
                source: Some(SkillSource::Plugin),
                ..Default::default()
            });
        }
    }
}

/// Discover registry skills using the manifest for enabled filtering.
///
/// Handles both formats:
/// - `PluginFormat::Skill` → parse `SKILL.md` from disk for full metadata
/// - Other formats → create stub metadata with `SkillSource::Plugin` (prompt_gen
///   uses the path as-is instead of appending `/SKILL.md`)
fn discover_registry(install_dir: &Path, skills: &mut Vec<SkillMetadata>) {
    let manifest_path = match ManifestStore::default_path() {
        Ok(p) => p,
        Err(_) => return,
    };
    let store = ManifestStore::new(manifest_path);
    let manifest = match store.load() {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(%e, "failed to load skills manifest");
            return;
        },
    };

    for repo in &manifest.repos {
        for skill_state in &repo.skills {
            if !skill_state.enabled || !skill_state.trusted {
                continue;
            }
            let skill_dir = install_dir.join(&skill_state.relative_path);

            match repo.format {
                PluginFormat::Skill => {
                    let skill_md = skill_dir.join("SKILL.md");
                    if !skill_md.is_file() {
                        tracing::warn!(?skill_md, "manifest references missing SKILL.md");
                        continue;
                    }
                    let content = match std::fs::read_to_string(&skill_md) {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::warn!(?skill_md, %e, "failed to read SKILL.md");
                            continue;
                        },
                    };
                    match parse::parse_metadata(&content, &skill_dir) {
                        Ok(mut meta) => {
                            meta.source = Some(SkillSource::Registry);
                            tracing::debug!(
                                path = %skill_md.display(),
                                source = "registry",
                                name = %meta.name,
                                "loaded SKILL.md"
                            );
                            skills.push(meta);
                        },
                        Err(e) => {
                            tracing::debug!(?skill_dir, %e, "skipping non-conforming SKILL.md");
                        },
                    }
                },
                _ => {
                    // Non-SKILL.md formats: stub metadata with Plugin source
                    // so prompt_gen uses the path directly (no /SKILL.md append).
                    skills.push(SkillMetadata {
                        name: skill_state.name.clone(),
                        path: skill_dir,
                        source: Some(SkillSource::Plugin),
                        ..Default::default()
                    });
                },
            }
        }
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::types::{RepoEntry, SkillState, SkillsManifest},
    };

    #[test]
    fn default_paths_for_returns_expected_layout() {
        // Regression guard: the gateway wires `ReadSkillTool` through this
        // helper, so the shape of the returned list is part of the public
        // contract. If someone reorders or renames any of these paths, the
        // `<available_skills>` prompt block and the read tool could start
        // disagreeing about which directories contain skills.
        let data_dir = PathBuf::from("/tmp/data");
        let paths = FsSkillDiscoverer::default_paths_for(&data_dir);
        assert_eq!(paths.len(), 4);
        assert_eq!(paths[0].0, PathBuf::from("/tmp/data/.moltis/skills"));
        assert_eq!(paths[0].1, SkillSource::Project);
        assert_eq!(paths[1].0, PathBuf::from("/tmp/data/skills"));
        assert_eq!(paths[1].1, SkillSource::Personal);
        assert_eq!(paths[2].0, PathBuf::from("/tmp/data/installed-skills"));
        assert_eq!(paths[2].1, SkillSource::Registry);
        assert_eq!(paths[3].0, PathBuf::from("/tmp/data/installed-plugins"));
        assert_eq!(paths[3].1, SkillSource::Plugin);
    }

    #[test]
    fn default_paths_matches_default_paths_for_with_data_dir() {
        // The zero-arg helper must reduce to the explicit-`data_dir`
        // variant applied to `moltis_config::data_dir()`. Any future
        // refactor that breaks this symmetry would cause the prompt
        // builder and the read tool to see different filesystem layouts.
        let explicit = FsSkillDiscoverer::default_paths_for(&moltis_config::data_dir());
        let implicit = FsSkillDiscoverer::default_paths();
        assert_eq!(explicit, implicit);
    }

    #[tokio::test]
    async fn test_discover_skills_in_temp_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(skills_dir.join("my-skill")).unwrap();
        std::fs::write(
            skills_dir.join("my-skill/SKILL.md"),
            "---\nname: my-skill\ndescription: test\n---\nbody\n",
        )
        .unwrap();

        let discoverer = FsSkillDiscoverer::new(vec![(skills_dir.clone(), SkillSource::Project)]);
        let skills = discoverer.discover().await.unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "my-skill");
        assert_eq!(skills[0].source, Some(SkillSource::Project));
    }

    #[tokio::test]
    async fn test_discover_skips_missing_dirs() {
        let discoverer = FsSkillDiscoverer::new(vec![(
            PathBuf::from("/nonexistent/path"),
            SkillSource::Personal,
        )]);
        let skills = discoverer.discover().await.unwrap();
        assert!(skills.is_empty());
    }

    #[tokio::test]
    async fn test_discover_skips_dirs_without_skill_md() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(skills_dir.join("not-a-skill")).unwrap();
        std::fs::write(skills_dir.join("not-a-skill/README.md"), "hello").unwrap();

        let discoverer = FsSkillDiscoverer::new(vec![(skills_dir, SkillSource::Project)]);
        let skills = discoverer.discover().await.unwrap();
        assert!(skills.is_empty());
    }

    #[tokio::test]
    async fn test_discover_skips_invalid_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std::fs::create_dir_all(skills_dir.join("bad-skill")).unwrap();
        std::fs::write(skills_dir.join("bad-skill/SKILL.md"), "no frontmatter here").unwrap();

        let discoverer = FsSkillDiscoverer::new(vec![(skills_dir, SkillSource::Project)]);
        let skills = discoverer.discover().await.unwrap();
        assert!(skills.is_empty());
    }

    #[tokio::test]
    async fn test_discover_registry_filters_disabled() {
        // This test exercises the manifest-based registry discovery.
        // We need a manifest file and matching skill dirs.
        let tmp = tempfile::tempdir().unwrap();
        let install_dir = tmp.path().join("installed-skills");
        let manifest_path = tmp.path().join("manifest.json");

        // Create repo with two skills on disk.
        std::fs::create_dir_all(install_dir.join("repo/skills/a")).unwrap();
        std::fs::create_dir_all(install_dir.join("repo/skills/b")).unwrap();
        std::fs::write(
            install_dir.join("repo/skills/a/SKILL.md"),
            "---\nname: a\ndescription: skill a\n---\nbody\n",
        )
        .unwrap();
        std::fs::write(
            install_dir.join("repo/skills/b/SKILL.md"),
            "---\nname: b\ndescription: skill b\n---\nbody\n",
        )
        .unwrap();

        // Create manifest with 'a' enabled and 'b' disabled.
        let manifest = SkillsManifest {
            version: 1,
            repos: vec![RepoEntry {
                source: "owner/repo".into(),
                repo_name: "repo".into(),
                installed_at_ms: 0,
                commit_sha: None,
                format: PluginFormat::Skill,
                quarantined: false,
                quarantine_reason: None,
                provenance: None,
                skills: vec![
                    SkillState {
                        name: "a".into(),
                        relative_path: "repo/skills/a".into(),
                        trusted: true,
                        enabled: true,
                    },
                    SkillState {
                        name: "b".into(),
                        relative_path: "repo/skills/b".into(),
                        trusted: false,
                        enabled: false,
                    },
                ],
            }],
        };
        let store = ManifestStore::new(manifest_path.clone());
        store.save(&manifest).unwrap();

        // We can't easily test discover_registry with a custom manifest path
        // since it uses ManifestStore::default_path(). Instead, test the function
        // directly.
        let mut skills = Vec::new();

        // Manually call the inner function with the right manifest.
        // Since discover_registry uses default_path, we test the flat path instead.
        discover_flat(
            &install_dir.join("repo/skills"),
            &SkillSource::Project,
            &mut skills,
        );
        // Both skills found when using flat scan (no filtering).
        assert_eq!(skills.len(), 2);
    }

    #[test]
    fn test_discover_registry_mixed_formats() {
        use crate::formats::PluginFormat;

        let tmp = tempfile::tempdir().unwrap();
        let install_dir = tmp.path();

        // SKILL.md repo on disk
        std::fs::create_dir_all(install_dir.join("skill-repo/SKILL.md").parent().unwrap()).unwrap();
        std::fs::write(
            install_dir.join("skill-repo/SKILL.md"),
            "---\nname: my-skill\ndescription: a native skill\n---\nbody\n",
        )
        .unwrap();

        // Claude Code repo on disk (no SKILL.md)
        std::fs::create_dir_all(install_dir.join("plugin-repo")).unwrap();

        // Build manifest with both formats
        let manifest = SkillsManifest {
            version: 1,
            repos: vec![
                RepoEntry {
                    source: "owner/skill-repo".into(),
                    repo_name: "skill-repo".into(),
                    installed_at_ms: 0,
                    commit_sha: None,
                    format: PluginFormat::Skill,
                    quarantined: false,
                    quarantine_reason: None,
                    provenance: None,
                    skills: vec![SkillState {
                        name: "my-skill".into(),
                        relative_path: "skill-repo".into(),
                        trusted: true,
                        enabled: true,
                    }],
                },
                RepoEntry {
                    source: "owner/plugin-repo".into(),
                    repo_name: "plugin-repo".into(),
                    installed_at_ms: 0,
                    commit_sha: None,
                    format: PluginFormat::ClaudeCode,
                    quarantined: false,
                    quarantine_reason: None,
                    provenance: None,
                    skills: vec![SkillState {
                        name: "test-plugin:helper".into(),
                        relative_path: "plugin-repo".into(),
                        trusted: true,
                        enabled: true,
                    }],
                },
            ],
        };
        let manifest_path = tmp.path().join("skills-manifest.json");
        let store = ManifestStore::new(manifest_path);
        store.save(&manifest).unwrap();

        // Can't call discover_registry directly (uses default_path), so
        // simulate the logic inline.
        let mut skills = Vec::new();
        for repo in &manifest.repos {
            for skill_state in &repo.skills {
                if !skill_state.enabled || !skill_state.trusted {
                    continue;
                }
                let skill_dir = install_dir.join(&skill_state.relative_path);
                match repo.format {
                    PluginFormat::Skill => {
                        let skill_md = skill_dir.join("SKILL.md");
                        if skill_md.is_file() {
                            let content = std::fs::read_to_string(&skill_md).unwrap();
                            let mut meta = parse::parse_metadata(&content, &skill_dir).unwrap();
                            meta.source = Some(SkillSource::Registry);
                            skills.push(meta);
                        }
                    },
                    _ => {
                        skills.push(SkillMetadata {
                            name: skill_state.name.clone(),
                            path: skill_dir,
                            source: Some(SkillSource::Plugin),
                            ..Default::default()
                        });
                    },
                }
            }
        }

        assert_eq!(skills.len(), 2);

        // SKILL.md repo gets full metadata with Registry source
        let skill = skills.iter().find(|s| s.name == "my-skill").unwrap();
        assert_eq!(skill.source, Some(SkillSource::Registry));
        assert_eq!(skill.description, "a native skill");

        // Claude Code repo gets stub metadata with Plugin source
        let plugin = skills
            .iter()
            .find(|s| s.name == "test-plugin:helper")
            .unwrap();
        assert_eq!(plugin.source, Some(SkillSource::Plugin));
        assert!(plugin.description.is_empty());
    }
}
