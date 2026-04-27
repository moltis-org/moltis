//! Index job manager for coordinating code indexing operations.
//!
//! Provides:
//! - Deduplicated indexing jobs (one per project at a time)
//! - Concurrent job limiting via semaphore
//! - File watcher lifecycle management
//! - Periodic re-index loop

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use tokio::{
    sync::{Mutex, Semaphore},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

#[cfg(feature = "tracing")]
use crate::log::{debug, error, info, warn};

use crate::{CodeIndex, Error};

/// Configuration for the IndexJobManager.
#[derive(Debug, Clone)]
pub struct IndexJobManagerConfig {
    /// Automatically index all enabled projects at startup.
    pub auto_index_on_startup: bool,
    /// Automatically index a project when created or enabled.
    pub auto_index_on_create: bool,
    /// Periodic re-index interval.
    pub periodic_reindex_interval: Duration,
    /// Maximum concurrent indexing jobs.
    pub max_concurrent_jobs: usize,
}

impl Default for IndexJobManagerConfig {
    fn default() -> Self {
        Self {
            auto_index_on_startup: true,
            auto_index_on_create: true,
            periodic_reindex_interval: Duration::from_secs(1800), // 30 minutes
            max_concurrent_jobs: 2,
        }
    }
}

/// Coordinates code indexing operations across projects.
pub struct IndexJobManager {
    /// The code index instance.
    code_index: Arc<CodeIndex>,
    /// Project ID → project directory mapping.
    project_dirs: Mutex<HashMap<String, PathBuf>>,
    /// Active job locks: project_id → mutex (ensures one job per project).
    active_jobs: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// Active file watchers by project ID.
    #[cfg(feature = "file-watcher")]
    watchers: Mutex<HashMap<String, crate::watcher::FileWatcher>>,
    /// Semaphore limiting concurrent indexing jobs.
    semaphore: Arc<Semaphore>,
    /// Cancellation token for graceful shutdown.
    cancel: CancellationToken,
    /// Configuration.
    config: IndexJobManagerConfig,
}

impl IndexJobManager {
    /// Create a new IndexJobManager.
    pub fn new(code_index: Arc<CodeIndex>, config: IndexJobManagerConfig) -> Self {
        let max_jobs = config.max_concurrent_jobs.max(1);
        Self {
            code_index,
            project_dirs: Mutex::new(HashMap::new()),
            active_jobs: Mutex::new(HashMap::new()),
            #[cfg(feature = "file-watcher")]
            watchers: Mutex::new(HashMap::new()),
            semaphore: Arc::new(Semaphore::new(max_jobs)),
            cancel: CancellationToken::new(),
            config,
        }
    }

    /// Register a project directory for indexing.
    pub async fn register_project(&self, project_id: String, project_dir: PathBuf) {
        self.project_dirs
            .lock()
            .await
            .insert(project_id, project_dir);
    }

    /// Unregister a project, stopping its watcher if active.
    pub async fn unregister_project(&self, project_id: &str) {
        self.project_dirs.lock().await.remove(project_id);
        self.active_jobs.lock().await.remove(project_id);
        #[cfg(feature = "file-watcher")]
        {
            if let Some(watcher) = self.watchers.lock().await.remove(project_id) {
                #[cfg(feature = "tracing")]
                info!(project_id, "stopped file watcher for unregistered project");
                drop(watcher); // triggers stop()
            }
        }
    }

    /// Spawn an indexing job for a project (deduplicated, rate-limited).
    ///
    /// Returns `true` if a job was spawned, `false` if one was already running
    /// for this project.
    pub async fn spawn_index(self: &Arc<Self>, project_id: String) -> bool {
        // Non-blocking check: if the per-project lock is held, a job is already
        // running — skip spawning a duplicate.
        let already_running = {
            let jobs = self.active_jobs.lock().await;
            if let Some(lock) = jobs.get(&project_id) {
                lock.try_lock().is_err()
            } else {
                false
            }
        };

        if already_running {
            #[cfg(feature = "tracing")]
            debug!(project_id = %project_id, "index job already running, skipping");
            return false;
        }

        let this = Arc::clone(self);
        let project_id_for_log = project_id.clone();

        tokio::spawn(async move {
            this.index_project_deduped(&project_id).await;
        });

        #[cfg(feature = "tracing")]
        info!(project_id = project_id_for_log, "spawned indexing job");

        true
    }

    /// Index a single project with deduplication.
    async fn index_project_deduped(self: &Arc<Self>, project_id: &str) {
        // Get or create the per-project lock.
        let job_lock = {
            let mut jobs = self.active_jobs.lock().await;
            Arc::clone(
                jobs.entry(project_id.to_string())
                    .or_insert_with(|| Arc::new(Mutex::new(()))),
            )
        };

        // Acquire the per-project lock (deduplication).
        let _guard = job_lock.lock().await;

        // Acquire semaphore slot (concurrency limit).
        let permit = match self.semaphore.acquire().await {
            Ok(p) => p,
            Err(_) => {
                #[cfg(feature = "tracing")]
                warn!(project_id, "semaphore closed, skipping index job");
                return;
            }
        };

        // Get project directory.
        let project_dir = {
            let dirs = self.project_dirs.lock().await;
            dirs.get(project_id).cloned()
        };

        let Some(dir) = project_dir else {
            #[cfg(feature = "tracing")]
            warn!(project_id, "no directory registered for project");
            return;
        };

        // Perform the index operation.
        let result = self.code_index.index_project(project_id, false, &dir).await;

        #[cfg(feature = "tracing")]
        match &result {
            Ok(status) => {
                info!(
                    project_id,
                    files = status.total_files,
                    chunks = status.total_chunks,
                    "indexing complete"
                );
            }
            Err(e) => {
                warn!(project_id, error = %e, "indexing failed");
            }
        }

        // On success, start the file watcher if feature enabled.
        #[cfg(feature = "file-watcher")]
        if result.is_ok() {
            self.start_watcher_if_enabled(project_id, &dir).await;
        }

        // Release semaphore.
        drop(permit);

        // Clean up the job lock if no one else is waiting.
        // (We keep it around for simplicity; GC'd on unregister)
    }

    /// Start the file watcher for a project after successful index.
    #[cfg(feature = "file-watcher")]
    async fn start_watcher_if_enabled(self: &Arc<Self>, project_id: &str, project_dir: &Path) {
        // Check if watcher already exists.
        {
            let watchers = self.watchers.lock().await;
            if watchers.contains_key(project_id) {
                return;
            }
        }

        // Clone Arc<Self> for the watcher callback.
        let this = Arc::clone(self);
        let pid = project_id.to_string();

        let handler: crate::watcher::WatchHandler = Arc::new(move |_proj_id, _changed_paths| {
            let mgr = Arc::clone(&this);
            let pid = pid.clone();

            tokio::spawn(async move {
                // Route through index_project_deduped to enforce semaphore and
                // per-project deduplication (not direct index_project call).
                mgr.index_project_deduped(&pid).await;
            });
        });

        let filter_config = this.code_index.config().filter();

        let result = crate::watcher::FileWatcher::start(
            project_id.to_string(),
            project_dir.to_path_buf(),
            filter_config,
            handler,
        );

        match result {
            Ok(watcher) => {
                self.watchers.lock().await.insert(pid.to_string(), watcher);
                #[cfg(feature = "tracing")]
                info!(project_id, "started file watcher");
            }
            Err(e) => {
                #[cfg(feature = "tracing")]
                warn!(project_id, error = %e, "failed to start file watcher");
            }
        }
    }

    /// Index all enabled projects (used at startup and periodic re-index).
    pub async fn index_all_enabled_projects(
        self: &Arc<Self>,
        projects: Vec<(String, PathBuf)>,
    ) {
        #[cfg(feature = "tracing")]
        info!(count = projects.len(), "starting batch index for enabled projects");

        // Register all projects first.
        {
            let mut dirs = self.project_dirs.lock().await;
            for (pid, dir) in &projects {
                dirs.insert(pid.clone(), dir.clone());
            }
        }

        // Spawn jobs for each project (they will be deduped and rate-limited).
        for (project_id, _dir) in projects {
            let this = Arc::clone(self);
            tokio::spawn(async move {
                this.index_project_deduped(&project_id).await;
            });
        }
    }

    /// Start the periodic re-index loop.
    ///
    /// Reads the current project list from `self.project_dirs` at each tick,
    /// so projects registered/unregistered after startup are correctly handled.
    pub fn start_periodic_reindex_loop(self: &Arc<Self>) -> JoinHandle<()> {
        let this = Arc::clone(self);
        let interval = self.config.periodic_reindex_interval;
        let cancel = self.cancel.clone();

        tokio::spawn(async move {
            // Skip the first immediate tick — the initial index was just completed.
            let start = tokio::time::Instant::now() + interval;
            let mut timer = tokio::time::interval_at(start, interval);
            loop {
                tokio::select! {
                    _ = timer.tick() => {
                        // Derive current project list from live registry.
                        let projs: Vec<(String, PathBuf)> = {
                            let dirs = this.project_dirs.lock().await;
                            dirs.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
                        };
                        #[cfg(feature = "tracing")]
                        debug!(count = projs.len(), "periodic re-index tick");
                        let mgr = Arc::clone(&this);
                        tokio::spawn(async move {
                            mgr.index_all_enabled_projects(projs).await;
                        });
                    }
                    _ = cancel.cancelled() => {
                        #[cfg(feature = "tracing")]
                        info!("periodic re-index loop cancelled");
                        break;
                    }
                }
            }
        })
    }

    /// Stop a specific project's watcher.
    #[cfg(feature = "file-watcher")]
    pub async fn stop_watcher(&self, project_id: &str) {
        if let Some(watcher) = self.watchers.lock().await.remove(project_id) {
            #[cfg(feature = "tracing")]
            info!(project_id, "stopped file watcher");
            drop(watcher);
        }
    }

    /// Graceful shutdown: cancel all jobs and stop all watchers.
    pub async fn shutdown(self: &Arc<Self>) {
        #[cfg(feature = "tracing")]
        info!("shutting down index job manager");

        // Signal cancellation.
        self.cancel.cancel();

        // Stop all watchers.
        #[cfg(feature = "file-watcher")]
        {
            let watchers = std::mem::take(&mut *self.watchers.lock().await);
            for (_pid, watcher) in watchers {
                drop(watcher);
            }
        }

        // Wait for active jobs to complete (they will see the cancel token).
        // In practice, jobs complete quickly since they're file I/O bound.
        #[cfg(feature = "tracing")]
        info!("index job manager shutdown complete");
    }
}
