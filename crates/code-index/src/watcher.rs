//! File watcher for code index directories.
//!
//! Watches a project directory for file create/modify/delete events
//! and sends debounced notifications through a channel. Only files
//! that pass the extension filter are reported.
//!
//! Follows the same `notify_debouncer_full` pattern used by
//! `moltis-skills::watcher` and `moltis-openclaw-import::watcher`.

use std::path::{Path, PathBuf};

use notify_debouncer_full::{
    DebounceEventResult, Debouncer, RecommendedCache, new_debouncer,
    notify::RecursiveMode,
};
use tokio::sync::mpsc;
#[cfg(feature = "tracing")]
use tracing::{debug, info, warn};

use crate::config::CodeIndexConfig;
use crate::filter::effective_extension;

/// Events emitted by the code index watcher.
#[derive(Debug, Clone)]
pub enum CodeWatchEvent {
    /// One or more files were created or modified.
    Changed(Vec<PathBuf>),
    /// One or more files were deleted.
    Removed(Vec<PathBuf>),
}

/// Debounce interval for the file watcher (500ms, same as skills watcher).
const DEBOUNCE_INTERVAL_MS: u64 = 500;

/// Watches a project directory for code file changes with debouncing.
///
/// The watcher must be kept alive (not dropped) for events to continue.
pub struct CodeIndexWatcher {
    _debouncer: Debouncer<notify_debouncer_full::notify::RecommendedWatcher, RecommendedCache>,
}

impl CodeIndexWatcher {
    /// Start watching a project directory for file changes.
    ///
    /// Only files that pass the config's extension filter and path
    /// exclusions generate events. The watcher uses 500ms debouncing
    /// to coalesce rapid changes into single events.
    ///
    /// Note: within a single debounced batch, the same path may
    /// appear in multiple events (e.g., a rename produces both a
    /// Create and a Remove). Callers should deduplicate if needed.
    ///
    /// Returns the watcher handle and a receiver for [`CodeWatchEvent`]s.
    /// Drop the watcher to stop watching.
    pub fn start(
        project_dir: &Path,
        config: CodeIndexConfig,
    ) -> anyhow::Result<(Self, mpsc::UnboundedReceiver<CodeWatchEvent>)> {
        let (tx, rx) = mpsc::unbounded_channel();

        let dir = project_dir.to_path_buf();
        let debouncer = new_debouncer(
            std::time::Duration::from_millis(DEBOUNCE_INTERVAL_MS),
            None,
            move |result: DebounceEventResult| match result {
                Ok(events) => {
                    let mut changed = Vec::new();
                    let mut removed = Vec::new();

                    for event in events {
                        // Filter paths through the config's extension and path rules.
                        for path in &event.paths {
                            let rel_path = path.strip_prefix(&dir).unwrap_or(path);

                            // Check path exclusions first (cheapest).
                            if config.path_skipped(&rel_path.to_string_lossy()) {
                                #[cfg(feature = "tracing")]
                                debug!(path = %path.display(), "watcher: skipped path exclusion");
                                continue;
                            }

                            // Check extension (handles extensionless files like Dockerfile).
                            let effective_ext = effective_extension(rel_path);

                            if !config.extension_allowed(effective_ext) {
                                continue;
                            }

                            use notify_debouncer_full::notify::EventKind;
                            match event.kind {
                                EventKind::Create(_) | EventKind::Modify(_) => {
                                    changed.push(path.clone());
                                },
                                EventKind::Remove(_) => {
                                    removed.push(path.clone());
                                },
                                _ => {},
                            }
                        }
                    }

                    if !changed.is_empty() {
                        let _ = tx.send(CodeWatchEvent::Changed(changed));
                    }
                    if !removed.is_empty() {
                        let _ = tx.send(CodeWatchEvent::Removed(removed));
                    }
                },
                Err(errors) => {
                    for e in errors {
                        #[cfg(feature = "tracing")]
                        warn!(error = %e, "code index watcher error");
                        #[cfg(not(feature = "tracing"))]
                        let _ = e;
                    }
                },
            },
        )?;

        let mut watcher = Self {
            _debouncer: debouncer,
        };

        watcher._debouncer.watch(project_dir, RecursiveMode::Recursive)?;
        #[cfg(feature = "tracing")]
        info!(
            path = %project_dir.display(),
            "code index watcher: watching project directory"
        );

        Ok((watcher, rx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_watcher_config_filters_extensions() {
        // Verify that the config's extension filter is wired correctly.
        let config = CodeIndexConfig::default();
        // Default config allows .rs files.
        assert!(config.extension_allowed("rs"));
        // Default config disallows .png files.
        assert!(!config.extension_allowed("png"));
    }

    #[test]
    fn test_watcher_config_filters_paths() {
        let config = CodeIndexConfig::default();
        assert!(config.path_skipped("vendor/lib/foo.rs"));
        assert!(!config.path_skipped("src/main.rs"));
    }

    #[test]
    fn test_watch_event_debug_format() {
        let changed = CodeWatchEvent::Changed(vec![PathBuf::from("/tmp/test.rs")]);
        let debug_str = format!("{changed:?}");
        assert!(debug_str.contains("Changed"));

        let removed = CodeWatchEvent::Removed(vec![PathBuf::from("/tmp/old.rs")]);
        let debug_str = format!("{removed:?}");
        assert!(debug_str.contains("Removed"));
    }
}