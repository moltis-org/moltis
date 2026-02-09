//! QMD implementation of the MemoryStore trait.
//!
//! This provides a bridge between the moltis memory system and the QMD backend,
//! allowing QMD to be used as an alternative to the built-in SQLite store.

use std::sync::Arc;

use {
    async_trait::async_trait,
    moltis_memory::{
        schema::{ChunkRow, FileRow},
        search::SearchResult,
        store::MemoryStore,
    },
    tracing::warn,
};

use crate::manager::QmdManager;

/// QMD-based memory store implementation.
///
/// This store delegates search operations to QMD while providing stub
/// implementations for write operations (QMD manages its own index).
pub struct QmdStore {
    manager: Arc<QmdManager>,
    /// Whether to include the built-in memory alongside QMD results.
    include_builtin: bool,
    /// Optional fallback store for write operations and when QMD is unavailable.
    fallback: Option<Box<dyn MemoryStore>>,
}

impl QmdStore {
    /// Create a new QMD store with the given manager.
    pub fn new(manager: Arc<QmdManager>) -> Self {
        Self {
            manager,
            include_builtin: false,
            fallback: None,
        }
    }

    /// Include built-in memory in searches.
    pub fn with_builtin_memory(mut self, include: bool) -> Self {
        self.include_builtin = include;
        self
    }

    /// Set a fallback store for write operations and when QMD is unavailable.
    pub fn with_fallback(mut self, store: Box<dyn MemoryStore>) -> Self {
        self.fallback = Some(store);
        self
    }

    /// Convert a QMD search result to a moltis SearchResult.
    fn convert_result(qmd_result: &crate::manager::QmdSearchResult) -> SearchResult {
        // Extract chunk ID from path and line
        let chunk_id = format!("{}:{}", qmd_result.path, qmd_result.line);

        // Determine source from path
        let source = if qmd_result.path.contains("MEMORY") {
            "longterm"
        } else {
            "daily"
        };

        SearchResult {
            chunk_id,
            path: qmd_result.path.clone(),
            source: source.into(),
            start_line: qmd_result.line,
            end_line: qmd_result.line + 10, // Approximate end line
            score: qmd_result.score,
            text: qmd_result.text.clone(),
        }
    }
}

#[async_trait]
impl MemoryStore for QmdStore {
    // ── File operations (delegated to fallback or no-op) ──

    async fn upsert_file(&self, file: &FileRow) -> anyhow::Result<()> {
        if let Some(ref fallback) = self.fallback {
            fallback.upsert_file(file).await
        } else {
            // QMD manages its own index, no-op
            Ok(())
        }
    }

    async fn get_file(&self, path: &str) -> anyhow::Result<Option<FileRow>> {
        if let Some(ref fallback) = self.fallback {
            fallback.get_file(path).await
        } else {
            Ok(None)
        }
    }

    async fn delete_file(&self, path: &str) -> anyhow::Result<()> {
        if let Some(ref fallback) = self.fallback {
            fallback.delete_file(path).await
        } else {
            Ok(())
        }
    }

    async fn list_files(&self) -> anyhow::Result<Vec<FileRow>> {
        if let Some(ref fallback) = self.fallback {
            fallback.list_files().await
        } else {
            Ok(Vec::new())
        }
    }

    // ── Chunk operations (delegated to fallback or no-op) ──

    async fn upsert_chunks(&self, chunks: &[ChunkRow]) -> anyhow::Result<()> {
        if let Some(ref fallback) = self.fallback {
            fallback.upsert_chunks(chunks).await
        } else {
            Ok(())
        }
    }

    async fn get_chunks_for_file(&self, path: &str) -> anyhow::Result<Vec<ChunkRow>> {
        if let Some(ref fallback) = self.fallback {
            fallback.get_chunks_for_file(path).await
        } else {
            Ok(Vec::new())
        }
    }

    async fn delete_chunks_for_file(&self, path: &str) -> anyhow::Result<()> {
        if let Some(ref fallback) = self.fallback {
            fallback.delete_chunks_for_file(path).await
        } else {
            Ok(())
        }
    }

    async fn get_chunk_by_id(&self, id: &str) -> anyhow::Result<Option<ChunkRow>> {
        if let Some(ref fallback) = self.fallback {
            fallback.get_chunk_by_id(id).await
        } else {
            Ok(None)
        }
    }

    // ── Embedding cache (delegated to fallback) ──

    async fn get_cached_embedding(
        &self,
        provider: &str,
        model: &str,
        hash: &str,
    ) -> anyhow::Result<Option<Vec<f32>>> {
        if let Some(ref fallback) = self.fallback {
            fallback.get_cached_embedding(provider, model, hash).await
        } else {
            Ok(None)
        }
    }

    async fn put_cached_embedding(
        &self,
        provider: &str,
        model: &str,
        provider_key: &str,
        hash: &str,
        embedding: &[f32],
    ) -> anyhow::Result<()> {
        if let Some(ref fallback) = self.fallback {
            fallback
                .put_cached_embedding(provider, model, provider_key, hash, embedding)
                .await
        } else {
            Ok(())
        }
    }

    async fn count_cached_embeddings(&self) -> anyhow::Result<usize> {
        if let Some(ref fallback) = self.fallback {
            fallback.count_cached_embeddings().await
        } else {
            Ok(0)
        }
    }

    async fn evict_embedding_cache(&self, keep: usize) -> anyhow::Result<usize> {
        if let Some(ref fallback) = self.fallback {
            fallback.evict_embedding_cache(keep).await
        } else {
            Ok(0)
        }
    }

    // ── Search operations (use QMD with optional fallback) ──

    async fn vector_search(
        &self,
        _query_embedding: &[f32],
        limit: usize,
    ) -> anyhow::Result<Vec<SearchResult>> {
        // QMD doesn't use pre-computed embeddings; use vsearch instead
        // This is a semantic search using QMD's built-in vector search
        // Note: We can't use the embedding directly, so this falls back to keyword search
        // unless we have a query string. In practice, the MemoryManager calls search() directly.

        if let Some(ref fallback) = self.fallback {
            fallback.vector_search(_query_embedding, limit).await
        } else {
            // Can't do vector search without a query string in QMD
            warn!("QMD vector_search called without query - returning empty results");
            Ok(Vec::new())
        }
    }

    async fn keyword_search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<SearchResult>> {
        // Try QMD first
        if self.manager.is_available().await {
            match self.manager.keyword_search(query, limit).await {
                Ok(qmd_results) => {
                    let mut results: Vec<SearchResult> =
                        qmd_results.iter().map(Self::convert_result).collect();

                    // Optionally merge with fallback results
                    if self.include_builtin
                        && let Some(ref fallback) = self.fallback
                        && let Ok(fallback_results) = fallback.keyword_search(query, limit).await
                    {
                        results.extend(fallback_results);
                        // Re-sort by score and take top limit
                        results.sort_by(|a, b| {
                            b.score
                                .partial_cmp(&a.score)
                                .unwrap_or(std::cmp::Ordering::Equal)
                        });
                        results.truncate(limit);
                    }

                    return Ok(results);
                },
                Err(e) => {
                    warn!(error = %e, "QMD keyword search failed, falling back");
                },
            }
        }

        // Fallback to built-in store
        if let Some(ref fallback) = self.fallback {
            fallback.keyword_search(query, limit).await
        } else {
            Ok(Vec::new())
        }
    }
}

/// Extension trait for QMD-specific search operations.
impl QmdStore {
    /// Perform a hybrid search using QMD (keyword + vector + LLM reranking).
    pub async fn hybrid_search(
        &self,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<SearchResult>> {
        if self.manager.is_available().await {
            match self.manager.hybrid_search(query, limit).await {
                Ok(qmd_results) => {
                    let results: Vec<SearchResult> =
                        qmd_results.iter().map(Self::convert_result).collect();
                    return Ok(results);
                },
                Err(e) => {
                    warn!(error = %e, "QMD hybrid search failed, falling back to keyword");
                },
            }
        }

        // Fallback to keyword search
        self.keyword_search(query, limit).await
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, crate::manager::QmdManagerConfig};

    #[tokio::test]
    async fn test_qmd_store_without_fallback() {
        let config = QmdManagerConfig {
            command: "nonexistent-qmd".into(),
            ..Default::default()
        };
        let manager = Arc::new(QmdManager::new(config));
        let store = QmdStore::new(manager);

        // Without fallback, file operations should no-op
        let result = store.list_files().await.unwrap();
        assert!(result.is_empty());

        // Search should return empty results
        let results = store.keyword_search("test", 5).await.unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_convert_result() {
        let qmd_result = crate::manager::QmdSearchResult {
            path: "memory/MEMORY.md".into(),
            line: 42,
            score: 0.85,
            text: "Some important memory content".into(),
            collection: None,
        };

        let result = QmdStore::convert_result(&qmd_result);

        assert_eq!(result.chunk_id, "memory/MEMORY.md:42");
        assert_eq!(result.path, "memory/MEMORY.md");
        assert_eq!(result.source, "longterm");
        assert_eq!(result.start_line, 42);
        assert!((result.score - 0.85).abs() < 0.001);
        assert_eq!(result.text, "Some important memory content");
    }

    #[test]
    fn test_convert_result_daily_source() {
        let qmd_result = crate::manager::QmdSearchResult {
            path: "memory/sessions/session-2024-01-15.md".into(),
            line: 10,
            score: 0.75,
            text: "Session content".into(),
            collection: None,
        };

        let result = QmdStore::convert_result(&qmd_result);

        assert_eq!(result.source, "daily");
    }
}
