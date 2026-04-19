//! Filesystem watcher for skill directories.
//!
//! Watches skill search paths for SKILL.md create/modify/delete events and sends
//! notifications through a channel so the gateway can broadcast `skills.changed`.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use {
    notify_debouncer_full::{
        DebounceEventResult, Debouncer, RecommendedCache, new_debouncer, notify::RecursiveMode,
    },
    tokio::sync::mpsc,
    tracing::{debug, info, warn},
};

use crate::{
    discover::FsSkillDiscoverer,
    error::Result,
    manifest::ManifestStore,
    types::{SkillSource, SkillsManifest},
};

/// Events emitted by the skill watcher.
#[derive(Debug, Clone)]
pub enum SkillWatchEvent {
    /// An enabled skill's `SKILL.md` changed.
    SkillChanged,
    /// The skills manifest changed, which may require rebuilding the watch set.
    ManifestChanged,
}

/// A filesystem path plus the recursion mode to use when watching it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchSpec {
    pub path: PathBuf,
    pub recursive_mode: RecursiveMode,
}

fn insert_watch_spec(
    specs: &mut BTreeMap<PathBuf, RecursiveMode>,
    path: PathBuf,
    recursive_mode: RecursiveMode,
) {
    specs
        .entry(path)
        .and_modify(|mode| {
            if recursive_mode == RecursiveMode::Recursive {
                *mode = RecursiveMode::Recursive;
            }
        })
        .or_insert(recursive_mode);
}

fn installed_skill_watch_path(data_dir: &Path, relative_path: &str) -> PathBuf {
    let primary = data_dir.join("installed-skills").join(relative_path);
    if primary.exists() {
        return primary;
    }

    let legacy = data_dir.join("installed-plugins").join(relative_path);
    if legacy.exists() {
        return legacy;
    }

    primary
}

pub(crate) fn build_watch_specs(
    data_dir: &Path,
    search_paths: &[(PathBuf, SkillSource)],
    manifest: &SkillsManifest,
) -> Vec<WatchSpec> {
    let mut specs = BTreeMap::<PathBuf, RecursiveMode>::new();

    for (path, source) in search_paths {
        match source {
            SkillSource::Project | SkillSource::Personal => {
                insert_watch_spec(&mut specs, path.clone(), RecursiveMode::Recursive);
            },
            SkillSource::Registry | SkillSource::Plugin => {},
        }
    }

    insert_watch_spec(
        &mut specs,
        data_dir.to_path_buf(),
        RecursiveMode::NonRecursive,
    );

    for repo in &manifest.repos {
        for skill in &repo.skills {
            if !skill.enabled || !skill.trusted {
                continue;
            }

            let watch_path = installed_skill_watch_path(data_dir, &skill.relative_path);
            insert_watch_spec(&mut specs, watch_path, RecursiveMode::Recursive);
        }
    }

    specs
        .into_iter()
        .map(|(path, recursive_mode)| WatchSpec {
            path,
            recursive_mode,
        })
        .collect()
}

pub fn default_watch_specs() -> Result<Vec<WatchSpec>> {
    let data_dir = moltis_config::data_dir();
    let search_paths = FsSkillDiscoverer::default_paths();
    let manifest_path = ManifestStore::default_path()?;
    let manifest = ManifestStore::new(manifest_path).load().unwrap_or_default();
    Ok(build_watch_specs(&data_dir, &search_paths, &manifest))
}

/// Watches skill directories for SKILL.md changes with debouncing.
pub struct SkillWatcher {
    _debouncer: Debouncer<notify_debouncer_full::notify::RecommendedWatcher, RecommendedCache>,
}

impl SkillWatcher {
    /// Start watching the given directories. Returns the watcher and a receiver for events.
    ///
    /// The watcher must be kept alive (not dropped) for events to continue.
    pub fn start(
        specs: Vec<WatchSpec>,
    ) -> Result<(Self, mpsc::UnboundedReceiver<SkillWatchEvent>)> {
        let (tx, rx) = mpsc::unbounded_channel();

        let debouncer = new_debouncer(
            std::time::Duration::from_millis(500),
            None,
            move |result: DebounceEventResult| match result {
                Ok(events) => {
                    let mut skill_changed = false;
                    let mut manifest_changed = false;
                    for event in events {
                        for path in &event.paths {
                            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                            if filename != "SKILL.md" && filename != "skills-manifest.json" {
                                continue;
                            }

                            use notify_debouncer_full::notify::EventKind;
                            match event.kind {
                                EventKind::Create(_)
                                | EventKind::Modify(_)
                                | EventKind::Remove(_) => {
                                    debug!(path = %path.display(), "skill watcher event");
                                    if filename == "skills-manifest.json" {
                                        manifest_changed = true;
                                    } else {
                                        skill_changed = true;
                                    }
                                },
                                _ => {},
                            }
                        }
                    }
                    if manifest_changed {
                        let _ = tx.send(SkillWatchEvent::ManifestChanged);
                    } else if skill_changed {
                        let _ = tx.send(SkillWatchEvent::SkillChanged);
                    }
                },
                Err(errors) => {
                    for e in errors {
                        warn!(error = %e, "skill watcher error");
                    }
                },
            },
        )?;

        let mut watcher = Self {
            _debouncer: debouncer,
        };

        for spec in &specs {
            if spec.path.exists() {
                watcher._debouncer.watch(&spec.path, spec.recursive_mode)?;
                info!(
                    path = %spec.path.display(),
                    recursive = matches!(spec.recursive_mode, RecursiveMode::Recursive),
                    "skill watcher: watching path"
                );
            }
        }

        Ok((watcher, rx))
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::types::{RepoEntry, SkillState},
    };

    fn has_spec(specs: &[WatchSpec], path: &Path, recursive_mode: RecursiveMode) -> bool {
        specs
            .iter()
            .any(|spec| spec.path == path && spec.recursive_mode == recursive_mode)
    }

    #[test]
    fn build_watch_specs_limits_registry_watch_roots_to_enabled_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path();
        let project_dir = data_dir.join(".moltis/skills");
        let personal_dir = data_dir.join("skills");
        let search_paths = vec![
            (project_dir.clone(), SkillSource::Project),
            (personal_dir.clone(), SkillSource::Personal),
            (data_dir.join("installed-skills"), SkillSource::Registry),
            (data_dir.join("installed-plugins"), SkillSource::Plugin),
        ];
        let manifest = SkillsManifest {
            version: 1,
            repos: vec![RepoEntry {
                source: "owner/repo".into(),
                repo_name: "repo".into(),
                installed_at_ms: 0,
                commit_sha: None,
                format: Default::default(),
                quarantined: false,
                quarantine_reason: None,
                provenance: None,
                skills: vec![
                    SkillState {
                        name: "enabled".into(),
                        relative_path: "repo/skills/enabled".into(),
                        trusted: true,
                        enabled: true,
                    },
                    SkillState {
                        name: "disabled".into(),
                        relative_path: "repo/skills/disabled".into(),
                        trusted: true,
                        enabled: false,
                    },
                ],
            }],
        };

        let specs = build_watch_specs(data_dir, &search_paths, &manifest);

        assert!(has_spec(&specs, &project_dir, RecursiveMode::Recursive));
        assert!(has_spec(&specs, &personal_dir, RecursiveMode::Recursive));
        assert!(has_spec(&specs, data_dir, RecursiveMode::NonRecursive));
        assert!(has_spec(
            &specs,
            &data_dir.join("installed-skills/repo/skills/enabled"),
            RecursiveMode::Recursive
        ));
        assert!(!has_spec(
            &specs,
            &data_dir.join("installed-skills"),
            RecursiveMode::Recursive
        ));
        assert!(!has_spec(
            &specs,
            &data_dir.join("installed-skills/repo/skills/disabled"),
            RecursiveMode::Recursive
        ));
    }
}
