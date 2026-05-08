use std::{path::Path, sync::Arc};

use {async_trait::async_trait, moltis_agents::memory_writer::MemoryWriter};

use crate::{
    config::CitationMode,
    error::Result,
    manager::{MemoryManager, MemoryStatus, SyncReport},
    schema::ChunkRow,
    search::SearchResult,
};

pub type DynMemoryRuntime = Arc<dyn MemoryRuntime>;

#[async_trait]
pub trait MemoryRuntime: MemoryWriter + Send + Sync {
    fn backend_name(&self) -> &'static str;

    fn data_dir(&self) -> Option<&Path>;

    fn has_embeddings(&self) -> bool;

    fn citation_mode(&self) -> CitationMode;

    fn llm_reranking_enabled(&self) -> bool;

    async fn sync(&self) -> Result<SyncReport>;

    async fn sync_path(&self, path: &Path) -> Result<bool>;

    async fn remove_path(&self, path: &Path) -> Result<bool>;

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>>;

    async fn get_chunk(&self, id: &str) -> Result<Option<ChunkRow>>;

    async fn status(&self) -> Result<MemoryStatus>;
}

#[async_trait]
impl MemoryRuntime for MemoryManager {
    fn backend_name(&self) -> &'static str {
        "builtin"
    }

    fn data_dir(&self) -> Option<&Path> {
        MemoryManager::data_dir(self)
    }

    fn has_embeddings(&self) -> bool {
        MemoryManager::has_embeddings(self)
    }

    fn citation_mode(&self) -> CitationMode {
        MemoryManager::citation_mode(self)
    }

    fn llm_reranking_enabled(&self) -> bool {
        MemoryManager::llm_reranking_enabled(self)
    }

    async fn sync(&self) -> Result<SyncReport> {
        MemoryManager::sync(self).await
    }

    async fn sync_path(&self, path: &Path) -> Result<bool> {
        MemoryManager::sync_path(self, path).await
    }

    async fn remove_path(&self, path: &Path) -> Result<bool> {
        MemoryManager::remove_path(self, path).await
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        MemoryManager::search(self, query, limit).await
    }

    async fn get_chunk(&self, id: &str) -> Result<Option<ChunkRow>> {
        MemoryManager::get_chunk(self, id).await
    }

    async fn status(&self) -> Result<MemoryStatus> {
        MemoryManager::status(self).await
    }
}
