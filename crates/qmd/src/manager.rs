//! QMD sidecar process manager.
//!
//! Manages communication with the QMD CLI for indexing and search operations.

use std::{collections::HashMap, path::PathBuf, process::Stdio, time::Duration};

use {
    serde::{Deserialize, Serialize},
    tokio::{process::Command, sync::RwLock, time::timeout},
    tracing::{debug, info, warn},
};

/// Configuration for the QMD manager.
#[derive(Debug, Clone)]
pub struct QmdManagerConfig {
    /// Path to the qmd binary (default: "qmd").
    pub command: String,
    /// Named collections with their paths.
    pub collections: HashMap<String, QmdCollection>,
    /// Maximum results to retrieve.
    pub max_results: usize,
    /// Search timeout in milliseconds.
    pub timeout_ms: u64,
    /// Working directory for QMD (typically the data directory).
    pub work_dir: PathBuf,
}

impl Default for QmdManagerConfig {
    fn default() -> Self {
        Self {
            command: "qmd".into(),
            collections: HashMap::new(),
            max_results: 10,
            timeout_ms: 30_000,
            work_dir: PathBuf::from("."),
        }
    }
}

/// A QMD collection configuration.
#[derive(Debug, Clone)]
pub struct QmdCollection {
    /// Paths to include in this collection.
    pub paths: Vec<PathBuf>,
    /// Glob patterns to filter files.
    pub globs: Vec<String>,
}

/// Search mode for QMD queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    /// BM25 keyword search (fast, instant).
    Keyword,
    /// Vector similarity search.
    Vector,
    /// Hybrid: keyword + vector + LLM reranking.
    Hybrid,
}

impl SearchMode {
    fn as_command(&self) -> &str {
        match self {
            SearchMode::Keyword => "search",
            SearchMode::Vector => "vsearch",
            SearchMode::Hybrid => "query",
        }
    }
}

/// A search result from QMD.
#[derive(Debug, Clone, Deserialize)]
pub struct QmdSearchResult {
    /// The file path.
    pub path: String,
    /// Line number where the match starts.
    pub line: i64,
    /// Relevance score (0.0-1.0).
    pub score: f32,
    /// The matched text content.
    pub text: String,
    /// Optional collection name.
    #[serde(default)]
    pub collection: Option<String>,
}

/// Status of the QMD backend.
#[derive(Debug, Clone, Serialize)]
pub struct QmdStatus {
    /// Whether QMD is available.
    pub available: bool,
    /// QMD version string.
    pub version: Option<String>,
    /// Number of indexed files per collection.
    pub indexed_files: HashMap<String, usize>,
    /// Error message if unavailable.
    pub error: Option<String>,
}

/// Manager for the QMD sidecar process.
pub struct QmdManager {
    config: QmdManagerConfig,
    /// Whether QMD is available on this system.
    available: RwLock<Option<bool>>,
}

impl QmdManager {
    /// Create a new QMD manager with the given configuration.
    pub fn new(config: QmdManagerConfig) -> Self {
        Self {
            config,
            available: RwLock::new(None),
        }
    }

    /// Check if QMD is available on this system.
    pub async fn is_available(&self) -> bool {
        // Check cached result
        {
            let cached = self.available.read().await;
            if let Some(available) = *cached {
                return available;
            }
        }

        // Check if qmd binary exists and is executable
        let available = self.check_qmd_available().await;
        *self.available.write().await = Some(available);
        available
    }

    async fn check_qmd_available(&self) -> bool {
        match Command::new(&self.config.command)
            .arg("--version")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
        {
            Ok(output) => {
                if output.status.success() {
                    let version = String::from_utf8_lossy(&output.stdout);
                    info!(version = %version.trim(), "QMD is available");
                    true
                } else {
                    warn!("QMD command failed with non-zero exit code");
                    false
                }
            },
            Err(e) => {
                debug!(error = %e, "QMD is not available");
                false
            },
        }
    }

    /// Get the status of the QMD backend.
    pub async fn status(&self) -> QmdStatus {
        if !self.is_available().await {
            return QmdStatus {
                available: false,
                version: None,
                indexed_files: HashMap::new(),
                error: Some("QMD binary not found".into()),
            };
        }

        // Get version
        let version = match Command::new(&self.config.command)
            .arg("--version")
            .output()
            .await
        {
            Ok(output) => Some(String::from_utf8_lossy(&output.stdout).trim().to_string()),
            Err(_) => None,
        };

        // Get indexed file counts per collection
        let mut indexed_files = HashMap::new();
        for name in self.config.collections.keys() {
            if let Ok(count) = self.get_indexed_count(name).await {
                indexed_files.insert(name.clone(), count);
            }
        }

        QmdStatus {
            available: true,
            version,
            indexed_files,
            error: None,
        }
    }

    async fn get_indexed_count(&self, collection: &str) -> anyhow::Result<usize> {
        let output = Command::new(&self.config.command)
            .arg("stats")
            .arg("--collection")
            .arg(collection)
            .arg("--json")
            .current_dir(&self.config.work_dir)
            .output()
            .await?;

        if output.status.success() {
            let stats: serde_json::Value = serde_json::from_slice(&output.stdout)?;
            Ok(stats["files"].as_u64().unwrap_or(0) as usize)
        } else {
            Ok(0)
        }
    }

    /// Index the given paths for a collection.
    pub async fn index(&self, collection: &str, paths: &[PathBuf]) -> anyhow::Result<()> {
        if !self.is_available().await {
            anyhow::bail!("QMD is not available");
        }

        let mut cmd = Command::new(&self.config.command);
        cmd.arg("index")
            .arg("--collection")
            .arg(collection)
            .current_dir(&self.config.work_dir);

        for path in paths {
            cmd.arg(path);
        }

        let output = cmd.output().await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("QMD index failed: {}", stderr);
        }

        info!(collection = %collection, paths = paths.len(), "indexed files for QMD");
        Ok(())
    }

    /// Search using the specified mode.
    pub async fn search(
        &self,
        query: &str,
        mode: SearchMode,
        collection: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<QmdSearchResult>> {
        if !self.is_available().await {
            anyhow::bail!("QMD is not available");
        }

        let mut cmd = Command::new(&self.config.command);
        cmd.arg(mode.as_command())
            .arg("--json")
            .arg("--limit")
            .arg(limit.to_string())
            .arg(query)
            .current_dir(&self.config.work_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(coll) = collection {
            cmd.arg("--collection").arg(coll);
        }

        let timeout_duration = Duration::from_millis(self.config.timeout_ms);

        let output = match timeout(timeout_duration, cmd.output()).await {
            Ok(result) => result?,
            Err(_) => anyhow::bail!("QMD search timed out after {}ms", self.config.timeout_ms),
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("QMD search failed: {}", stderr);
        }

        let results: Vec<QmdSearchResult> = serde_json::from_slice(&output.stdout)?;

        debug!(
            query = %query,
            mode = ?mode,
            results = results.len(),
            "QMD search completed"
        );

        Ok(results)
    }

    /// Hybrid search (combines keyword, vector, and LLM reranking).
    pub async fn hybrid_search(
        &self,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<QmdSearchResult>> {
        self.search(query, SearchMode::Hybrid, None, limit).await
    }

    /// Fast keyword search using BM25.
    pub async fn keyword_search(
        &self,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<QmdSearchResult>> {
        self.search(query, SearchMode::Keyword, None, limit).await
    }

    /// Vector similarity search.
    pub async fn vector_search(
        &self,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<QmdSearchResult>> {
        self.search(query, SearchMode::Vector, None, limit).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_manager_config_default() {
        let config = QmdManagerConfig::default();
        assert_eq!(config.command, "qmd");
        assert_eq!(config.max_results, 10);
        assert_eq!(config.timeout_ms, 30_000);
    }

    #[tokio::test]
    async fn test_search_mode_commands() {
        assert_eq!(SearchMode::Keyword.as_command(), "search");
        assert_eq!(SearchMode::Vector.as_command(), "vsearch");
        assert_eq!(SearchMode::Hybrid.as_command(), "query");
    }

    #[tokio::test]
    async fn test_manager_unavailable() {
        // Use a non-existent command
        let config = QmdManagerConfig {
            command: "nonexistent-qmd-binary-12345".into(),
            ..Default::default()
        };
        let manager = QmdManager::new(config);

        assert!(!manager.is_available().await);

        let status = manager.status().await;
        assert!(!status.available);
        assert!(status.error.is_some());
    }
}
