/// Storage abstraction for memory files, chunks, and embedding cache.
use async_trait::async_trait;

use crate::{
    schema::{ChunkRow, FileRow},
    search::SearchResult,
};

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
    async fn upsert_file(&self, file: &FileRow) -> anyhow::Result<()>;
    async fn get_file(&self, path: &str) -> anyhow::Result<Option<FileRow>>;
    async fn delete_file(&self, path: &str) -> anyhow::Result<()>;
    async fn list_files(&self) -> anyhow::Result<Vec<FileRow>>;

    // ---- chunks ----
    async fn upsert_chunks(&self, chunks: &[ChunkRow]) -> anyhow::Result<()>;
    async fn get_chunks_for_file(&self, path: &str) -> anyhow::Result<Vec<ChunkRow>>;
    async fn delete_chunks_for_file(&self, path: &str) -> anyhow::Result<()>;
    async fn get_chunk_by_id(&self, id: &str) -> anyhow::Result<Option<ChunkRow>>;

    // ---- embedding cache ----
    async fn get_cached_embedding(
        &self,
        provider: &str,
        model: &str,
        hash: &str,
    ) -> anyhow::Result<Option<Vec<f32>>>;

    async fn put_cached_embedding(
        &self,
        provider: &str,
        model: &str,
        provider_key: &str,
        hash: &str,
        embedding: &[f32],
    ) -> anyhow::Result<()>;

    /// Batch-insert multiple embedding cache entries in a single transaction.
    async fn put_cached_embeddings_batch(&self, entries: &[CacheEntry<'_>]) -> anyhow::Result<()>;

    /// Count the number of rows in the embedding cache.
    async fn count_cached_embeddings(&self) -> anyhow::Result<usize>;

    /// Evict the oldest cache rows, keeping at most `keep` entries.
    async fn evict_embedding_cache(&self, keep: usize) -> anyhow::Result<usize>;

    // ---- search ----
    async fn vector_search(
        &self,
        query_embedding: &[f32],
        limit: usize,
    ) -> anyhow::Result<Vec<SearchResult>>;

    async fn keyword_search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<SearchResult>>;
}
