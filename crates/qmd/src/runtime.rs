use std::{path::Path, sync::Arc};

use {
    async_trait::async_trait,
    moltis_agents::memory_writer::{MemoryWriteResult, MemoryWriter},
    moltis_memory::{
        Result as MemoryResult,
        manager::{MemoryManager, MemoryStatus, SyncReport},
        runtime::MemoryRuntime,
        schema::ChunkRow,
        search::SearchResult,
    },
};

use crate::manager::{QmdManager, QmdSearchResult, SearchMode};

const DEFAULT_QMD_GET_LINES: usize = 120;

/// QMD-backed runtime that keeps a lightweight local index for path metadata
/// and delegates search and retrieval to the QMD CLI.
pub struct QmdMemoryRuntime {
    manager: Arc<QmdManager>,
    fallback: Arc<MemoryManager>,
    disable_rag: bool,
}

impl QmdMemoryRuntime {
    pub fn new(manager: Arc<QmdManager>, fallback: Arc<MemoryManager>, disable_rag: bool) -> Self {
        Self {
            manager,
            fallback,
            disable_rag,
        }
    }

    fn search_mode(&self) -> SearchMode {
        if self.disable_rag {
            SearchMode::Keyword
        } else {
            SearchMode::Hybrid {
                rerank: self.fallback.llm_reranking_enabled(),
            }
        }
    }

    async fn refresh_qmd_index(&self) -> anyhow::Result<()> {
        Ok(self.manager.refresh_index(!self.disable_rag).await?)
    }

    async fn resolve_result_path(&self, result: &QmdSearchResult) -> anyhow::Result<String> {
        if let Some(path) = self
            .fallback
            .resolve_file_by_hash_prefix(&result.docid)
            .await?
        {
            return Ok(path);
        }

        let file_path = Path::new(&result.file);
        if file_path.is_absolute() {
            return Ok(result.file.clone());
        }

        if let Some(data_dir) = self.fallback.data_dir() {
            return Ok(data_dir.join(&result.file).to_string_lossy().into_owned());
        }

        Ok(result.file.clone())
    }

    async fn convert_result(&self, result: QmdSearchResult) -> anyhow::Result<SearchResult> {
        let path = self.resolve_result_path(&result).await?;
        let start_line = result.line.max(1);
        Ok(SearchResult {
            chunk_id: format!("qmd:{}:{}", result.docid_ref(), start_line),
            path: path.clone(),
            source: qmd_source_for_path(&path),
            start_line,
            end_line: start_line,
            score: result.score,
            text: result.text(),
        })
    }

    fn parse_chunk_id(id: &str) -> Option<(String, i64)> {
        let payload = id.strip_prefix("qmd:")?;
        let (docid, line) = payload.rsplit_once(':')?;
        let start_line = line.parse::<i64>().ok()?.max(1);
        Some((docid.to_string(), start_line))
    }
}

fn qmd_source_for_path(path: &str) -> String {
    let normalized_components: Vec<String> = Path::new(path)
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(segment) => {
                Some(segment.to_string_lossy().to_ascii_lowercase())
            },
            _ => None,
        })
        .collect();
    if normalized_components
        .iter()
        .any(|segment| segment == "memory")
        || normalized_components
            .last()
            .is_some_and(|segment| segment == "memory.md")
    {
        "longterm".into()
    } else {
        "daily".into()
    }
}

fn strip_qmd_context_header(text: &str) -> &str {
    if let Some(content) = text.strip_prefix("Folder Context:")
        && let Some((_, body)) = content.split_once("\n---\n\n")
    {
        return body;
    }
    text
}

#[async_trait]
impl MemoryWriter for QmdMemoryRuntime {
    async fn write_memory(
        &self,
        file: &str,
        content: &str,
        append: bool,
    ) -> anyhow::Result<MemoryWriteResult> {
        let result = self.fallback.write_memory(file, content, append).await?;
        self.refresh_qmd_index().await.map_err(|error| {
            anyhow::anyhow!(
                "memory content was saved to {} but QMD reindex failed: {error}",
                result.location
            )
        })?;
        Ok(result)
    }
}

#[async_trait]
impl MemoryRuntime for QmdMemoryRuntime {
    fn backend_name(&self) -> &'static str {
        "qmd"
    }

    fn data_dir(&self) -> Option<&Path> {
        self.fallback.data_dir()
    }

    fn has_embeddings(&self) -> bool {
        !self.disable_rag
    }

    fn citation_mode(&self) -> moltis_memory::config::CitationMode {
        self.fallback.citation_mode()
    }

    fn llm_reranking_enabled(&self) -> bool {
        self.fallback.llm_reranking_enabled()
    }

    async fn sync(&self) -> MemoryResult<SyncReport> {
        let report = self.fallback.sync().await?;
        self.refresh_qmd_index()
            .await
            .map_err(|e| moltis_memory::Error::Validation(e.to_string()))?;
        Ok(report)
    }

    async fn sync_path(&self, path: &Path) -> MemoryResult<bool> {
        let changed = self.fallback.sync_path(path).await?;
        self.refresh_qmd_index()
            .await
            .map_err(|e| moltis_memory::Error::Validation(e.to_string()))?;
        Ok(changed)
    }

    async fn remove_path(&self, path: &Path) -> MemoryResult<bool> {
        let removed = self.fallback.remove_path(path).await?;
        self.refresh_qmd_index()
            .await
            .map_err(|e| moltis_memory::Error::Validation(e.to_string()))?;
        Ok(removed)
    }

    async fn search(&self, query: &str, limit: usize) -> MemoryResult<Vec<SearchResult>> {
        let qmd_results = match self.search_mode() {
            SearchMode::Keyword => self
                .manager
                .keyword_search(query, limit)
                .await
                .map_err(|e| moltis_memory::Error::Validation(e.to_string()))?,
            SearchMode::Vector => self
                .manager
                .vector_search(query, limit)
                .await
                .map_err(|e| moltis_memory::Error::Validation(e.to_string()))?,
            SearchMode::Hybrid { rerank } => self
                .manager
                .hybrid_search(query, limit, rerank)
                .await
                .map_err(|e| moltis_memory::Error::Validation(e.to_string()))?,
        };

        let mut results = Vec::with_capacity(qmd_results.len());
        for result in qmd_results {
            results.push(
                self.convert_result(result)
                    .await
                    .map_err(|e| moltis_memory::Error::Validation(e.to_string()))?,
            );
        }
        Ok(results)
    }

    async fn get_chunk(&self, id: &str) -> MemoryResult<Option<ChunkRow>> {
        let Some((docid, start_line)) = Self::parse_chunk_id(id) else {
            return self.fallback.get_chunk(id).await;
        };

        let path = self
            .fallback
            .resolve_file_by_hash_prefix(&docid)
            .await?
            .unwrap_or_else(|| docid.clone());

        let body = self
            .manager
            .get_document(&docid, Some(start_line), Some(DEFAULT_QMD_GET_LINES))
            .await
            .map_err(|e| moltis_memory::Error::Validation(e.to_string()))?;
        let text = strip_qmd_context_header(&body).trim().to_string();
        let line_count = text.lines().count().max(1) as i64;

        Ok(Some(ChunkRow {
            id: id.to_string(),
            path: path.clone(),
            source: qmd_source_for_path(&path),
            start_line,
            end_line: start_line + line_count - 1,
            hash: String::new(),
            model: if self.disable_rag {
                "qmd-keyword".into()
            } else {
                "qmd".into()
            },
            text,
            embedding: None,
            updated_at: String::new(),
        }))
    }

    async fn status(&self) -> MemoryResult<MemoryStatus> {
        let mut status = self.fallback.status().await?;
        status.embedding_model = if self.disable_rag {
            "qmd (keyword)".into()
        } else if self.fallback.llm_reranking_enabled() {
            "qmd (hybrid+rerank)".into()
        } else {
            "qmd (hybrid)".into()
        };
        Ok(status)
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
    };

    use {
        sha2::{Digest, Sha256},
        tempfile::TempDir,
    };

    use super::*;

    fn real_qmd_available() -> bool {
        std::process::Command::new("qmd")
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn real_qmd_env(root: &TempDir) -> std::collections::HashMap<String, String> {
        let config_home = root.path().join("config");
        let cache_home = root.path().join("cache");
        let home_dir = root.path().join("home");
        fs::create_dir_all(&config_home).unwrap();
        fs::create_dir_all(&cache_home).unwrap();
        fs::create_dir_all(&home_dir).unwrap();
        std::collections::HashMap::from([
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

    fn write_fake_qmd_script(dir: &TempDir, log_path: &Path) -> PathBuf {
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
  get)
    echo 'gamma'
    echo 'delta'
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

    #[test]
    fn strips_qmd_context_header() {
        let text = "Folder Context: test\n---\n\nhello";
        assert_eq!(strip_qmd_context_header(text), "hello");
    }

    #[test]
    fn parses_qmd_chunk_ids() {
        assert_eq!(
            QmdMemoryRuntime::parse_chunk_id("qmd:#abc123:42"),
            Some(("#abc123".into(), 42))
        );
        assert_eq!(QmdMemoryRuntime::parse_chunk_id("builtin:1"), None);
    }

    #[test]
    fn classifies_memory_paths_case_insensitively() {
        assert_eq!(qmd_source_for_path("/tmp/moltis/MEMORY.md"), "longterm");
        assert_eq!(
            qmd_source_for_path("/tmp/moltis/agents/ops/memory/notes.md"),
            "longterm"
        );
        assert_eq!(
            qmd_source_for_path("/tmp/moltis/agents/ops/Memory/notes.md"),
            "longterm"
        );
        assert_eq!(qmd_source_for_path("/tmp/moltis/daily/journal.md"), "daily");
    }

    #[tokio::test]
    async fn search_mode_matches_rag_and_rerank_flags() {
        let tmp = TempDir::new().unwrap();
        let pool = sqlx::SqlitePool::connect(":memory:").await.unwrap();
        moltis_memory::schema::run_migrations(&pool).await.unwrap();
        let config = moltis_memory::config::MemoryConfig {
            db_path: ":memory:".into(),
            data_dir: Some(tmp.path().to_path_buf()),
            llm_reranking: true,
            ..Default::default()
        };
        let store = Box::new(moltis_memory::store_sqlite::SqliteMemoryStore::new(pool));
        let fallback = Arc::new(MemoryManager::keyword_only(config, store));
        let manager = Arc::new(QmdManager::new(crate::manager::QmdManagerConfig::default()));

        let runtime = QmdMemoryRuntime::new(manager, fallback, false);
        assert_eq!(runtime.search_mode(), SearchMode::Hybrid { rerank: true });
    }

    #[tokio::test]
    async fn get_chunk_uses_qmd_docid_and_resolves_real_file_path() {
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("qmd.log");
        let script = write_fake_qmd_script(&tmp, &log_path);

        let memory_dir = tmp.path().join("memory");
        fs::create_dir_all(&memory_dir).unwrap();
        let file_path = memory_dir.join("notes.md");
        let content = "alpha\nbeta\ngamma\ndelta";
        fs::write(&file_path, content).unwrap();

        let pool = sqlx::SqlitePool::connect(":memory:").await.unwrap();
        moltis_memory::schema::run_migrations(&pool).await.unwrap();
        let config = moltis_memory::config::MemoryConfig {
            db_path: ":memory:".into(),
            data_dir: Some(tmp.path().to_path_buf()),
            memory_dirs: vec![memory_dir.clone()],
            ..Default::default()
        };
        let store = Box::new(moltis_memory::store_sqlite::SqliteMemoryStore::new(pool));
        let fallback = Arc::new(MemoryManager::keyword_only(config, store));
        fallback.sync().await.unwrap();

        let docid = format!("{:x}", Sha256::digest(content.as_bytes()))[..6].to_string();
        let manager = Arc::new(QmdManager::new(crate::manager::QmdManagerConfig {
            command: script.to_string_lossy().into_owned(),
            timeout_ms: 5_000,
            work_dir: tmp.path().to_path_buf(),
            index_name: "runtime-index".into(),
            ..Default::default()
        }));
        let runtime = QmdMemoryRuntime::new(manager, fallback, false);

        let chunk = runtime
            .get_chunk(&format!("qmd:#{docid}:3"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(chunk.path, file_path.to_string_lossy());
        assert_eq!(chunk.start_line, 3);
        assert_eq!(chunk.end_line, 4);
        assert_eq!(chunk.text, "gamma\ndelta");

        let log = fs::read_to_string(&log_path).unwrap();
        assert!(log.contains("--index runtime-index get #"));
        assert!(log.contains("--from 3 -l 120"));
    }

    #[tokio::test]
    async fn live_qmd_runtime_search_and_get_chunk_round_trip() {
        if !real_qmd_available() {
            eprintln!("skipping live qmd runtime test because qmd is not installed");
            return;
        }

        let tmp = TempDir::new().unwrap();
        let memory_dir = tmp.path().join("memory");
        fs::create_dir_all(&memory_dir).unwrap();
        let file_path = memory_dir.join("notes.md");
        let content = "# Notes\n\nalpha\nbeta\nruntime keyword target\ndelta";
        fs::write(&file_path, content).unwrap();

        let pool = sqlx::SqlitePool::connect(":memory:").await.unwrap();
        moltis_memory::schema::run_migrations(&pool).await.unwrap();
        let config = moltis_memory::config::MemoryConfig {
            db_path: ":memory:".into(),
            data_dir: Some(tmp.path().to_path_buf()),
            memory_dirs: vec![memory_dir.clone()],
            ..Default::default()
        };
        let store = Box::new(moltis_memory::store_sqlite::SqliteMemoryStore::new(pool));
        let fallback = Arc::new(MemoryManager::keyword_only(config, store));

        let manager = Arc::new(QmdManager::new(crate::manager::QmdManagerConfig {
            command: "qmd".into(),
            collections: std::collections::HashMap::from([(
                "notes".into(),
                crate::manager::QmdCollection {
                    path: memory_dir.clone(),
                    glob: "**/*.md".into(),
                },
            )]),
            max_results: 5,
            timeout_ms: 60_000,
            work_dir: tmp.path().to_path_buf(),
            index_name: "moltis-live-runtime".into(),
            env_overrides: real_qmd_env(&tmp),
        }));
        let runtime = QmdMemoryRuntime::new(manager, fallback, true);

        runtime.sync().await.unwrap();
        let results = runtime.search("runtime keyword target", 5).await.unwrap();
        assert!(
            !results.is_empty(),
            "expected runtime search results from live qmd"
        );
        assert!(
            results[0].chunk_id.starts_with("qmd:#"),
            "expected qmd chunk id, got {}",
            results[0].chunk_id
        );

        let chunk = runtime
            .get_chunk(&results[0].chunk_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(chunk.path, file_path.to_string_lossy());
        assert!(
            chunk.text.contains("runtime keyword target"),
            "expected qmd-backed get_chunk to return file content, got: {}",
            chunk.text
        );
    }
}
