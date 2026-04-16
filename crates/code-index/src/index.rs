//! Code index orchestrator.
//!
//! Ties together discover → filter → QMD indexing into a single
//! `CodeIndex` struct that owns a `QmdManager` and provides
//! `index_project()` and `search()` methods.

use std::path::Path;

use tracing::info;

use crate::config::CodeIndexConfig;
use crate::discover::discover_tracked_files;
use crate::error::{Error, Result};
use crate::filter::filter_tracked_files;
use crate::types::IndexStatus;

/// Code index manager.
///
/// Owns a configuration and (when the `qmd` feature is enabled) a
/// QMD backend for indexing and search.
pub struct CodeIndex {
    config: CodeIndexConfig,
    #[cfg(feature = "qmd")]
    qmd: moltis_qmd::QmdManager,
}

impl CodeIndex {
    /// Create a new code index with the given config and QMD manager.
    ///
    /// The QMD manager should already be configured with the correct
    /// collections for the project(s) to be indexed.
    #[cfg(feature = "qmd")]
    pub fn new(config: CodeIndexConfig, qmd: moltis_qmd::QmdManager) -> Self {
        Self { config, qmd }
    }

    /// Create a code index with default config and no QMD backend.
    ///
    /// Useful for discover/filter-only workflows where search is not needed.
    pub fn config_only(config: CodeIndexConfig) -> Self {
        Self::new_without_backend(config)
    }

    #[cfg(not(feature = "qmd"))]
    fn new_without_backend(config: CodeIndexConfig) -> Self {
        Self { config }
    }

    #[cfg(feature = "qmd")]
    fn new_without_backend(config: CodeIndexConfig) -> Self {
        // Build a default QMD manager so the struct field is populated.
        // It won't be used for search but the field must exist.
        let qmd_config = moltis_qmd::QmdManagerConfig::default();
        let qmd = moltis_qmd::QmdManager::new(qmd_config);
        Self { config, qmd }
    }

    /// Discover and list all git-tracked files that pass the filter.
    ///
    /// This is a pure-read operation — it does not index anything.
    /// Useful for inspecting what would be indexed before committing
    /// to a full `index_project()` run.
    pub fn list_indexable_files(&self, project_dir: &Path) -> Result<Vec<crate::types::FilteredFile>> {
        let tracked = discover_tracked_files(project_dir)?;
        let filtered = filter_tracked_files(project_dir, &tracked, &self.config)?;
        Ok(filtered)
    }

    /// Full indexing pipeline for a project.
    ///
    /// 1. Discover git-tracked files
    /// 2. Filter by extension, size, binary
    /// 3. Ensure QMD collection exists
    /// 4. Trigger QMD reindex
    ///
    /// Returns the number of files that passed filtering.
    #[cfg(feature = "qmd")]
    pub async fn index_project(
        &self,
        project_id: &str,
        project_dir: &Path,
    ) -> Result<IndexStatus> {
        let filtered = self.list_indexable_files(project_dir)?;

        info!(
            project_id,
            total = filtered.len(),
            "starting code index for project"
        );

        // Ensure QMD collections are registered.
        self.qmd.ensure_collections().await.map_err(|e| {
            Error::BackendUnavailable(format!("QMD ensure_collections failed: {e}"))
        })?;

        // Refresh the index — this triggers QMD to re-scan the files.
        self.qmd.refresh_index(true).await.map_err(|e| {
            Error::IndexFailed {
                project_id: project_id.to_string(),
                message: format!("QMD refresh_index failed: {e}"),
            }
        })?;

        info!(
            project_id,
            files_indexed = filtered.len(),
            "code index complete"
        );

        Ok(IndexStatus {
            project_id: project_id.to_string(),
            total_files: filtered.len(),
            total_chunks: 0, // QMD doesn't expose chunk count directly
            last_sync_ms: Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            ),
            embedding_model: None,
            backend: "qmd".to_string(),
        })
    }

    /// Search the code index for a project.
    ///
    /// Delegates to QMD hybrid search (keyword + vector).
    #[cfg(feature = "qmd")]
    pub async fn search(
        &self,
        project_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<crate::types::SearchResult>> {
        let raw_results = self
            .qmd
            .hybrid_search(query, limit, true)
            .await
            .map_err(|e| {
                Error::BackendUnavailable(format!("QMD search failed: {e}"))
            })?;

        Ok(crate::search::from_qmd_results(&raw_results, project_id))
    }

    /// Keyword-only search (BM25, no vector embeddings).
    #[cfg(feature = "qmd")]
    pub async fn keyword_search(
        &self,
        project_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<crate::types::SearchResult>> {
        let raw_results = self
            .qmd
            .keyword_search(query, limit)
            .await
            .map_err(|e| {
                Error::BackendUnavailable(format!("QMD keyword_search failed: {e}"))
            })?;

        Ok(crate::search::from_qmd_results(&raw_results, project_id))
    }

    /// Get the current index status for a project.
    ///
    /// Checks QMD availability and reports file counts.
    #[cfg(feature = "qmd")]
    pub async fn status(&self, project_id: &str, project_dir: &Path) -> Result<IndexStatus> {
        let available = self.qmd.is_available().await;

        if !available {
            return Ok(IndexStatus {
                project_id: project_id.to_string(),
                total_files: 0,
                total_chunks: 0,
                last_sync_ms: None,
                embedding_model: None,
                backend: "qmd (unavailable)".to_string(),
            });
        }

        // Try to count indexable files even if QMD is available.
        let filtered = self.list_indexable_files(project_dir).unwrap_or_default();

        Ok(IndexStatus {
            project_id: project_id.to_string(),
            total_files: filtered.len(),
            total_chunks: 0,
            last_sync_ms: None, // Would need QMD metadata to determine this
            embedding_model: None,
            backend: "qmd".to_string(),
        })
    }

    /// Get the current index status (no QMD backend).
    ///
    /// Only reports the number of indexable files.
    #[cfg(not(feature = "qmd"))]
    pub async fn status(&self, project_id: &str, project_dir: &Path) -> Result<IndexStatus> {
        let filtered = self.list_indexable_files(project_dir).unwrap_or_default();

        Ok(IndexStatus {
            project_id: project_id.to_string(),
            total_files: filtered.len(),
            total_chunks: 0,
            last_sync_ms: None,
            embedding_model: None,
            backend: "none".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_list_indexable_files_on_moltis_repo() {
        let config = CodeIndexConfig::default();
        let idx = CodeIndex::config_only(config);

        let repo_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();

        let files = idx.list_indexable_files(repo_dir).unwrap();
        assert!(!files.is_empty(), "moltis repo should have indexable files");

        // Rust files must be present.
        assert!(
            files.iter().any(|f| f.relative_path.to_string_lossy().ends_with(".rs")),
            "should find .rs files in the moltis repo"
        );

        // Target directory should be excluded (.gitignored, not tracked).
        assert!(
            !files.iter().any(|f| f.relative_path.to_string_lossy().starts_with("target/")),
            "target/ files should not be tracked"
        );
    }

    #[test]
    fn test_list_indexable_files_nonexistent_dir() {
        let config = CodeIndexConfig::default();
        let idx = CodeIndex::config_only(config);

        let result = idx.list_indexable_files(Path::new("/nonexistent/path/that/does/not/exist"));
        assert!(result.is_err());
    }
}