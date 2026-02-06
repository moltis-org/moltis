//! Filesystem watcher for skill directories.
//!
//! Watches skill search paths for SKILL.md create/modify/delete events and sends
//! notifications through a channel so the gateway can broadcast `skills.changed`.

use std::path::PathBuf;

use {
    anyhow::Result,
    notify_debouncer_full::{
        DebounceEventResult, Debouncer, RecommendedCache, new_debouncer, notify::RecursiveMode,
    },
    tokio::sync::mpsc,
    tracing::{debug, info, warn},
};

/// Events emitted by the skill watcher.
#[derive(Debug, Clone)]
pub enum SkillWatchEvent {
    /// A skill was created, modified, or deleted.
    Changed,
}

/// Watches skill directories for SKILL.md changes with debouncing.
pub struct SkillWatcher {
    _debouncer: Debouncer<notify_debouncer_full::notify::RecommendedWatcher, RecommendedCache>,
}

impl SkillWatcher {
    /// Start watching the given directories. Returns the watcher and a receiver for events.
    ///
    /// The watcher must be kept alive (not dropped) for events to continue.
    pub fn start(dirs: Vec<PathBuf>) -> Result<(Self, mpsc::UnboundedReceiver<SkillWatchEvent>)> {
        let (tx, rx) = mpsc::unbounded_channel();

        let debouncer = new_debouncer(
            std::time::Duration::from_millis(500),
            None,
            move |result: DebounceEventResult| match result {
                Ok(events) => {
                    let mut changed = false;
                    for event in events {
                        for path in &event.paths {
                            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                            if filename != "SKILL.md" {
                                continue;
                            }

                            use notify_debouncer_full::notify::EventKind;
                            match event.kind {
                                EventKind::Create(_)
                                | EventKind::Modify(_)
                                | EventKind::Remove(_) => {
                                    debug!(path = %path.display(), "skill watcher event");
                                    changed = true;
                                },
                                _ => {},
                            }
                        }
                    }
                    if changed {
                        let _ = tx.send(SkillWatchEvent::Changed);
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

        for dir in &dirs {
            if dir.exists() {
                watcher._debouncer.watch(dir, RecursiveMode::Recursive)?;
                info!(dir = %dir.display(), "skill watcher: watching directory");
            }
        }

        Ok((watcher, rx))
    }
}
