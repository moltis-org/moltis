//! File watcher for incremental code index updates.
//!
//! Uses `notify-debouncer-full` to watch project directories for file changes,
//! then invokes a handler callback with the set of changed file paths.
//!
//! The watcher is started via [`CodeIndex::start_watcher`] in `index.rs`.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use {
    notify_debouncer_full::{new_debouncer, notify::RecursiveMode},
    tokio::sync::mpsc,
    tracing::{debug, info},
};

use crate::{filter::FilterConfig, types::Language};

/// Debounce delay for file system events (ms).
const DEBOUNCE_MS: u64 = 500;

/// A debounced file-system event, sent from the notify backend to the processor.
#[derive(Debug)]
struct WatchEvent {
    paths: Vec<PathBuf>,
    kind: notify_debouncer_full::notify::EventKind,
}

/// Handler invoked when files change in a watched project.
pub type WatchHandler = Arc<dyn Fn(&str, &[PathBuf]) + Send + Sync>;

/// A running file watcher for a single project directory.
#[allow(dead_code)] // watch_dir kept for future diagnostics/health-check
pub struct FileWatcher {
    /// The project ID this watcher is associated with.
    project_id: String,
    /// The root directory being watched.
    watch_dir: PathBuf,
    /// Cancellation token to stop the watcher.
    cancel: tokio_util::sync::CancellationToken,
}

impl FileWatcher {
    /// Start watching a project directory for file changes.
    ///
    /// Spawns a background task that debounces filesystem events and calls
    /// `handler` with the set of changed file paths.
    pub fn start(
        project_id: String,
        watch_dir: PathBuf,
        filter_config: FilterConfig,
        handler: WatchHandler,
    ) -> Result<Self, notify_debouncer_full::notify::Error> {
        let cancel = tokio_util::sync::CancellationToken::new();
        let cancel_clone = cancel.clone();
        let pid = project_id.clone();

        // Channel for debounced events from the notify backend.
        let (tx, mut rx) = mpsc::channel::<WatchEvent>(256);

        // Create the filesystem watcher.
        let mut debouncer = new_debouncer(
            Duration::from_millis(DEBOUNCE_MS),
            None,
            move |result: Result<
                Vec<notify_debouncer_full::DebouncedEvent>,
                Vec<notify_debouncer_full::notify::Error>,
            >| {
                if let Ok(events) = result {
                    for event in events {
                        let watch_event = WatchEvent {
                            paths: event.paths.clone(),
                            kind: event.kind,
                        };
                        let _ = tx.blocking_send(watch_event);
                    }
                }
            },
        )?;

        debouncer.watch(&watch_dir, RecursiveMode::Recursive)?;

        info!(
            project_id = %pid,
            path = %watch_dir.display(),
            "file watcher started"
        );

        // Spawn the event processing loop.
        let _watcher_guard = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel_clone.cancelled() => {
                        debug!(project_id = %pid, "file watcher stopping");
                        break;
                    }
                    event = rx.recv() => {
                        match event {
                            Some(event) => {
                                Self::handle_event(&pid, &filter_config, &handler, event);
                            }
                            None => break, // Channel closed
                        }
                    }
                }
            }
        });

        Ok(Self {
            project_id,
            watch_dir,
            cancel,
        })
    }

    /// Stop the watcher.
    pub fn stop(&self) {
        info!(project_id = %self.project_id, "stopping file watcher");
        self.cancel.cancel();
    }

    /// Return the project ID this watcher is for.
    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    fn handle_event(
        project_id: &str,
        filter_config: &FilterConfig,
        handler: &WatchHandler,
        event: WatchEvent,
    ) {
        use notify_debouncer_full::notify::EventKind;

        // We only care about create, modify, and remove events.
        let is_relevant = matches!(
            event.kind,
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
        );
        if !is_relevant {
            return;
        }

        // Filter to indexable files only.
        let indexable: Vec<PathBuf> = event
            .paths
            .into_iter()
            .filter(|p| Self::is_indexable(p, filter_config))
            .collect();

        if !indexable.is_empty() {
            debug!(
                project_id,
                count = indexable.len(),
                "files changed, invoking handler"
            );
            handler(project_id, &indexable);
        }
    }

    /// Check if a file path should be indexed based on extension and filter config.
    fn is_indexable(path: &Path, config: &FilterConfig) -> bool {
        // Must be a file (not a directory).
        if !path.is_file() {
            return false;
        }

        // Check extension.
        let ext = match path.extension().and_then(|e| e.to_str()) {
            Some(e) => e,
            None => return false,
        };

        // Check against known language extensions.
        let lang = Language::from_extension(ext);
        if matches!(lang, Language::Unknown) {
            return false;
        }

        // Check against ignored patterns.
        let path_str = path.to_string_lossy();
        for pattern in &config.skip_paths {
            if path_str.contains(pattern) {
                return false;
            }
        }

        true
    }
}

impl Drop for FileWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}
