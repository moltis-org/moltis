//! Filesystem watcher for OpenClaw session directories.
//!
//! Watches the OpenClaw sessions directory for `.jsonl` create/modify events and
//! sends coalesced notifications so the gateway can run incremental session import.

use std::path::PathBuf;

use {
    notify_debouncer_full::{
        DebounceEventResult, Debouncer, RecommendedCache, new_debouncer, notify::RecursiveMode,
    },
    tokio::sync::mpsc,
    tracing::{debug, info, warn},
};

use crate::error::Result;

/// Events emitted by the import watcher.
#[derive(Debug, Clone)]
pub enum ImportWatchEvent {
    /// One or more `.jsonl` session files were created or modified.
    SessionChanged,
}

/// Watches an OpenClaw sessions directory for JSONL changes with debouncing.
pub struct ImportWatcher {
    _debouncer: Debouncer<notify_debouncer_full::notify::RecommendedWatcher, RecommendedCache>,
}

impl ImportWatcher {
    /// Start watching the given sessions directory. Returns the watcher and a
    /// receiver for coalesced events.
    ///
    /// The watcher must be kept alive (not dropped) for events to continue.
    /// Uses a 5-second debounce window because OpenClaw appends frequently
    /// to session JSONL files during active conversations.
    pub fn start(
        sessions_dir: PathBuf,
    ) -> Result<(Self, mpsc::UnboundedReceiver<ImportWatchEvent>)> {
        let debounce = std::time::Duration::from_secs(5);
        let (tx, rx) = mpsc::unbounded_channel();

        let debouncer = new_debouncer(debounce, None, move |result: DebounceEventResult| {
            match result {
                Ok(events) => {
                    let mut changed = false;
                    for event in events {
                        for path in &event.paths {
                            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                            if ext != "jsonl" {
                                continue;
                            }

                            use notify_debouncer_full::notify::EventKind;
                            match event.kind {
                                EventKind::Create(_) | EventKind::Modify(_) => {
                                    debug!(path = %path.display(), "openclaw session watcher event");
                                    changed = true;
                                },
                                _ => {},
                            }
                        }
                    }
                    if changed {
                        let _ = tx.send(ImportWatchEvent::SessionChanged);
                    }
                },
                Err(errors) => {
                    for e in errors {
                        warn!(error = %e, "openclaw session watcher error");
                    }
                },
            }
        })?;

        let mut watcher = Self {
            _debouncer: debouncer,
        };

        if sessions_dir.exists() {
            watcher
                ._debouncer
                .watch(&sessions_dir, RecursiveMode::NonRecursive)?;
            info!(dir = %sessions_dir.display(), "openclaw: watching sessions directory");
        }

        Ok((watcher, rx))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use {
        super::*,
        notify_debouncer_full::{
            new_debouncer_opt,
            notify::{Config, PollWatcher},
        },
    };

    /// Create a watcher backed by `PollWatcher` so tests don't depend on
    /// OS-level event delivery (macOS FSEvents has unpredictable latency).
    fn start_poll_watcher(
        sessions_dir: PathBuf,
    ) -> Result<(
        Debouncer<PollWatcher, RecommendedCache>,
        mpsc::UnboundedReceiver<ImportWatchEvent>,
    )> {
        let (tx, rx) = mpsc::unbounded_channel();

        let poll_interval = std::time::Duration::from_millis(250);
        let debounce = std::time::Duration::from_millis(500);

        let config = Config::default().with_poll_interval(poll_interval);

        let mut debouncer = new_debouncer_opt::<_, PollWatcher, RecommendedCache>(
            debounce,
            None,
            move |result: DebounceEventResult| {
                if let Ok(events) = result {
                    let changed = events.iter().any(|event| {
                        use notify_debouncer_full::notify::EventKind;
                        matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_))
                            && event
                                .paths
                                .iter()
                                .any(|p| p.extension().and_then(|e| e.to_str()) == Some("jsonl"))
                    });
                    if changed {
                        let _ = tx.send(ImportWatchEvent::SessionChanged);
                    }
                }
            },
            RecommendedCache::default(),
            config,
        )?;

        debouncer.watch(&sessions_dir, RecursiveMode::NonRecursive)?;
        Ok((debouncer, rx))
    }

    #[tokio::test]
    async fn watcher_detects_new_jsonl_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();

        let (_watcher, mut rx) = start_poll_watcher(dir.clone()).unwrap();

        // Write a new JSONL file — the watcher should fire
        std::fs::write(
            dir.join("test-session.jsonl"),
            r#"{"type":"message","message":{"role":"user","content":"hi"}}"#,
        )
        .unwrap();

        let event = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
            .await
            .expect("timed out waiting for watcher event")
            .expect("channel closed");

        assert!(matches!(event, ImportWatchEvent::SessionChanged));
    }

    #[tokio::test]
    async fn watcher_ignores_non_jsonl_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();

        let (_watcher, mut rx) = start_poll_watcher(dir.clone()).unwrap();

        // Write a non-JSONL file — the watcher should NOT fire
        std::fs::write(dir.join("notes.txt"), "some text").unwrap();

        let result = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await;

        assert!(result.is_err(), "expected timeout, no event should fire");
    }
}
