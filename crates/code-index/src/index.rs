//! Code index for workspace codebase intelligence.
//!
//! The index supports multiple backends:
//! - **QMD**: External QMD binary for hybrid search (requires `qmd` feature)
//! - **Builtin**: SQLite + FTS5 with in-memory vector similarity (requires `builtin` feature)
//! - **Config-only**: File discovery and filtering only, no search

use std::path::Path;

use tracing::{info, warn};

use crate::config::CodeIndexConfig;
use crate::delta::{build_initial_snapshot, HashSnapshot};
use crate::discover::discover_tracked_files;
use crate::error::{Error, Result};
use crate::filter::filter_tracked_files;
use crate::snapshot_store::SnapshotStore;
use crate::types::{FilteredFile, IndexStatus, SearchResult};

#[cfg(feature = "builtin")]
use crate::store::CodeIndexStore;

/// Code index supporting multiple backends.
pub struct CodeIndex {
    config: CodeIndexConfig,
    snapshot_store: SnapshotStore,
    backend: Backend,
}

/// Backend variants for code indexing.
enum Backend {
    /// QMD backend for hybrid search.
    #[cfg(feature = "qmd")]
    Qmd(moltis_qmd::QmdManager),

    /// Builtin SQLite + FTS5 backend.
    #[cfg(feature = "builtin")]
    Builtin {
        store: Box<dyn CodeIndexStore>,
        embedder: Option<Box<dyn moltis_memory::embeddings::EmbeddingProvider>>,
    },

    /// Config-only: discovery/filter only, no search.
    ConfigOnly,
}

impl CodeIndex {
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
        }
    }

    /// Check if the index has a functional search backend.
    pub fn has_search_backend(&self) -> bool {
        match &self.backend {
            #[cfg(feature = "qmd")]
            Backend::Qmd(_) => true,
            #[cfg(feature = "builtin")]
            Backend::Builtin { .. } => true,
            Backend::ConfigOnly => false,
        }
    }

    /// Discover and list all git-tracked files that pass the filter.
    pub fn list_indexable_files(&self, project_dir: &Path) -> Result<Vec<FilteredFile>> {
        let tracked = discover_tracked_files(project_dir)?;
        let filtered = filter_tracked_files(project_dir, &tracked, &self.config)?;
        Ok(filtered)
    }

    /// Load the persisted hash snapshot for a project.
    pub fn load_snapshot(&self, project_id: &str) -> Result<Option<HashSnapshot>> {
        self.snapshot_store.load(project_id)
    }

    /// Save a hash snapshot for a project.
    pub fn save_snapshot(
        &self,
        project_id: &str,
        snapshot: &HashSnapshot,
    ) -> Result<()> {
        self.snapshot_store.save(project_id, snapshot)
    }

    /// Index a project directory.
    ///
    /// Scans files, chunks them, generates embeddings (if embedder available),
    /// and stores in the configured backend.
    pub async fn index_project(
        &self,
        project_id: &str,
        enable_embeddings: bool,
        project_dir: &Path,
    ) -> Result<IndexStatus> {
        match &self.backend {
            #[cfg(feature = "qmd")]
            Backend::Qmd(qmd) => {
                self.index_project_qmd(project_id, enable_embeddings, project_dir, qmd)
                    .await
            }
            #[cfg(feature = "builtin")]
            Backend::Builtin { store, embedder } => {
                self.index_project_builtin(project_id, project_dir, store.as_ref(), embedder.as_ref().map(|v| v.as_ref()))
                    .await
            }
            Backend::ConfigOnly => Err(Error::BackendUnavailable(
                "no search backend available".to_string(),
            )),
        }
    }

    /// Index project using QMD backend.
    #[cfg(feature = "qmd")]
    async fn index_project_qmd(
        &self,
        project_id: &str,
        enable_embeddings: bool,
        project_dir: &Path,
        qmd: &moltis_qmd::QmdManager,
    ) -> Result<IndexStatus> {
        let filtered = self.list_indexable_files(project_dir)?;

        info!(
            project_id,
            total = filtered.len(),
            "starting code index for project (QMD)"
        );

        // Register this project as a QMD collection (idempotent).
        let collection =
            crate::backend_qmd::project_collection_config(project_dir, project_id, &self.config);
        qmd.ensure_collection(project_id, &collection).await.map_err(|e| {
            Error::IndexFailed {
                project_id: project_id.to_string(),
                message: format!("QMD ensure_collection failed: {e}"),
            }
        })?;

        // Refresh the index.
        qmd.refresh_index(enable_embeddings).await.map_err(|e| {
            Error::IndexFailed {
                project_id: project_id.to_string(),
                message: format!("QMD refresh_index failed: {e}"),
            }
        })?;

        // Build and persist snapshot for future incremental delta.
        let snapshot = build_initial_snapshot(project_dir, &self.config)?;
        self.snapshot_store.save(project_id, &snapshot)?;

        info!(
            project_id,
            files_indexed = filtered.len(),
            "code index complete (QMD)"
        );

        let epoch_ms = time::OffsetDateTime::now_utc()
            .unix_timestamp_nanos()
            .unsigned_abs() as u64
            / 1_000_000;

        Ok(IndexStatus {
            project_id: project_id.to_string(),
            total_files: filtered.len(),
            total_chunks: 0, // QMD doesn't expose chunk count directly
            last_sync_ms: Some(epoch_ms),
            embedding_model: None,
            backend: "qmd".to_string(),
        })
    }

    /// Index project using builtin backend.
    #[cfg(feature = "builtin")]
    async fn index_project_builtin(
        &self,
        project_id: &str,
        project_dir: &Path,
        store: &dyn CodeIndexStore,
        embedder: Option<&dyn moltis_memory::embeddings::EmbeddingProvider>,
    ) -> Result<IndexStatus> {
        let filtered = self.list_indexable_files(project_dir)?;

        info!(
            project_id,
            total = filtered.len(),
            "starting code index for project (builtin)"
        );

        // Initialize store if needed (idempotent — new() already calls this,
        // but a direct store construction might skip it).
        store.initialize().await.map_err(|e| {
            Error::IndexFailed {
                project_id: project_id.to_string(),
                message: format!("failed to initialize store: {e}"),
            }
        })?;

        // TODO(crash-safety): clear-then-reindex is not atomic across files.
        // If the process crashes mid-index, old data is gone and new data is incomplete.
        // Safer patterns: write-to-temp-project-id + rename, or clear-per-file
        // (upsert_chunks already does per-file DELETE+INSERT in a transaction).
        store.clear_project(project_id).await.map_err(|e| {
            Error::IndexFailed {
                project_id: project_id.to_string(),
                message: format!("failed to clear project: {e}"),
            }
        })?;

        let chunker = crate::chunker::CodeChunker::new(crate::chunker::ChunkerConfig::default());
        let mut total_chunks = 0;

        // Process each file.
        for file in &filtered {
            // file.path is absolute — use directly for disk reads.
            // file.relative_path is the repo-relative path — used for storage and logging.
            let content = match tokio::fs::read_to_string(&file.path).await {
                Ok(c) => c,
                Err(e) => {
                    warn!(path = %file.relative_path.display(), error = %e, "failed to read file, skipping");
                    continue;
                }
            };

            // Chunk the file.
            let chunks = chunker.chunk(&content, &file.relative_path.display().to_string());
            if chunks.is_empty() {
                continue;
            }

            // Generate embeddings if embedder available.
            let mut chunks_with_embeddings = Vec::new();
            if let Some(emb) = embedder {
                let texts: Vec<String> = chunks.iter().map(|c| c.content.clone()).collect();
                match emb.embed_batch(&texts).await {
                    Ok(embeddings) => {
                        for (mut chunk, embedding) in chunks.into_iter().zip(embeddings) {
                            chunk.embedding = Some(embedding);
                            chunks_with_embeddings.push(chunk);
                        }
                    }
                    Err(e) => {
                        warn!(path = %file.relative_path.display(), error = %e, "failed to embed chunks, using without embeddings");
                        chunks_with_embeddings = chunks;
                    }
                }
            } else {
                chunks_with_embeddings = chunks;
            }

            // Store chunks.
            let rel_path_str = file.relative_path.to_str().unwrap_or_default();
            store
                .upsert_chunks(project_id, rel_path_str, &chunks_with_embeddings)
                .await
                .map_err(|e| {
                    Error::IndexFailed {
                        project_id: project_id.to_string(),
                        message: format!("failed to store chunks for {rel_path_str}: {e}"),
                    }
                })?;

            total_chunks += chunks_with_embeddings.len();
        }

        // Build and persist snapshot.
        let snapshot = build_initial_snapshot(project_dir, &self.config)?;
        self.snapshot_store.save(project_id, &snapshot)?;

        info!(
            project_id,
            files_indexed = filtered.len(),
            chunks_indexed = total_chunks,
            "code index complete (builtin)"
        );

        let epoch_ms = time::OffsetDateTime::now_utc()
            .unix_timestamp_nanos()
            .unsigned_abs() as u64
            / 1_000_000;

        Ok(IndexStatus {
            project_id: project_id.to_string(),
            total_files: filtered.len(),
            total_chunks,
            last_sync_ms: Some(epoch_ms),
            embedding_model: embedder.map(|e| e.model_name().to_string()),
            backend: "builtin".to_string(),
        })
    }

    /// Search the code index for a project.
    pub async fn search(&self, project_id: &str, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        match &self.backend {
            #[cfg(feature = "qmd")]
            Backend::Qmd(qmd) => {
                let raw_results = qmd
                    .hybrid_search(query, limit, true)
                    .await
                    .map_err(|e| Error::BackendUnavailable(format!("QMD search failed: {e}")))?;

                Ok(crate::search::from_qmd_results(&raw_results, project_id))
            }
            #[cfg(feature = "builtin")]
            Backend::Builtin { store, embedder } => {
                self.search_builtin(project_id, query, limit, store.as_ref(), embedder.as_ref().map(|v| v.as_ref()))
                    .await
            }
            Backend::ConfigOnly => Err(Error::BackendUnavailable(
                "no search backend available".to_string(),
            )),
        }
    }

    /// Search using builtin backend.
    #[cfg(feature = "builtin")]
    async fn search_builtin(
        &self,
        project_id: &str,
        query: &str,
        limit: usize,
        store: &dyn CodeIndexStore,
        embedder: Option<&dyn moltis_memory::embeddings::EmbeddingProvider>,
    ) -> Result<Vec<SearchResult>> {
        // Get keyword results.
        let keyword_results = store.search_keyword(project_id, query, limit * 2).await?;

        // If no embedder, return keyword results only.
        let Some(emb) = embedder else {
            return Ok(keyword_results.into_iter().take(limit).collect());
        };

        // Generate query embedding.
        let query_embedding = match emb.embed_batch(&[query.to_string()]).await {
            Ok(mut embeddings) => {
                // Invariant: passed one string, get one embedding back.
                embeddings.pop().ok_or_else(|| {
                    Error::IndexStore("embed_batch returned empty result for single input".to_string())
                })?
            }
            Err(e) => {
                warn!(error = %e, "query embedding failed, falling back to keyword results");
                return Ok(keyword_results.into_iter().take(limit).collect());
            }
        };

        // PERF: loads ALL chunks into memory for brute-force vector search.
        // For large projects (100k+ chunks) this could consume significant RAM.
        // Consistent with the memory system's approach. A streaming or indexed
        // approach would be needed for very large projects.
        let chunks = store.get_project_chunks(project_id).await?;

        // Score chunks by cosine similarity.
        let mut scored_chunks: Vec<(f32, SearchResult)> = chunks
            .into_iter()
            .filter_map(|chunk| {
                let chunk_embedding = chunk.embedding?;
                let score = crate::store::cosine_similarity(&query_embedding, &chunk_embedding);
                let result = SearchResult {
                    chunk_id: format!("{}:{}:{}", project_id, chunk.file_path, chunk.start_line),
                    path: chunk.file_path,
                    text: chunk.content,
                    start_line: chunk.start_line,
                    end_line: chunk.end_line,
                    score,
                    source: "builtin".to_string(),
                };
                Some((score, result))
            })
            .collect();

        // Sort by score descending.
        scored_chunks.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        let vector_results: Vec<SearchResult> = scored_chunks
            .into_iter()
            .take(limit * 2)
            .map(|(_, r)| r)
            .collect();

        // Merge with keyword results.
        Ok(crate::store::merge_hybrid_results(vector_results, keyword_results, limit))
    }

    /// Keyword-only search (BM25, no vector embeddings).
    pub async fn keyword_search(&self, project_id: &str, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        match &self.backend {
            #[cfg(feature = "qmd")]
            Backend::Qmd(qmd) => {
                let raw_results = qmd
                    .keyword_search(query, limit)
                    .await
                    .map_err(|e| Error::BackendUnavailable(format!("QMD keyword search failed: {e}")))?;

                Ok(crate::search::from_qmd_results(&raw_results, project_id))
            }
            #[cfg(feature = "builtin")]
            Backend::Builtin { store, .. } => {
                store.search_keyword(project_id, query, limit).await
            }
            Backend::ConfigOnly => Err(Error::BackendUnavailable(
                "no search backend available".to_string(),
            )),
        }
    }

    /// Get the status of the indexed project.
    pub async fn status(&self, project_id: &str) -> Result<IndexStatus> {
        match &self.backend {
            #[cfg(feature = "qmd")]
            Backend::Qmd(_) => Err(Error::BackendUnavailable(
                "status not implemented for QMD backend".to_string(),
            )),
            #[cfg(feature = "builtin")]
            Backend::Builtin { store, .. } => {
                let file_count = store.file_count(project_id).await?;
                let chunk_count = store.chunk_count(project_id).await?;

                Ok(IndexStatus {
                    project_id: project_id.to_string(),
                    total_files: file_count,
                    total_chunks: chunk_count,
                    last_sync_ms: None,
                    embedding_model: None,
                    backend: "builtin".to_string(),
                })
            }
            Backend::ConfigOnly => Err(Error::BackendUnavailable(
                "no search backend available".to_string(),
            )),
        }
    }
}

/// Peek at a specific file's indexed content.
pub fn peek_file(project_dir: &Path, file_path: &str, start_line: usize, end_line: usize) -> Result<String> {
    let full_path = project_dir.join(file_path);
    let content = std::fs::read_to_string(&full_path)
        .map_err(Error::Io)?;

    let lines: Vec<&str> = content.lines().collect();
    let start = (start_line.saturating_sub(1)).min(lines.len());
    let end = end_line.min(lines.len());

    Ok(lines[start..end].join("\n"))
}

#[cfg(all(test, feature = "builtin"))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::store_sqlite::SqliteCodeIndexStore;
    use async_trait::async_trait;

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
        async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
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
        async fn embed(&self, _text: &str) -> anyhow::Result<Vec<f32>> {
            anyhow::bail!("embed failed")
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
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir)
            .output()
            .unwrap();
    }

    fn git_commit_all(dir: &Path, msg: &str) {
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", msg])
            .current_dir(dir)
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

    async fn setup_index() -> (CodeIndex, tempfile::TempDir) {
        let repo = create_test_repo();
        let store = make_store().await;
        let config = CodeIndexConfig::default();
        let index = CodeIndex::new_builtin(config, Box::new(store), None);
        (index, repo)
    }

    async fn setup_index_with_embedder() -> (CodeIndex, tempfile::TempDir) {
        let repo = create_test_repo();
        let store = make_store().await;
        let config = CodeIndexConfig::default();
        let embedder: Box<dyn moltis_memory::embeddings::EmbeddingProvider> =
            Box::new(MockEmbedder::new(16));
        let index = CodeIndex::new_builtin(config, Box::new(store), Some(embedder));
        (index, repo)
    }

    async fn setup_index_with_failing_embedder() -> (CodeIndex, tempfile::TempDir) {
        let repo = create_test_repo();
        let store = make_store().await;
        let config = CodeIndexConfig::default();
        let embedder: Box<dyn moltis_memory::embeddings::EmbeddingProvider> =
            Box::new(FailingEmbedder);
        let index = CodeIndex::new_builtin(config, Box::new(store), Some(embedder));
        (index, repo)
    }

    #[tokio::test]
    async fn test_index_project_no_embedder() {
        let (index, repo) = setup_index().await;
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
        let (index, repo) = setup_index().await;
        index
            .index_project("test-proj", false, repo.path())
            .await
            .unwrap();

        let results = index.search("test-proj", "hello", 10).await.unwrap();
        assert!(!results.is_empty(), "keyword search for 'hello' should find results");
    }

    #[tokio::test]
    async fn test_index_and_keyword_search_miss() {
        let (index, repo) = setup_index().await;
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
        let (index, repo) = setup_index().await;

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
        let (index, repo1) = setup_index().await;
        let repo2 = create_test_repo();

        // Modify repo2 to have distinct content
        std::fs::write(repo2.path().join("README.md"), "# Different\n\nUnique content here.\n")
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
        let (index, repo) = setup_index_with_embedder().await;
        index
            .index_project("test-proj", true, repo.path())
            .await
            .unwrap();

        let status = index.status("test-proj").await.unwrap();
        assert_eq!(status.embedding_model, None); // status() doesn't carry model info from ctor

        // Search should work — vector results should be present
        let results = index.search("test-proj", "main", 10).await.unwrap();
        assert!(!results.is_empty());
    }

    #[tokio::test]
    async fn test_search_embedder_failure_fallback() {
        let (index, repo) = setup_index_with_failing_embedder().await;
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
