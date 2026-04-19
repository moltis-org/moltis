//! QMD sidecar process manager.
//!
//! Manages the current QMD CLI for indexing and search operations.

use std::{collections::HashMap, path::PathBuf, process::Stdio, time::Duration};

use {
    serde::{Deserialize, Serialize},
    tokio::{process::Command, sync::RwLock, time::timeout},
    tracing::{debug, info},
};

use crate::error::{Error, Result};

/// Configuration for the QMD manager.
#[derive(Debug, Clone)]
pub struct QmdManagerConfig {
    /// Path to the qmd binary (default: "qmd").
    pub command: String,
    /// Named collections with their paths and masks.
    pub collections: HashMap<String, QmdCollection>,
    /// Maximum results to retrieve.
    pub max_results: usize,
    /// Search timeout in milliseconds.
    pub timeout_ms: u64,
    /// Working directory for QMD commands.
    pub work_dir: PathBuf,
    /// Named QMD index used to isolate Moltis-managed collections.
    pub index_name: String,
    /// Optional environment overrides for spawned qmd commands.
    pub env_overrides: HashMap<String, String>,
}

impl Default for QmdManagerConfig {
    fn default() -> Self {
        Self {
            command: "qmd".into(),
            collections: HashMap::new(),
            max_results: 10,
            timeout_ms: 30_000,
            work_dir: PathBuf::from("."),
            index_name: "moltis".into(),
            env_overrides: HashMap::new(),
        }
    }
}

/// A single QMD collection configuration.
#[derive(Debug, Clone)]
pub struct QmdCollection {
    /// Filesystem root for this collection.
    pub path: PathBuf,
    /// Glob mask used when indexing.
    pub glob: String,
}

/// Search mode for QMD queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    /// BM25 keyword search.
    Keyword,
    /// Vector similarity search.
    Vector,
    /// Hybrid search, with optional reranking.
    Hybrid { rerank: bool },
}

impl SearchMode {
    fn command_name(&self) -> &'static str {
        match self {
            Self::Keyword => "search",
            Self::Vector => "vsearch",
            Self::Hybrid { .. } => "query",
        }
    }
}

/// A search result from QMD JSON output.
#[derive(Debug, Clone, Deserialize)]
pub struct QmdSearchResult {
    /// Short QMD docid (without or with a leading '#').
    pub docid: String,
    /// Display path emitted by QMD.
    pub file: String,
    /// Line number where the snippet starts.
    #[serde(default = "default_qmd_line", alias = "from")]
    pub line: i64,
    /// Relevance score (0.0-1.0).
    pub score: f32,
    /// Optional title.
    #[serde(default)]
    pub title: Option<String>,
    /// Optional folder context.
    #[serde(default)]
    pub context: Option<String>,
    /// Snippet output for normal JSON search responses.
    #[serde(default)]
    pub snippet: Option<String>,
    /// Full body when `--full` is used.
    #[serde(default)]
    pub body: Option<String>,
}

impl QmdSearchResult {
    /// Canonical `#docid` form understood by `qmd get`.
    pub fn docid_ref(&self) -> String {
        if self.docid.starts_with('#') {
            self.docid.clone()
        } else {
            format!("#{}", self.docid)
        }
    }

    /// Best available text payload from the search result.
    pub fn text(&self) -> String {
        self.body
            .clone()
            .or_else(|| self.snippet.clone())
            .unwrap_or_default()
    }
}

fn default_qmd_line() -> i64 {
    1
}

/// Status of the QMD backend.
#[derive(Debug, Clone, Serialize)]
pub struct QmdStatus {
    /// Whether QMD is available.
    pub available: bool,
    /// QMD version string.
    pub version: Option<String>,
    /// Number of indexed files per collection. Presently best-effort only.
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

    /// Expose the active index name.
    pub fn index_name(&self) -> &str {
        &self.config.index_name
    }

    /// Expose configured collections.
    pub fn collections(&self) -> &HashMap<String, QmdCollection> {
        &self.config.collections
    }

    /// Check if QMD is available on this system.
    pub async fn is_available(&self) -> bool {
        {
            let cached = self.available.read().await;
            if let Some(available) = *cached {
                return available;
            }
        }

        let available = self.check_qmd_available().await;
        *self.available.write().await = Some(available);
        available
    }

    async fn check_qmd_available(&self) -> bool {
        match Command::new(&self.config.command)
            .arg("--version")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .envs(&self.config.env_overrides)
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                let version = String::from_utf8_lossy(&output.stdout);
                info!(version = %version.trim(), "QMD is available");
                true
            },
            Ok(_) => {
                info!("QMD command failed with non-zero exit code");
                false
            },
            Err(error) => {
                debug!(%error, "QMD is not available");
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

        QmdStatus {
            available: true,
            version: self.version().await.ok(),
            indexed_files: HashMap::new(),
            error: None,
        }
    }

    /// Return the current QMD version.
    pub async fn version(&self) -> Result<String> {
        let output = Command::new(&self.config.command)
            .arg("--version")
            .envs(&self.config.env_overrides)
            .output()
            .await?;
        if !output.status.success() {
            return Err(Error::CommandFailed {
                command: "version".into(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().into(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn command_with_index(&self) -> Command {
        let mut command = Command::new(&self.config.command);
        if !self.config.index_name.is_empty() {
            command.arg("--index").arg(&self.config.index_name);
        }
        command.current_dir(&self.config.work_dir);
        command.envs(&self.config.env_overrides);
        command
    }

    async fn run_with_timeout(&self, mut command: Command) -> Result<std::process::Output> {
        let timeout_duration = Duration::from_millis(self.config.timeout_ms);
        match timeout(timeout_duration, command.output()).await {
            Ok(result) => Ok(result?),
            Err(_) => Err(Error::Timeout(self.config.timeout_ms)),
        }
    }

    async fn collection_exists(&self, name: &str) -> Result<bool> {
        let mut command = self.command_with_index();
        command
            .arg("collection")
            .arg("show")
            .arg(name)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        Ok(self.run_with_timeout(command).await?.status.success())
    }

    /// Ensure a single collection exists in the selected QMD index.
    ///
    /// Idempotent — if the collection already exists, this is a no-op.
    /// Useful for registering collections discovered at runtime (e.g. per-project
    /// code index directories) rather than only those pre-configured at startup.
    pub async fn ensure_collection(&self, name: &str, collection: &QmdCollection) -> Result<()> {
        if !self.is_available().await {
            return Err(Error::NotAvailable);
        }

        if self.collection_exists(name).await? {
            return Ok(());
        }

        let mut command = self.command_with_index();
        command
            .arg("collection")
            .arg("add")
            .arg(&collection.path)
            .arg("--name")
            .arg(name)
            .arg("--mask")
            .arg(&collection.glob)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output = self.run_with_timeout(command).await?;
        if !output.status.success() {
            return Err(Error::CommandFailed {
                command: format!("collection add {name}"),
                stderr: String::from_utf8_lossy(&output.stderr).trim().into(),
            });
        }

        Ok(())
    }

    /// Ensure all pre-configured collections exist in the selected QMD index.
    ///
    /// Iterates the collections set at construction time and registers any
    /// that don't already exist. For dynamic per-call registration, use
    /// [`ensure_collection`](Self::ensure_collection) instead.
    pub async fn ensure_collections(&self) -> Result<()> {
        for (name, collection) in &self.config.collections {
            self.ensure_collection(name, collection).await?;
        }
        Ok(())
    }

    /// Refresh indexed content, and optionally embeddings, after a write or startup sync.
    pub async fn refresh_index(&self, enable_embeddings: bool) -> Result<()> {
        self.ensure_collections().await?;

        let mut update = self.command_with_index();
        update
            .arg("update")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = self.run_with_timeout(update).await?;
        if !output.status.success() {
            return Err(Error::CommandFailed {
                command: "update".into(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().into(),
            });
        }

        if enable_embeddings {
            let mut embed = self.command_with_index();
            embed
                .arg("embed")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let output = self.run_with_timeout(embed).await?;
            if !output.status.success() {
                return Err(Error::CommandFailed {
                    command: "embed".into(),
                    stderr: String::from_utf8_lossy(&output.stderr).trim().into(),
                });
            }
        }

        Ok(())
    }

    /// Search using the specified mode.
    pub async fn search(
        &self,
        query: &str,
        mode: SearchMode,
        limit: usize,
    ) -> Result<Vec<QmdSearchResult>> {
        if !self.is_available().await {
            return Err(Error::NotAvailable);
        }

        let mut command = self.command_with_index();
        let effective_limit = if limit == 0 {
            self.config.max_results.max(1)
        } else {
            limit.min(self.config.max_results.max(1))
        };
        command
            .arg(mode.command_name())
            .arg("--json")
            .arg("-n")
            .arg(effective_limit.to_string())
            .arg(query)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if matches!(mode, SearchMode::Hybrid { rerank: false }) {
            command.arg("--no-rerank");
        }

        let output = self.run_with_timeout(command).await?;
        if !output.status.success() {
            return Err(Error::CommandFailed {
                command: mode.command_name().into(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().into(),
            });
        }

        let results: Vec<QmdSearchResult> = serde_json::from_slice(&output.stdout)?;
        debug!(query = %query, ?mode, results = results.len(), "QMD search completed");
        Ok(results)
    }

    /// Fast keyword search using BM25.
    pub async fn keyword_search(&self, query: &str, limit: usize) -> Result<Vec<QmdSearchResult>> {
        self.search(query, SearchMode::Keyword, limit).await
    }

    /// Vector similarity search.
    pub async fn vector_search(&self, query: &str, limit: usize) -> Result<Vec<QmdSearchResult>> {
        self.search(query, SearchMode::Vector, limit).await
    }

    /// Hybrid search, optionally disabling reranking.
    pub async fn hybrid_search(
        &self,
        query: &str,
        limit: usize,
        rerank: bool,
    ) -> Result<Vec<QmdSearchResult>> {
        self.search(query, SearchMode::Hybrid { rerank }, limit)
            .await
    }

    /// Retrieve document content by path or docid.
    pub async fn get_document(
        &self,
        target: &str,
        from_line: Option<i64>,
        max_lines: Option<usize>,
    ) -> Result<String> {
        if !self.is_available().await {
            return Err(Error::NotAvailable);
        }

        let mut command = self.command_with_index();
        command
            .arg("get")
            .arg(target)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(from_line) = from_line {
            command.arg("--from").arg(from_line.to_string());
        }
        if let Some(max_lines) = max_lines {
            command.arg("-l").arg(max_lines.to_string());
        }

        let output = self.run_with_timeout(command).await?;
        if !output.status.success() {
            return Err(Error::CommandFailed {
                command: "get".into(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().into(),
            });
        }

        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt};

    use tempfile::TempDir;

    use super::*;

    fn real_qmd_available() -> bool {
        std::process::Command::new("qmd")
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn real_qmd_env(root: &TempDir) -> HashMap<String, String> {
        let config_home = root.path().join("config");
        let cache_home = root.path().join("cache");
        let home_dir = root.path().join("home");
        fs::create_dir_all(&config_home).unwrap();
        fs::create_dir_all(&cache_home).unwrap();
        fs::create_dir_all(&home_dir).unwrap();
        HashMap::from([
            (
                "XDG_CONFIG_HOME".into(),
                config_home.to_string_lossy().into_owned(),
            ),
            (
                "XDG_CACHE_HOME".into(),
                cache_home.to_string_lossy().into_owned(),
            ),
            ("HOME".into(), home_dir.to_string_lossy().into_owned()),
        ])
    }

    fn write_fake_qmd_script(dir: &TempDir, log_path: &std::path::Path) -> PathBuf {
        let script = dir.path().join("qmd");
        let contents = format!(
            r#"#!/bin/sh
printf '%s\n' "$*" >> "{}"
if [ "$1" = "--version" ]; then
  echo "qmd 2.1.0"
  exit 0
fi
if [ "$1" = "--index" ]; then
  shift
  shift
fi
case "$1" in
  collection)
    if [ "$2" = "show" ]; then
      exit 1
    fi
    exit 0
    ;;
  update)
    exit 0
    ;;
  embed)
    exit 0
    ;;
  search|vsearch)
    echo '[{{"docid":"abc123","file":"memory/notes.md","line":7,"score":0.91,"snippet":"keyword result"}}]'
    exit 0
    ;;
  query)
    echo '[{{"docid":"abc123","file":"memory/notes.md","line":7,"score":0.93,"snippet":"hybrid result"}}]'
    exit 0
    ;;
  get)
    echo 'retrieved body'
    exit 0
    ;;
esac
exit 0
"#,
            log_path.display()
        );
        fs::write(&script, contents).unwrap();
        let mut permissions = fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).unwrap();
        script
    }

    #[tokio::test]
    async fn manager_config_default() {
        let config = QmdManagerConfig::default();
        assert_eq!(config.command, "qmd");
        assert_eq!(config.max_results, 10);
        assert_eq!(config.timeout_ms, 30_000);
        assert_eq!(config.index_name, "moltis");
    }

    #[test]
    fn search_mode_commands() {
        assert_eq!(SearchMode::Keyword.command_name(), "search");
        assert_eq!(SearchMode::Vector.command_name(), "vsearch");
        assert_eq!(SearchMode::Hybrid { rerank: true }.command_name(), "query");
    }

    #[tokio::test]
    async fn manager_unavailable() {
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

    #[tokio::test]
    async fn refresh_index_bootstraps_collections_and_embeddings() {
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("qmd.log");
        let script = write_fake_qmd_script(&tmp, &log_path);

        let manager = QmdManager::new(QmdManagerConfig {
            command: script.to_string_lossy().into_owned(),
            collections: HashMap::from([("notes".into(), QmdCollection {
                path: tmp.path().join("memory"),
                glob: "**/*.md".into(),
            })]),
            max_results: 10,
            timeout_ms: 5_000,
            work_dir: tmp.path().to_path_buf(),
            index_name: "test-index".into(),
            env_overrides: HashMap::new(),
        });
        *manager.available.write().await = Some(true);

        manager.refresh_index(true).await.unwrap();

        let log = fs::read_to_string(&log_path).unwrap();
        assert!(log.contains("--index test-index collection show notes"));
        assert!(log.contains("collection add"));
        assert!(log.contains("--name notes"));
        assert!(log.contains("--mask **/*.md"));
        assert!(log.contains("--index test-index update"));
        assert!(log.contains("--index test-index embed"));
    }

    #[tokio::test]
    async fn hybrid_search_uses_current_json_shape_and_no_rerank_flag() {
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("qmd.log");
        let script = write_fake_qmd_script(&tmp, &log_path);

        let manager = QmdManager::new(QmdManagerConfig {
            command: script.to_string_lossy().into_owned(),
            timeout_ms: 5_000,
            work_dir: tmp.path().to_path_buf(),
            index_name: "search-index".into(),
            ..Default::default()
        });
        *manager.available.write().await = Some(true);

        let results = manager.hybrid_search("auth flow", 5, false).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].docid_ref(), "#abc123");
        assert_eq!(results[0].line, 7);
        assert_eq!(results[0].text(), "hybrid result");

        let log = fs::read_to_string(&log_path).unwrap();
        assert!(log.contains("--index search-index query --json -n 5 auth flow --no-rerank"));
    }

    #[tokio::test]
    async fn live_qmd_keyword_search_and_get_round_trip() {
        if !real_qmd_available() {
            eprintln!("skipping live qmd test because qmd is not installed");
            return;
        }

        let tmp = TempDir::new().unwrap();
        let memory_dir = tmp.path().join("memory");
        fs::create_dir_all(&memory_dir).unwrap();
        fs::write(
            memory_dir.join("notes.md"),
            "# Notes\n\nalpha\nbeta\ngamma keyword target\ndelta",
        )
        .unwrap();

        let manager = QmdManager::new(QmdManagerConfig {
            command: "qmd".into(),
            collections: HashMap::from([("notes".into(), QmdCollection {
                path: memory_dir.clone(),
                glob: "**/*.md".into(),
            })]),
            max_results: 5,
            timeout_ms: 60_000,
            work_dir: tmp.path().to_path_buf(),
            index_name: "moltis-live-manager".into(),
            env_overrides: real_qmd_env(&tmp),
        });

        manager.refresh_index(false).await.unwrap();
        let results = manager.keyword_search("keyword target", 5).await.unwrap();
        assert!(!results.is_empty(), "expected keyword result from live qmd");
        let first = &results[0];
        assert!(
            first.file.contains("notes.md"),
            "expected search result to reference notes.md, got {}",
            first.file
        );

        let body = manager
            .get_document(&first.docid_ref(), Some(first.line), Some(10))
            .await
            .unwrap();
        assert!(
            body.contains("gamma keyword target"),
            "expected qmd get to return indexed content, got: {body}"
        );
    }
}
