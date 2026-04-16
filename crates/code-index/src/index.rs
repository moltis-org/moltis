//! Code index orchestrator.
//!
//! Ties together discover → filter → QMD indexing into a single
//! `CodeIndex` struct that optionally owns a `QmdManager` and provides
//! `index_project()` and `search()` methods.

use std::path::Path;

#[cfg(feature = "qmd")]
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
    qmd: Option<moltis_qmd::QmdManager>,
}

impl CodeIndex {
    /// Create a new code index with the given config and QMD backend.
    ///
    /// The QMD manager should already be configured with the correct
    /// collections for the project(s) to be indexed.
    #[cfg(feature = "qmd")]
    pub fn new(config: CodeIndexConfig, qmd: moltis_qmd::QmdManager) -> Self {
        Self {
            config,
            qmd: Some(qmd),
        }
    }

    /// Create a code index with config but no backend.
    ///
    /// Useful for discover/filter-only workflows where search is not needed.
    /// Calling [`search`](CodeIndex::search), [`keyword_search`](CodeIndex::keyword_search),
    /// or [`index_project`](CodeIndex::index_project) on a config-only instance will return
    /// [`Error::BackendUnavailable`].
    pub fn config_only(config: CodeIndexConfig) -> Self {
        Self {
            config,
            #[cfg(feature = "qmd")]
            qmd: None,
        }
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
    /// Requires a QMD backend — returns [`Error::BackendUnavailable`] if
    /// constructed via [`CodeIndex::config_only`].
    ///
    /// 1. Discover git-tracked files
    /// 2. Filter by extension, size, binary
    /// 3. Ensure QMD collection exists
    /// 4. Trigger QMD reindex
    ///
    /// Returns the status of the indexed project.
    #[cfg(feature = "qmd")]
    pub async fn index_project(
        &self,
        project_id: &str,
        enable_embeddings: bool,
        project_dir: &Path,
    ) -> Result<IndexStatus> {
        let qmd = self.qmd.as_ref().ok_or_else(|| {
            Error::BackendUnavailable(
                "no QMD backend configured \u{2014} use CodeIndex::new() to provide one"
                    .to_string(),
            )
        })?;

        let filtered = self.list_indexable_files(project_dir)?;

        info!(
            project_id,
            total = filtered.len(),
            "starting code index for project"
        );

        // Ensure QMD collections are registered.
        qmd.ensure_collections().await.map_err(|e| {
            Error::IndexFailed {
                project_id: project_id.to_string(),
                message: format!("QMD ensure_collections failed: {e}"),
            }
        })?;

        // Refresh the index — this triggers QMD to re-scan the files.
        qmd.refresh_index(enable_embeddings).await.map_err(|e| {
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

    /// Search the code index for a project.
    ///
    /// Delegates to QMD hybrid search (keyword + vector).
    /// Requires a QMD backend — returns [`Error::BackendUnavailable`] otherwise.
    #[cfg(feature = "qmd")]
    pub async fn search(
        &self,
        project_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<crate::types::SearchResult>> {
        let qmd = self.qmd.as_ref().ok_or_else(|| {
            Error::BackendUnavailable(
                "no QMD backend configured \u{2014} use CodeIndex::new() to provide one"
                    .to_string(),
            )
        })?;

        let raw_results = qmd
            .hybrid_search(query, limit, true)
            .await
            .map_err(|e| {
                Error::BackendUnavailable(format!("QMD search failed: {e}"))
            })?;

        Ok(crate::search::from_qmd_results(&raw_results, project_id))
    }

    /// Keyword-only search (BM25, no vector embeddings).
    ///
    /// Requires a QMD backend — returns [`Error::BackendUnavailable`] otherwise.
    #[cfg(feature = "qmd")]
    pub async fn keyword_search(
        &self,
        project_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<crate::types::SearchResult>> {
        let qmd = self.qmd.as_ref().ok_or_else(|| {
            Error::BackendUnavailable(
                "no QMD backend configured \u{2014} use CodeIndex::new() to provide one"
                    .to_string(),
            )
        })?;

        let raw_results = qmd
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
    /// Propagates discover/filter errors instead of silently reporting zero.
    #[cfg(feature = "qmd")]
    pub async fn status(&self, project_id: &str, project_dir: &Path) -> Result<IndexStatus> {
        match &self.qmd {
            None => Ok(IndexStatus {
                project_id: project_id.to_string(),
                total_files: 0,
                total_chunks: 0,
                last_sync_ms: None,
                embedding_model: None,
                backend: "none (config-only)".to_string(),
            }),
            Some(qmd) => {
                if !qmd.is_available().await {
                    return Ok(IndexStatus {
                        project_id: project_id.to_string(),
                        total_files: 0,
                        total_chunks: 0,
                        last_sync_ms: None,
                        embedding_model: None,
                        backend: "qmd (unavailable)".to_string(),
                    });
                }

                let filtered = self.list_indexable_files(project_dir)?;

                Ok(IndexStatus {
                    project_id: project_id.to_string(),
                    total_files: filtered.len(),
                    total_chunks: 0,
                    last_sync_ms: None, // Would need QMD metadata to determine this
                    embedding_model: None,
                    backend: "qmd".to_string(),
                })
            }
        }
    }

    /// Get the current index status (no QMD backend).
    ///
    /// Only reports the number of indexable files.
    /// Propagates discover/filter errors instead of silently reporting zero.
    #[cfg(not(feature = "qmd"))]
    pub async fn status(&self, project_id: &str, project_dir: &Path) -> Result<IndexStatus> {
        let filtered = self.list_indexable_files(project_dir)?;

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

    /// Config-only instances must reject search/index calls with BackendUnavailable.
    #[cfg(feature = "qmd")]
    #[tokio::test]
    async fn test_config_only_rejects_search() {
        let config = CodeIndexConfig::default();
        let idx = CodeIndex::config_only(config);

        let result = idx.search("test-project", "fn main", 10).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, Error::BackendUnavailable(_)),
            "expected BackendUnavailable, got {err:?}"
        );
    }

    #[cfg(feature = "qmd")]
    #[tokio::test]
    async fn test_config_only_rejects_keyword_search() {
        let config = CodeIndexConfig::default();
        let idx = CodeIndex::config_only(config);

        let result = idx.keyword_search("test-project", "fn main", 10).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, Error::BackendUnavailable(_)),
            "expected BackendUnavailable, got {err:?}"
        );
    }

    #[cfg(feature = "qmd")]
    #[tokio::test]
    async fn test_config_only_rejects_index_project() {
        let config = CodeIndexConfig::default();
        let idx = CodeIndex::config_only(config);

        let result = idx
            .index_project("test-project", true, Path::new("/tmp"))
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, Error::BackendUnavailable(_)),
            "expected BackendUnavailable, got {err:?}"
        );
    }
}