/// Storage abstraction for memory files, chunks, embedding cache, and hybrid search.
use async_trait::async_trait;

use crate::{
    error::Result,
    schema::{ChunkRow, FileRow},
    search::SearchResult,
};

/// Strategy for merging vector and keyword search results.
///
/// The per-source weights (`vector_weight` / `keyword_weight`) are passed
/// separately to [`MemoryStore::hybrid_search`] and apply to every variant,
/// so they are intentionally not duplicated here.
#[derive(Debug, Clone, Copy)]
pub enum MergeStrategy {
    /// Reciprocal Rank Fusion with rank-constant `k` (default k=60).
    Rrf { k: u32 },
    /// Weighted linear blend (uses the weights passed to `hybrid_search`).
    Weighted,
}

impl Default for MergeStrategy {
    fn default() -> Self {
        Self::Rrf { k: 60 }
    }
}

/// A single embedding cache entry for batch inserts.
pub struct CacheEntry<'a> {
    pub provider: &'a str,
    pub model: &'a str,
    pub provider_key: &'a str,
    pub hash: &'a str,
    pub embedding: &'a [f32],
}

#[async_trait]
pub trait MemoryStore: Send + Sync {
    // ---- files ----
    async fn upsert_file(&self, file: &FileRow) -> Result<()>;
    async fn get_file(&self, path: &str) -> Result<Option<FileRow>>;
    async fn delete_file(&self, path: &str) -> Result<()>;
    async fn list_files(&self) -> Result<Vec<FileRow>>;

    // ---- chunks ----
    async fn upsert_chunks(&self, chunks: &[ChunkRow]) -> Result<()>;
    async fn get_chunks_for_file(&self, path: &str) -> Result<Vec<ChunkRow>>;
    async fn delete_chunks_for_file(&self, path: &str) -> Result<()>;
    async fn get_chunk_by_id(&self, id: &str) -> Result<Option<ChunkRow>>;

    // ---- embedding cache ----
    async fn get_cached_embedding(
        &self,
        provider: &str,
        model: &str,
        hash: &str,
    ) -> Result<Option<Vec<f32>>>;

    async fn put_cached_embedding(
        &self,
        provider: &str,
        model: &str,
        provider_key: &str,
        hash: &str,
        embedding: &[f32],
    ) -> Result<()>;

    /// Batch-insert multiple embedding cache entries in a single transaction.
    async fn put_cached_embeddings_batch(&self, entries: &[CacheEntry<'_>]) -> Result<()>;

    /// Count the number of rows in the embedding cache.
    async fn count_cached_embeddings(&self) -> Result<usize>;

    /// Evict the oldest cache rows, keeping at most `keep` entries.
    async fn evict_embedding_cache(&self, keep: usize) -> Result<usize>;

    // ---- search ----
    /// Perform hybrid search: vector similarity + keyword/FTS, merged by the
    /// chosen strategy. `vector_weight` and `keyword_weight` apply to every
    /// [`MergeStrategy`] variant. Pass an empty embedding slice for keyword-only
    /// search (only the FTS sub-query contributes); pass an empty query string
    /// for vector-only search.
    async fn hybrid_search(
        &self,
        embedding: &[f32],
        query: &str,
        vector_weight: f32,
        keyword_weight: f32,
        merge_strategy: MergeStrategy,
        limit: usize,
    ) -> Result<Vec<SearchResult>>;

    /// Backend type identifier: "sqlite" for the built-in SQLite store,
    /// "zvec" for the zvec HNSW store.
    fn store_type(&self) -> &'static str {
        "sqlite"
    }

    /// HNSW index build percentage (zvec only, returns None for SQLite).
    fn hnsw_percent(&self) -> Option<f64> {
        None
    }

    /// Approximate on-disk footprint of the store's data files in bytes.
    ///
    /// Backends that own their data files (zvec) override this so the memory
    /// status reports real disk usage. The built-in SQLite store reports 0
    /// here and lets the manager fall back to measuring its `db_path` file.
    /// Returns 0 for in-memory stores or when the files are unavailable.
    fn disk_size_bytes(&self) -> u64 {
        0
    }
}
