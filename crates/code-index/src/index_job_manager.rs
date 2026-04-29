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
use crate::log::{debug, info, warn};

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
    /// Spawned job handles for shutdown tracking.
    job_handles: Mutex<Vec<JoinHandle<()>>>,
    /// Periodic re-index loop handle for graceful shutdown.
    periodic_loop_handle: Mutex<Option<JoinHandle<()>>>,
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
            job_handles: Mutex::new(Vec::new()),
            periodic_loop_handle: Mutex::new(None),
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

    /// Get a reference to the configuration.
    pub fn config(&self) -> &IndexJobManagerConfig {
        &self.config
    }

    /// Spawn an indexing job for a project (deduplicated, rate-limited).
    ///
    /// Returns `true` if a job was spawned, `false` if one was already running.
    ///
    /// Uses `try_lock()` on the per-project mutex to detect concurrent requests.
    /// If the lock is already held, another job is running and we skip spawning.
    pub async fn spawn_index(self: &Arc<Self>, project_id: String) -> bool {
        // Get or create the per-project lock.
        let job_lock = {
            let mut jobs = self.active_jobs.lock().await;
            Arc::clone(
                jobs.entry(project_id.clone())
                    .or_insert_with(|| Arc::new(Mutex::new(()))),
            )
        };

        // Try to acquire the lock without waiting.
        // If already locked, another job is running — skip spawning.
        let _guard = match job_lock.try_lock() {
            Some(g) => g,
            None => {
                #[cfg(feature = "tracing")]
                debug!(project_id = %project_id, "index job already running, skipping");
                return false;
            }
        };

        let this = Arc::clone(self);
        let project_id_for_log = project_id.clone();

        let handle = tokio::spawn(async move {
            // Run the index job.
            this.index_project_deduped(&project_id).await;
        });

        // Track the handle for shutdown.
        // Purge completed handles before adding to prevent unbounded growth.
        {
            let mut handles = self.job_handles.lock().await;
            handles.retain(|h| !h.is_finished());
            handles.push(handle);
        }

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
        // Check if project is still registered (may have been unregistered during indexing).
        {
            let dirs = self.project_dirs.lock().await;
            if !dirs.contains_key(project_id) {
                #[cfg(feature = "tracing")]
                debug!(project_id, "project unregistered, skipping watcher startup");
                return;
            }
        }

        // Check if watcher already exists.
        {
            let watchers = self.watchers.lock().await;
            if watchers.contains_key(project_id) {
                return;
            }
        }

        // Compute filter_config before the closure consumes `this` and `pid`.
        let filter_config = self.code_index.config().filter();

        // Clone Arc<Self> and pid for the watcher callback.
        let this = Arc::clone(self);
        let pid_for_cb = project_id.to_string(); // captured by closure

        let handler: crate::watcher::WatchHandler = Arc::new(move |_proj_id, _changed_paths| {
            let mgr = Arc::clone(&this);
            let pid = pid_for_cb.clone();

            // File watcher callbacks run on a blocking thread pool, so we must spawn
            // to return to the tokio runtime. The spawned task calls spawn_index(),
            // which handles deduplication and shutdown tracking.
            tokio::spawn(async move {
                let _ = mgr.spawn_index(pid).await;
            });
        });

        let result = crate::watcher::FileWatcher::start(
            project_id.to_string(),
            project_dir.to_path_buf(),
            filter_config,
            handler,
        );

        match result {
            Ok(watcher) => {
                self.watchers.lock().await.insert(project_id.to_string(), watcher);
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
    ///
    /// Reads the current project list from `self.project_dirs` and routes all
    /// jobs through `spawn_index()` to ensure proper deduplication, rate limiting,
    /// and shutdown tracking.
    pub async fn index_all_enabled_projects(self: &Arc<Self>) {
        // Read current project list from registry.
        let projects: Vec<(String, PathBuf)> = {
            let dirs = self.project_dirs.lock().await;
            dirs.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        };

        #[cfg(feature = "tracing")]
        info!(count = projects.len(), "starting batch index for enabled projects");

        // Call spawn_index for each project. Jobs are tracked via job_handles
        // inside spawn_index() for proper shutdown synchronization.
        for (project_id, _dir) in projects {
            self.spawn_index(project_id).await;
        }
    }

    /// Start the periodic re-index loop.
    ///
    /// Reads the current project list from `self.project_dirs` at each tick,
    /// so projects registered/unregistered after startup are correctly handled.
    ///
    /// The returned `JoinHandle` should be stored in `IndexJobManager` via
    /// `set_periodic_loop_handle()` to ensure it is awaited during shutdown.
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
                        #[cfg(feature = "tracing")]
                        debug!("periodic re-index tick");
                        this.index_all_enabled_projects().await;
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

    /// Store the periodic re-index loop handle for shutdown tracking.
    pub async fn set_periodic_loop_handle(&self, handle: JoinHandle<()>) {
        *self.periodic_loop_handle.lock().await = Some(handle);
    }

    /// Graceful shutdown: cancel all jobs and wait for active jobs to complete.
    pub async fn shutdown(self: &Arc<Self>) {
        #[cfg(feature = "tracing")]
        info!("shutting down index job manager");

        // Signal cancellation to periodic loop and watchers.
        self.cancel.cancel();

        // Stop all watchers.
        #[cfg(feature = "file-watcher")]
        {
            let watchers = std::mem::take(&mut *self.watchers.lock().await);
            for (_pid, watcher) in watchers {
                drop(watcher);
            }
        }

        // Wait for all spawned jobs to complete.
        let handles = {
            let mut h = self.job_handles.lock().await;
            std::mem::take(&mut *h)
        };
        for handle in handles {
            let _ = handle.await;
        }

        // Wait for the periodic re-index loop to complete.
        if let Some(loop_handle) = self.periodic_loop_handle.lock().await.take() {
            let _ = loop_handle.await;
        }

        #[cfg(feature = "tracing")]
        info!("index job manager shutdown complete");
    }
}
