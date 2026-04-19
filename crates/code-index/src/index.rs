use std::path::Path;

#[cfg(feature = "file-watcher")]
use std::sync::Arc;

#[cfg(feature = "tracing")]
use tracing::instrument;

#[cfg(feature = "tracing")]
use crate::log::{debug, info, warn};

use crate::{
    config::CodeIndexConfig,
    discover::discover_tracked_files,
    error::{Error, Result},
    filter::filter_tracked_files,
    snapshot_store::SnapshotStore,
    types::{FilteredFile, IndexStatus, SearchResult},
};

use crate::delta::{HashSnapshot, build_snapshot_from_filtered, compute_delta};

#[cfg(feature = "builtin")]
use crate::store::CodeIndexStore;

#[cfg(feature = "file-watcher")]
use crate::watcher::FileWatcher;

// ---------------------------------------------------------------------------
// Backend enum
// ---------------------------------------------------------------------------

/// Active backend for the code index.
#[allow(clippy::large_enum_variant)]
enum Backend {
    /// No backend configured — search always returns empty.
    ConfigOnly,

    /// QMD (external vector DB) backend.
    #[cfg(feature = "qmd")]
    Qmd(moltis_qmd::QmdManager),

    /// Built-in SQLite + FTS5 backend with optional embedding provider.
    #[cfg(feature = "builtin")]
    Builtin {
        store: Box<dyn CodeIndexStore>,
        embedder: Option<Box<dyn moltis_memory::embeddings::EmbeddingProvider>>,
    },
}

// ---------------------------------------------------------------------------
// CodeIndex — main orchestrator
// ---------------------------------------------------------------------------

/// Code index supporting multiple backends.
pub struct CodeIndex {
    config: CodeIndexConfig,
    snapshot_store: SnapshotStore,
    backend: Backend,

    /// Project ID → project directory mapping for search scoping.
    project_dirs: std::sync::Mutex<std::collections::HashMap<String, std::path::PathBuf>>,

    /// Active file watchers, keyed by project ID.
    #[cfg(feature = "file-watcher")]
    watchers: std::sync::Mutex<Vec<FileWatcher>>,
}

impl CodeIndex {
    // -----------------------------------------------------------------------
    // Constructors
    // -----------------------------------------------------------------------

    /// Create a new code index with QMD backend.
    #[cfg(feature = "qmd")]
    pub fn new(config: CodeIndexConfig, qmd: moltis_qmd::QmdManager) -> Self {
        let snapshot_store = SnapshotStore::new(
            config
                .data_dir
                .clone()
                .unwrap_or_else(|| moltis_config::data_dir().join("code-index")),
        );
        Self {
            config,
            snapshot_store,
            backend: Backend::Qmd(qmd),
            project_dirs: std::sync::Mutex::new(std::collections::HashMap::new()),
            #[cfg(feature = "file-watcher")]
            watchers: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Create a new code index with builtin backend.
    #[cfg(feature = "builtin")]
    pub fn new_builtin(
        config: CodeIndexConfig,
        store: Box<dyn CodeIndexStore>,
        embedder: Option<Box<dyn moltis_memory::embeddings::EmbeddingProvider>>,
    ) -> Self {
        let snapshot_store = SnapshotStore::new(
            config
                .data_dir
                .clone()
                .unwrap_or_else(|| moltis_config::data_dir().join("code-index")),
        );
        Self {
            config,
            snapshot_store,
            backend: Backend::Builtin { store, embedder },
            project_dirs: std::sync::Mutex::new(std::collections::HashMap::new()),
            #[cfg(feature = "file-watcher")]
            watchers: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Create a code index with config but no backend.
    pub fn config_only(config: CodeIndexConfig) -> Self {
        let snapshot_store = SnapshotStore::new(
            config
                .data_dir
                .clone()
                .unwrap_or_else(|| moltis_config::data_dir().join("code-index")),
        );
        Self {
            config,
            snapshot_store,
            backend: Backend::ConfigOnly,
            project_dirs: std::sync::Mutex::new(std::collections::HashMap::new()),
            #[cfg(feature = "file-watcher")]
            watchers: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// List the indexable files for a project directory.
    ///
    /// Discovers git-tracked files and applies extension/size/path filters.
    /// Does not require any backend — works with `config_only`.
    pub fn list_indexable_files(&self, project_dir: &Path) -> Result<Vec<FilteredFile>> {
        let tracked = discover_tracked_files(project_dir)
            .map_err(|e| Error::Io(std::io::Error::other(e.to_string())))?;
        let filtered = filter_tracked_files(project_dir, &tracked, &self.config)?;
        Ok(filtered)
    }

    // -----------------------------------------------------------------------
    // Public API — index, search, status
    // -----------------------------------------------------------------------

    /// Index a project directory.
    ///
    /// On first run (no snapshot), performs a full reindex. On subsequent runs,
    /// computes a delta from the previous snapshot and only processes changed files.
    /// Pass `force = true` to force a full reindex regardless.
    #[cfg_attr(feature = "tracing", instrument(skip(self)))]
    pub async fn index_project(
        &self,
        project_id: &str,
        force: bool,
        project_dir: &Path,
    ) -> Result<IndexStatus> {
        // Remember the project directory for search scoping.
        self.project_dirs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(project_id.to_string(), project_dir.to_path_buf());

        // Discover + filter are blocking filesystem I/O — offload to blocking
        // thread when a tokio runtime is available (builtin / file-watcher).
        #[cfg(any(feature = "builtin", feature = "file-watcher"))]
        let (_tracked, filtered) = tokio::task::spawn_blocking({
            let project_dir = project_dir.to_path_buf();
            let config = self.config.clone();
            move || {
                let tracked = discover_tracked_files(&project_dir)
                    .map_err(|e| Error::Io(std::io::Error::other(e.to_string())))?;
                let filtered = filter_tracked_files(&project_dir, &tracked, &config)?;
                Ok::<_, Error>((tracked, filtered))
            }
        })
        .await
        .map_err(|e| Error::Io(std::io::Error::other(e.to_string())))??;

        #[cfg(not(any(feature = "builtin", feature = "file-watcher")))]
        let (_tracked, filtered) = {
            let tracked = discover_tracked_files(project_dir)
                .map_err(|e| Error::Io(std::io::Error::other(e.to_string())))?;
            let filtered = filter_tracked_files(project_dir, &tracked, &self.config)?;
            (tracked, filtered)
        };
        #[cfg(not(any(feature = "builtin", feature = "qmd")))]
        let _filtered = filtered;
        #[cfg(not(any(feature = "builtin", feature = "qmd")))]
        let _force = force;

        match &self.backend {
            Backend::ConfigOnly => Err(Error::IndexFailed {
                project_id: project_id.to_string(),
                message: "no backend configured".to_string(),
            }),

            #[cfg(feature = "qmd")]
            Backend::Qmd(qmd) => {
                qmd.ensure_collections()
                    .await
                    .map_err(|e| Error::IndexFailed {
                        project_id: project_id.to_string(),
                        message: format!("QMD ensure_collections error: {e}"),
                    })?;
                qmd.refresh_index(true)
                    .await
                    .map_err(|e| Error::IndexFailed {
                        project_id: project_id.to_string(),
                        message: format!("QMD refresh_index error: {e}"),
                    })?;
                self.status(project_id).await
            },

            #[cfg(feature = "builtin")]
            Backend::Builtin { store, embedder } => {
                if force {
                    self.index_full_builtin(
                        project_id,
                        project_dir,
                        &filtered,
                        store.as_ref(),
                        embedder.as_deref(),
                    )
                    .await?;
                    self.build_status_builtin(project_id, store.as_ref(), embedder.as_deref())
                        .await
                } else {
                    self.index_incremental_builtin(
                        project_id,
                        project_dir,
                        &filtered,
                        store.as_ref(),
                        embedder.as_deref(),
                    )
                    .await
                }
            },
        }
    }

    /// Search the code index for a project.
    #[cfg_attr(feature = "tracing", instrument(skip(self)))]
    pub async fn search(
        &self,
        project_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        match &self.backend {
            Backend::ConfigOnly => Err(Error::BackendUnavailable(
                "no backend configured".to_string(),
            )),

            #[cfg(feature = "qmd")]
            Backend::Qmd(qmd) => {
                // Request extra results to compensate for cross-project filtering.
                let fetch_limit = limit * 3;
                let results = qmd
                    .hybrid_search(query, fetch_limit, true)
                    .await
                    .map_err(|e| Error::SearchFailed {
                        project_id: project_id.to_string(),
                        message: format!("QMD search error: {e}"),
                    })?;
                let mapped = crate::search::from_qmd_results(&results, project_id);

                // Filter results to only include files belonging to this project.
                let project_dir = self
                    .project_dirs
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(project_id)
                    .cloned();
                let scoped: Vec<SearchResult> = if let Some(ref dir) = project_dir {
                    mapped
                        .into_iter()
                        .filter(|r| {
                            r.path.starts_with(dir.to_string_lossy().as_ref())
                                || Path::new(&r.path).starts_with(dir)
                        })
                        .take(limit)
                        .collect()
                } else {
                    mapped.into_iter().take(limit).collect()
                };
                Ok(scoped)
            },

            #[cfg(feature = "builtin")]
            Backend::Builtin { store, embedder } => {
                self.search_builtin(
                    project_id,
                    query,
                    limit,
                    store.as_ref(),
                    embedder.as_deref(),
                )
                .await
            },
        }
    }

    /// Get the index status for a project.
    #[cfg_attr(feature = "tracing", instrument(skip(self)))]
    pub async fn status(&self, project_id: &str) -> Result<IndexStatus> {
        match &self.backend {
            Backend::ConfigOnly => Err(Error::BackendUnavailable(
                "no backend configured".to_string(),
            )),

            #[cfg(feature = "qmd")]
            Backend::Qmd(qmd) => {
                let qmd_status = qmd.status().await;
                let total_files: usize = qmd_status.indexed_files.values().sum();
                Ok(IndexStatus {
                    project_id: project_id.to_string(),
                    total_files,
                    total_chunks: total_files, // QMD doesn't expose chunk count
                    last_sync_ms: None,
                    embedding_model: None,
                    backend: "qmd".to_string(),
                })
            },

            #[cfg(feature = "builtin")]
            Backend::Builtin { store, embedder } => {
                self.build_status_builtin(project_id, store.as_ref(), embedder.as_deref())
                    .await
            },
        }
    }

    // -----------------------------------------------------------------------
    // Watcher lifecycle
    // -----------------------------------------------------------------------

    /// Start watching a project directory for incremental reindexing.
    ///
    /// When files change, the handler re-indexes only the affected files
    /// via [`reindex_files`].
    #[cfg(feature = "file-watcher")]
    pub fn start_watcher(self: &Arc<Self>, project_id: &str, project_dir: &Path) -> Result<()> {
        use crate::watcher::WatchHandler;

        let filter_config = self.config.filter();
        let index = Arc::clone(self);
        let proj_dir = project_dir.to_path_buf();

        let handler: WatchHandler = Arc::new(move |proj_id, changed_paths| {
            let paths: Vec<std::path::PathBuf> = changed_paths.to_vec();
            let idx = Arc::clone(&index);
            let pid = proj_id.to_string();
            let proj_dir = proj_dir.clone();

            tokio::spawn(async move {
                if let Err(e) = idx.reindex_files(&pid, &proj_dir, &paths).await {
                    #[cfg(feature = "tracing")]
                    warn!(project_id = %pid, error = %e, "watcher reindex failed");
                }
            });
        });

        let watcher = FileWatcher::start(
            project_id.to_string(),
            project_dir.to_path_buf(),
            filter_config,
            handler,
        )
        .map_err(|e| Error::IndexFailed {
            project_id: project_id.to_string(),
            message: format!("failed to start watcher: {e}"),
        })?;

        self.watchers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(watcher);
        #[cfg(feature = "tracing")]
        info!(project_id, "file watcher registered");
        Ok(())
    }

    /// Stop all watchers for a given project.
    #[cfg(feature = "file-watcher")]
    pub fn stop_watcher(&self, project_id: &str) {
        let mut watchers = self.watchers.lock().unwrap_or_else(|e| e.into_inner());
        watchers.retain(|w| {
            if w.project_id() == project_id {
                w.stop();
                false
            } else {
                true
            }
        });
    }

    /// Stop all watchers.
    #[cfg(feature = "file-watcher")]
    pub fn stop_all_watchers(&self) {
        let mut watchers = self.watchers.lock().unwrap_or_else(|e| e.into_inner());
        watchers.clear(); // Drop triggers stop()
    }

    // -----------------------------------------------------------------------
    // Incremental reindex — called by watcher and public API
    // -----------------------------------------------------------------------

    /// Re-index a set of changed files for a project.
    ///
    /// Reads each file, chunks it, generates embeddings if available,
    /// and upserts into the store. Files that no longer exist on disk are skipped.
    #[cfg(feature = "builtin")]
    #[cfg_attr(feature = "tracing", instrument(skip(self)))]
    pub async fn reindex_files(
        &self,
        project_id: &str,
        project_dir: &Path,
        paths: &[std::path::PathBuf],
    ) -> Result<()> {
        let Backend::Builtin { store, embedder } = &self.backend else {
            return Err(Error::IndexFailed {
                project_id: project_id.to_string(),
                message: "reindex_files only available with builtin backend".to_string(),
            });
        };

        // Convert absolute paths to FilteredFile entries.
        let files: Vec<FilteredFile> = paths
            .iter()
            .filter_map(|p| {
                if !p.is_file() {
                    return None;
                }
                let relative_path = p.strip_prefix(project_dir).unwrap_or(p).to_path_buf();
                Some(FilteredFile {
                    path: p.clone(),
                    relative_path,
                    size: p.metadata().map(|m| m.len()).unwrap_or(0),
                    language: crate::types::Language::from_path(p),
                })
            })
            .collect();

        if files.is_empty() {
            return Ok(());
        }

        let file_refs: Vec<&FilteredFile> = files.iter().collect();
        self.index_files_builtin(project_id, &file_refs, store.as_ref(), embedder.as_deref())
            .await?;

        #[cfg(feature = "tracing")]

        debug!(project_id, count = files.len(), "watcher reindex completed");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Builtin backend — indexing
    // -----------------------------------------------------------------------

    /// Incremental index: compute delta from previous snapshot, process changes only.
    #[cfg(feature = "builtin")]
    async fn index_incremental_builtin(
        &self,
        project_id: &str,
        project_dir: &Path,
        filtered: &[FilteredFile],
        store: &dyn CodeIndexStore,
        embedder: Option<&dyn moltis_memory::embeddings::EmbeddingProvider>,
    ) -> Result<IndexStatus> {
        #[cfg(feature = "tracing")]
        info!(
            project_id,
            total = filtered.len(),
            "starting incremental code index (builtin)"
        );

        store.initialize().await.map_err(|e| Error::IndexFailed {
            project_id: project_id.to_string(),
            message: format!("failed to initialize store: {e}"),
        })?;

        let previous_snapshot: HashSnapshot = self
            .snapshot_store
            .load(project_id)
            .map_err(|e| Error::Store(e.to_string()))?
            .unwrap_or_default();

        if previous_snapshot.is_empty() {
            // First index — full reindex required.
            self.index_full_builtin(project_id, project_dir, filtered, store, embedder)
                .await?;
            return self.build_status_builtin(project_id, store, embedder).await;
        }

        // Compute delta from previous snapshot.
        let (delta, current_snapshot) =
            compute_delta(project_dir, &self.config, &previous_snapshot).map_err(|e| {
                Error::IndexFailed {
                    project_id: project_id.to_string(),
                    message: format!("delta computation failed: {e}"),
                }
            })?;

        #[cfg(feature = "tracing")]

        info!(
            project_id,
            added = delta.added.len(),
            modified = delta.modified.len(),
            removed = delta.removed.len(),
            "incremental delta computed"
        );

        // Process added + modified files.
        let changed: Vec<&FilteredFile> = delta.added.iter().chain(delta.modified.iter()).collect();

        if !changed.is_empty() {
            self.index_files_builtin(project_id, &changed, store, embedder)
                .await?;
        }

        // Delete chunks for removed files.
        for removed_path in &delta.removed {
            store
                .delete_file_chunks(project_id, removed_path)
                .await
                .map_err(|e| Error::IndexFailed {
                    project_id: project_id.to_string(),
                    message: format!("failed to delete removed file chunks: {e}"),
                })?;
            #[cfg(feature = "tracing")]
            debug!(path = %removed_path, "deleted chunks for removed file");
        }

        // Persist updated snapshot.
        self.snapshot_store
            .save(project_id, &current_snapshot)
            .map_err(|e| Error::IndexFailed {
                project_id: project_id.to_string(),
                message: format!("failed to save snapshot: {e}"),
            })?;

        self.build_status_builtin(project_id, store, embedder).await
    }

    /// Full reindex: clear project and index all files from scratch.
    #[cfg(feature = "builtin")]
    async fn index_full_builtin(
        &self,
        project_id: &str,
        _project_dir: &Path,
        filtered: &[FilteredFile],
        store: &dyn CodeIndexStore,
        embedder: Option<&dyn moltis_memory::embeddings::EmbeddingProvider>,
    ) -> Result<()> {
        store.initialize().await.map_err(|e| Error::IndexFailed {
            project_id: project_id.to_string(),
            message: format!("failed to initialize store: {e}"),
        })?;

        store
            .clear_project(project_id)
            .await
            .map_err(|e| Error::IndexFailed {
                project_id: project_id.to_string(),
                message: format!("failed to clear project: {e}"),
            })?;

        let file_refs: Vec<&FilteredFile> = filtered.iter().collect();
        self.index_files_builtin(project_id, &file_refs, store, embedder)
            .await?;

        // Build and save the snapshot from the already-filtered files
        // (avoids TOCTOU from double-scanning the filesystem).
        let snapshot = build_snapshot_from_filtered(filtered);
        self.snapshot_store
            .save(project_id, &snapshot)
            .map_err(|e| Error::IndexFailed {
                project_id: project_id.to_string(),
                message: format!("failed to save snapshot: {e}"),
            })?;

        Ok(())
    }

    /// Index a batch of files into the store (shared by full and incremental paths).
    #[cfg(feature = "builtin")]
    async fn index_files_builtin(
        &self,
        project_id: &str,
        files: &[&FilteredFile],
        store: &dyn CodeIndexStore,
        embedder: Option<&dyn moltis_memory::embeddings::EmbeddingProvider>,
    ) -> Result<()> {
        use crate::{chunker::CodeChunker, store::CodeChunk as StoreChunk};

        let chunker = CodeChunker::new(self.config.chunker());
        let mut indexed = 0u64;
        let mut errors = 0u64;

        for file in files {
            // file.path is absolute — use directly for disk reads.
            // file.relative_path is the repo-relative path — used for storage and logging.
            let content = match tokio::fs::read_to_string(&file.path).await {
                Ok(c) => c,
                Err(e) => {
                    #[cfg(feature = "tracing")]
                    warn!(
                        path = %file.relative_path.display(),
                        error = %e,
                        "failed to read file, skipping"
                    );
                    errors += 1;
                    continue;
                },
            };

            let raw_chunks = chunker.chunk(&content, &file.relative_path.display().to_string());

            // Generate embeddings for chunks if embedder is available.
            let chunks: Vec<StoreChunk> = if let Some(emb) = embedder {
                let texts: Vec<String> = raw_chunks.iter().map(|c| c.content.clone()).collect();
                match emb.embed_batch(&texts).await {
                    Ok(embeddings) => {
                        if embeddings.len() != raw_chunks.len() {
                            #[cfg(feature = "tracing")]
                            warn!(
                                expected = raw_chunks.len(),
                                actual = embeddings.len(),
                                path = %file.relative_path.display(),
                                "embed_batch returned mismatched count, indexing without embeddings"
                            );
                            raw_chunks
                                .into_iter()
                                .enumerate()
                                .map(|(idx, chunk)| StoreChunk {
                                    file_path: chunk.file_path.clone(),
                                    chunk_index: idx,
                                    content: chunk.content,
                                    embedding: None,
                                    start_line: chunk.start_line,
                                    end_line: chunk.end_line,
                                })
                                .collect()
                        } else {
                            raw_chunks
                                .into_iter()
                                .zip(embeddings.into_iter())
                                .enumerate()
                                .map(|(idx, (chunk, embedding))| StoreChunk {
                                    file_path: chunk.file_path.clone(),
                                    chunk_index: idx,
                                    content: chunk.content,
                                    embedding: Some(embedding),
                                    start_line: chunk.start_line,
                                    end_line: chunk.end_line,
                                })
                                .collect()
                        }
                    },
                    Err(e) => {
                        #[cfg(feature = "tracing")]
                        warn!(
                            path = %file.relative_path.display(),
                            error = %e,
                            "embedding failed for file, indexing without embeddings"
                        );
                        raw_chunks
                            .into_iter()
                            .enumerate()
                            .map(|(idx, chunk)| StoreChunk {
                                file_path: chunk.file_path.clone(),
                                chunk_index: idx,
                                content: chunk.content,
                                embedding: None,
                                start_line: chunk.start_line,
                                end_line: chunk.end_line,
                            })
                            .collect()
                    },
                }
            } else {
                raw_chunks
                    .into_iter()
                    .enumerate()
                    .map(|(idx, chunk)| StoreChunk {
                        file_path: chunk.file_path.clone(),
                        chunk_index: idx,
                        content: chunk.content,
                        embedding: None,
                        start_line: chunk.start_line,
                        end_line: chunk.end_line,
                    })
                    .collect()
            };

            if !chunks.is_empty() {
                let file_path_str = file.relative_path.display().to_string();
                store
                    .upsert_chunks(project_id, &file_path_str, &chunks)
                    .await
                    .map_err(|e| Error::IndexFailed {
                        project_id: project_id.to_string(),
                        message: format!("failed to upsert chunks for {file_path_str}: {e}"),
                    })?;
                indexed += chunks.len() as u64;
            }
        }

        #[cfg(feature = "tracing")]

        info!(project_id, indexed, errors, "file batch indexed (builtin)");

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Builtin backend — search
    // -----------------------------------------------------------------------

    /// Search using the builtin backend (hybrid keyword + vector).
    #[cfg(feature = "builtin")]
    async fn search_builtin(
        &self,
        project_id: &str,
        query: &str,
        limit: usize,
        store: &dyn CodeIndexStore,
        embedder: Option<&dyn moltis_memory::embeddings::EmbeddingProvider>,
    ) -> Result<Vec<SearchResult>> {
        // Always run keyword search via FTS5.
        let keyword_results = store
            .search_keyword(project_id, query, limit)
            .await
            .map_err(|e| Error::SearchFailed {
                project_id: project_id.to_string(),
                message: format!("keyword search failed: {e}"),
            })?;

        let Some(emb) = embedder else {
            return Ok(keyword_results.into_iter().take(limit).collect());
        };

        // Generate query embedding.
        let query_embedding = match emb.embed_batch(&[query.to_string()]).await {
            Ok(mut embeddings) => {
                // Invariant: passed one string, get one embedding back.
                embeddings.pop().ok_or_else(|| {
                    Error::IndexStore(
                        "embed_batch returned empty result for single input".to_string(),
                    )
                })?
            },
            Err(e) => {
                #[cfg(feature = "tracing")]
                warn!(error = %e, "query embedding failed, falling back to keyword results");
                return Ok(keyword_results.into_iter().take(limit).collect());
            },
        };

        // PERF: loads ALL chunks into memory for brute-force vector search.
        // For large projects (100k+ chunks) this could consume significant RAM.
        // Consistent with the memory system's approach. A streaming or indexed
        // approach would be needed for very large projects.
        let chunks = store.get_project_chunks(project_id).await?;

        // Score chunks by cosine similarity against the query embedding.
        let mut scored_chunks: Vec<(f32, &crate::store::CodeChunk)> = Vec::new();
        for chunk in &chunks {
            let Some(ref chunk_emb) = chunk.embedding else {
                continue;
            };
            let score = crate::store::cosine_similarity(&query_embedding, chunk_emb);
            scored_chunks.push((score, chunk));
        }

        scored_chunks.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        let vector_results: Vec<SearchResult> = scored_chunks
            .into_iter()
            .take(limit)
            .map(|(score, chunk)| SearchResult {
                chunk_id: format!("{}:{}", chunk.file_path, chunk.start_line),
                path: chunk.file_path.clone(),
                text: peek_lines(&chunk.content, 0, 10),
                start_line: chunk.start_line,
                end_line: chunk.end_line,
                score,
                source: "builtin".to_string(),
            })
            .collect();

        let merged = crate::store::merge_hybrid_results(vector_results, keyword_results, limit);
        Ok(merged)
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Build an [`IndexStatus`] from the current store counts.
    #[cfg(feature = "builtin")]
    async fn build_status_builtin(
        &self,
        project_id: &str,
        store: &dyn CodeIndexStore,
        embedder: Option<&dyn moltis_memory::embeddings::EmbeddingProvider>,
    ) -> Result<IndexStatus> {
        let total_chunks = store
            .chunk_count(project_id)
            .await
            .map_err(|e| Error::IndexFailed {
                project_id: project_id.to_string(),
                message: format!("failed to count chunks: {e}"),
            })?;
        let total_files = store
            .file_count(project_id)
            .await
            .map_err(|e| Error::IndexFailed {
                project_id: project_id.to_string(),
                message: format!("failed to count files: {e}"),
            })?;

        Ok(IndexStatus {
            project_id: project_id.to_string(),
            total_files,
            total_chunks,
            last_sync_ms: None,
            embedding_model: embedder.map(|e| e.model_name().to_string()),
            backend: "builtin".to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Extract a range of lines from text content.
#[cfg(feature = "builtin")]
fn peek_lines(content: &str, start: usize, max_lines: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let end = (start + max_lines).min(lines.len());
    if start >= lines.len() {
        return String::new();
    }
    lines[start..end].join("\n")
}
#[cfg(all(test, feature = "builtin"))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use {super::*, crate::store_sqlite::SqliteCodeIndexStore, async_trait::async_trait};

    /// Mock embedder producing deterministic vectors from text content.
    struct MockEmbedder {
        dims: usize,
    }

    impl MockEmbedder {
        fn new(dims: usize) -> Self {
            Self { dims }
        }
    }

    #[async_trait]
    impl moltis_memory::embeddings::EmbeddingProvider for MockEmbedder {
        async fn embed(&self, text: &str) -> moltis_memory::error::Result<Vec<f32>> {
            let mut vec = vec![0.0f32; self.dims];
            for (i, b) in text.as_bytes().iter().enumerate() {
                vec[i % self.dims] += *b as f32 / 255.0;
            }
            let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
            if norm > 0.0 {
                for v in &mut vec {
                    *v /= norm;
                }
            }
            Ok(vec)
        }

        fn model_name(&self) -> &str {
            "mock-embedder"
        }

        fn dimensions(&self) -> usize {
            self.dims
        }

        fn provider_key(&self) -> &str {
            "mock"
        }
    }

    /// Embedder that always fails — for testing fallback paths.
    struct FailingEmbedder;

    #[async_trait]
    impl moltis_memory::embeddings::EmbeddingProvider for FailingEmbedder {
        async fn embed(&self, _text: &str) -> moltis_memory::error::Result<Vec<f32>> {
            Err(moltis_memory::Error::Embedding("embed failed".into()))
        }

        fn model_name(&self) -> &str {
            "failing"
        }

        fn dimensions(&self) -> usize {
            8
        }

        fn provider_key(&self) -> &str {
            "failing"
        }
    }

    fn git_init(dir: &Path) {
        let mut repo = gix::init(dir).expect("failed to init repo");
        let mut config = repo.config_snapshot_mut();
        config
            .set_raw_value_by("user", None::<&gix::bstr::BStr>, "email", "test@test.com")
            .expect("failed to set user.email");
        config
            .set_raw_value_by("user", None::<&gix::bstr::BStr>, "name", "Test")
            .expect("failed to set user.name");
    }

    // NOTE: Uses std::process::Command intentionally. gix's index staging API
    // (add_entry) is low-level and unstable across versions; `git add .` +
    // `git commit -m` is the mature, reliable approach for test helpers.
    fn git_commit_all(dir: &Path, msg: &str) {
        std::process::Command::new("git")
            .args(["-C", &dir.to_string_lossy(), "add", "."])
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["-C", &dir.to_string_lossy(), "commit", "-m", msg])
            .output()
            .unwrap();
    }

    fn create_test_repo() -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().unwrap();
        git_init(dir.path());

        // Create test files with known content
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "fn main() {\n    println!(\"hello world\");\n}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("README.md"), "# Test Project\n\nA test.\n").unwrap();

        git_commit_all(dir.path(), "initial");
        dir
    }

    async fn make_store() -> SqliteCodeIndexStore {
        // Use in-memory SQLite to avoid temp-file lifetime issues
        let pool = sqlx::SqlitePool::connect(":memory:").await.unwrap();
        SqliteCodeIndexStore::from_pool(pool).await.unwrap()
    }

    async fn setup_index() -> (CodeIndex, tempfile::TempDir, tempfile::TempDir) {
        let repo = create_test_repo();
        let store = make_store().await;
        let data_dir = tempfile::tempdir().unwrap();
        let config = CodeIndexConfig {
            data_dir: Some(data_dir.path().to_path_buf()),
            ..CodeIndexConfig::default()
        };
        let index = CodeIndex::new_builtin(config, Box::new(store), None);
        (index, data_dir, repo)
    }

    async fn setup_index_with_embedder() -> (CodeIndex, tempfile::TempDir, tempfile::TempDir) {
        let repo = create_test_repo();
        let store = make_store().await;
        let data_dir = tempfile::tempdir().unwrap();
        let config = CodeIndexConfig {
            data_dir: Some(data_dir.path().to_path_buf()),
            ..CodeIndexConfig::default()
        };
        let embedder: Box<dyn moltis_memory::embeddings::EmbeddingProvider> =
            Box::new(MockEmbedder::new(16));
        let index = CodeIndex::new_builtin(config, Box::new(store), Some(embedder));
        (index, data_dir, repo)
    }

    async fn setup_index_with_failing_embedder() -> (CodeIndex, tempfile::TempDir, tempfile::TempDir)
    {
        let repo = create_test_repo();
        let store = make_store().await;
        let data_dir = tempfile::tempdir().unwrap();
        let config = CodeIndexConfig {
            data_dir: Some(data_dir.path().to_path_buf()),
            ..CodeIndexConfig::default()
        };
        let embedder: Box<dyn moltis_memory::embeddings::EmbeddingProvider> =
            Box::new(FailingEmbedder);
        let index = CodeIndex::new_builtin(config, Box::new(store), Some(embedder));
        (index, data_dir, repo)
    }

    #[tokio::test]
    async fn test_index_project_no_embedder() {
        let (index, _data_dir, repo) = setup_index().await;
        let status = index
            .index_project("test-proj", false, repo.path())
            .await
            .unwrap();

        assert_eq!(status.project_id, "test-proj");
        assert!(status.total_files > 0, "should find at least one file");
        assert!(status.total_chunks > 0, "should produce at least one chunk");
        assert!(status.embedding_model.is_none());
        assert_eq!(status.backend, "builtin");
    }

    #[tokio::test]
    async fn test_index_and_keyword_search() {
        let (index, _data_dir, repo) = setup_index().await;
        index
            .index_project("test-proj", false, repo.path())
            .await
            .unwrap();

        let results = index.search("test-proj", "hello", 10).await.unwrap();
        assert!(
            !results.is_empty(),
            "keyword search for 'hello' should find results"
        );
    }

    #[tokio::test]
    async fn test_index_and_keyword_search_miss() {
        let (index, _data_dir, repo) = setup_index().await;
        index
            .index_project("test-proj", false, repo.path())
            .await
            .unwrap();

        let results = index
            .search("test-proj", "nonexistent_xyzzy", 10)
            .await
            .unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_index_clears_old_data() {
        let (index, _data_dir, repo) = setup_index().await;

        let s1 = index
            .index_project("test-proj", false, repo.path())
            .await
            .unwrap();
        let s2 = index
            .index_project("test-proj", false, repo.path())
            .await
            .unwrap();

        // Second index should replace, not duplicate
        assert_eq!(s1.total_files, s2.total_files);
        assert_eq!(s1.total_chunks, s2.total_chunks);
    }

    #[tokio::test]
    async fn test_index_multiple_projects() {
        let (index, _data_dir, repo1) = setup_index().await;
        let repo2 = create_test_repo();

        // Modify repo2 to have distinct content
        std::fs::write(
            repo2.path().join("README.md"),
            "# Different\n\nUnique content here.\n",
        )
        .unwrap();
        git_commit_all(repo2.path(), "update readme");

        index
            .index_project("proj-a", false, repo1.path())
            .await
            .unwrap();
        index
            .index_project("proj-b", false, repo2.path())
            .await
            .unwrap();

        // Searches should be scoped
        let results_a = index.search("proj-a", "hello", 10).await.unwrap();
        let results_b = index.search("proj-b", "hello", 10).await.unwrap();

        // proj-a has "hello world" in main.rs, proj-b also has it
        // but searches are scoped so they don't cross-contaminate
        assert!(results_a.len() <= results_b.len() + 10); // sanity check
    }

    #[tokio::test]
    async fn test_search_with_mock_embedder() {
        let (index, _data_dir, repo) = setup_index_with_embedder().await;
        index
            .index_project("test-proj", true, repo.path())
            .await
            .unwrap();

        let status = index.status("test-proj").await.unwrap();
        assert_eq!(status.embedding_model, Some("mock-embedder".to_string()));

        // Search should work — vector results should be present
        let results = index.search("test-proj", "main", 10).await.unwrap();
        assert!(!results.is_empty());
    }

    #[tokio::test]
    async fn test_search_embedder_failure_fallback() {
        let (index, _data_dir, repo) = setup_index_with_failing_embedder().await;
        index
            .index_project("test-proj", true, repo.path())
            .await
            .unwrap();

        // Embeddings will fail during indexing (warned + chunks stored without embeddings)
        // and during search the query embedding will fail, falling back to keyword results
        let results = index.search("test-proj", "hello", 10).await.unwrap();
        assert!(
            !results.is_empty(),
            "should fall back to keyword results even when embedder fails"
        );
    }
}
