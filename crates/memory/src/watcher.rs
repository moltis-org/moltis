/// Real-time file watching for memory sync using notify-debouncer-full.
use std::{collections::BTreeMap, path::PathBuf};

use {
    anyhow::Result,
    notify_debouncer_full::{
        DebounceEventResult, Debouncer, RecommendedCache, new_debouncer, notify::RecursiveMode,
    },
    tokio::sync::mpsc,
    tracing::{debug, info, warn},
};

/// Events emitted by the file watcher.
#[derive(Debug, Clone)]
pub enum WatchEvent {
    Created(PathBuf),
    Modified(PathBuf),
    Removed(PathBuf),
}

/// A filesystem path plus the recursion mode to use when watching it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchSpec {
    pub path: PathBuf,
    pub recursive_mode: RecursiveMode,
}

/// Convert configured memory scan paths into a minimal set of watch roots.
///
/// File paths are watched via their parent directory in non-recursive mode so
/// root-level files like `MEMORY.md` do not force a recursive watch on the
/// entire data directory. Directory paths keep recursive watching.
pub fn build_watch_specs(paths: &[PathBuf]) -> Vec<WatchSpec> {
    let mut specs = BTreeMap::<PathBuf, RecursiveMode>::new();

    for path in paths {
        let recursive_mode = if path.is_dir() {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };

        let watch_path = if recursive_mode == RecursiveMode::Recursive {
            path.clone()
        } else {
            path.parent().unwrap_or(path.as_path()).to_path_buf()
        };

        specs
            .entry(watch_path)
            .and_modify(|mode| {
                if recursive_mode == RecursiveMode::Recursive {
                    *mode = RecursiveMode::Recursive;
                }
            })
            .or_insert(recursive_mode);
    }

    specs
        .into_iter()
        .map(|(path, recursive_mode)| WatchSpec {
            path,
            recursive_mode,
        })
        .collect()
}

/// Watches directories for markdown file changes with debouncing.
pub struct MemoryFileWatcher {
    debouncer: Debouncer<notify_debouncer_full::notify::RecommendedWatcher, RecommendedCache>,
}

impl MemoryFileWatcher {
    /// Start watching the given directories. Returns the watcher and a receiver for events.
    pub fn start(specs: Vec<WatchSpec>) -> Result<(Self, mpsc::UnboundedReceiver<WatchEvent>)> {
        let (tx, rx) = mpsc::unbounded_channel();

        let debouncer = new_debouncer(
            std::time::Duration::from_millis(1500),
            None,
            move |result: DebounceEventResult| {
                match result {
                    Ok(events) => {
                        for event in events {
                            for path in &event.paths {
                                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                                if ext != "md" && ext != "markdown" {
                                    continue;
                                }

                                use notify_debouncer_full::notify::EventKind;
                                let watch_event = match event.kind {
                                    EventKind::Create(_) => WatchEvent::Created(path.clone()),
                                    EventKind::Modify(_) => WatchEvent::Modified(path.clone()),
                                    EventKind::Remove(_) => WatchEvent::Removed(path.clone()),
                                    _ => continue,
                                };

                                debug!(path = %path.display(), "file watcher event");
                                if tx.send(watch_event).is_err() {
                                    return; // receiver dropped
                                }
                            }
                        }
                    },
                    Err(errors) => {
                        for e in errors {
                            warn!(error = %e, "file watcher error");
                        }
                    },
                }
            },
        )?;

        let mut watcher = Self { debouncer };

        for spec in &specs {
            if spec.path.exists() {
                watcher.debouncer.watch(&spec.path, spec.recursive_mode)?;
                info!(
                    path = %spec.path.display(),
                    recursive = matches!(spec.recursive_mode, RecursiveMode::Recursive),
                    "file watcher: watching path"
                );
            }
        }

        Ok((watcher, rx))
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    fn has_spec(
        specs: &[WatchSpec],
        path: &std::path::Path,
        recursive_mode: RecursiveMode,
    ) -> bool {
        specs
            .iter()
            .any(|spec| spec.path == path && spec.recursive_mode == recursive_mode)
    }

    #[test]
    fn build_watch_specs_does_not_recurse_over_data_dir_for_files() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path();
        let specs = build_watch_specs(&[
            data_dir.join("MEMORY.md"),
            data_dir.join("memory.md"),
            data_dir.join("memory"),
            data_dir.join("agents"),
        ]);

        assert!(has_spec(&specs, data_dir, RecursiveMode::NonRecursive));
        assert!(!has_spec(&specs, data_dir, RecursiveMode::Recursive));
    }

    #[test]
    fn build_watch_specs_recurse_only_existing_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path();
        let memory_dir = data_dir.join("memory");
        let agents_dir = data_dir.join("agents");
        std::fs::create_dir_all(&memory_dir).unwrap();
        std::fs::create_dir_all(&agents_dir).unwrap();

        let specs = build_watch_specs(&[
            data_dir.join("MEMORY.md"),
            data_dir.join("memory.md"),
            memory_dir.clone(),
            agents_dir.clone(),
        ]);

        assert!(has_spec(&specs, data_dir, RecursiveMode::NonRecursive));
        assert!(has_spec(&specs, &memory_dir, RecursiveMode::Recursive));
        assert!(has_spec(&specs, &agents_dir, RecursiveMode::Recursive));
    }
}
