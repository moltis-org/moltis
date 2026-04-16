//! Agent tools for codebase indexing.
//!
//! Three tools exposed to the LLM agent:
//! - `codebase_search` — hybrid (keyword + vector) search across indexed code
//! - `codebase_peek` — list indexable files in a project directory
//! - `codebase_status` — report indexing status for a project

use std::path::PathBuf;
use std::sync::Arc;

use {async_trait::async_trait, moltis_agents::tool_registry::AgentTool, serde_json::json};

use crate::CodeIndex;

#[cfg(test)]
use crate::CodeIndexConfig;

#[cfg(feature = "qmd")]
use crate::Error;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Parse a required string parameter from the tool invocation JSON.
fn require_str(params: &serde_json::Value, key: &str) -> anyhow::Result<String> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("missing required parameter '{key}'"))
}


/// Parse an optional usize parameter, defaulting to the given value.
fn opt_usize_or(params: &serde_json::Value, key: &str, default: usize) -> usize {
    params
        .get(key)
        .and_then(|v| v.as_u64())
        .unwrap_or(default as u64) as usize
}

// ---------------------------------------------------------------------------
// CodebaseSearchTool
// ---------------------------------------------------------------------------

/// Search the codebase index for a project using hybrid (keyword + vector) search.
///
/// Requires a QMD backend. Returns ranked results with file path, line range,
/// score, and matched text.
pub struct CodebaseSearchTool {
    index: Arc<CodeIndex>,
}

impl CodebaseSearchTool {
    pub fn new(index: Arc<CodeIndex>) -> Self {
        Self { index }
    }
}

#[async_trait]
impl AgentTool for CodebaseSearchTool {
    fn name(&self) -> &str {
        "codebase_search"
    }

    fn description(&self) -> &str {
        "Search the codebase index for relevant code chunks. \
         Uses hybrid search (keyword + vector embeddings) to find functions, \
         types, patterns, and code across all indexed files in a project. \
         Returns file path, line range, relevance score, and matched text."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["project_id", "query"],
            "properties": {
                "project_id": {
                    "type": "string",
                    "description": "Project identifier to scope the search to."
                },
                "query": {
                    "type": "string",
                    "description": "Natural language or keyword search query."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return.",
                    "default": 10
                }
            }
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let project_id = require_str(&params, "project_id")?;
        let query = require_str(&params, "query")?;
        let limit = opt_usize_or(&params, "limit", 10);

        let results = self.index.search(&project_id, &query, limit).await?;

        let items: Vec<serde_json::Value> = results
            .iter()
            .map(|r| {
                json!({
                    "chunk_id": r.chunk_id,
                    "path": r.path,
                    "start_line": r.start_line,
                    "end_line": r.end_line,
                    "score": r.score,
                    "text": r.text,
                    "source": r.source,
                })
            })
            .collect();

        Ok(json!({
            "results": items,
            "total": items.len(),
            "project_id": project_id,
        }))
    }
}

// ---------------------------------------------------------------------------
// CodebasePeekTool
// ---------------------------------------------------------------------------

/// List the files that would be indexed for a given project directory.
///
/// This is a read-only operation — it discovers git-tracked files and
/// applies the configured filters, but does not trigger indexing.
pub struct CodebasePeekTool {
    index: Arc<CodeIndex>,
}

impl CodebasePeekTool {
    pub fn new(index: Arc<CodeIndex>) -> Self {
        Self { index }
    }
}

#[async_trait]
impl AgentTool for CodebasePeekTool {
    fn name(&self) -> &str {
        "codebase_peek"
    }

    fn description(&self) -> &str {
        "List files that would be indexed for a project directory. \
         Discovers git-tracked files, applies extension and size filters, \
         and returns the list with language and size info. \
         Does NOT trigger indexing — use this to preview what gets indexed."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["project_dir"],
            "properties": {
                "project_dir": {
                    "type": "string",
                    "description": "Absolute path to the project directory (must contain a .git folder)."
                }
            }
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let dir = require_str(&params, "project_dir")?;
        let project_dir = PathBuf::from(&dir);

        if !project_dir.is_dir() {
            return Ok(json!({
                "error": format!("directory does not exist: {dir}"),
            }));
        }

        let files = self.index.list_indexable_files(&project_dir)?;

        let total_size: u64 = files.iter().map(|f| f.size).sum();

        let items: Vec<serde_json::Value> = files
            .iter()
            .map(|f| {
                json!({
                    "path": f.relative_path.to_string_lossy(),
                    "language": format!("{:?}", f.language).to_lowercase(),
                    "size": f.size,
                })
            })
            .collect();

        Ok(json!({
            "files": items,
            "total_files": items.len(),
            "total_size_bytes": total_size,
            "project_dir": dir,
        }))
    }
}

// ---------------------------------------------------------------------------
// CodebaseStatusTool
// ---------------------------------------------------------------------------

/// Report the indexing status for a project.
///
/// Returns file counts, backend type, and last sync time.
/// Works with or without a QMD backend — config-only instances report
/// discover stats without search capability.
pub struct CodebaseStatusTool {
    index: Arc<CodeIndex>,
}

impl CodebaseStatusTool {
    pub fn new(index: Arc<CodeIndex>) -> Self {
        Self { index }
    }
}

#[async_trait]
impl AgentTool for CodebaseStatusTool {
    fn name(&self) -> &str {
        "codebase_status"
    }

    fn description(&self) -> &str {
        "Report the indexing status for a project. Returns file counts, \
         backend type, and whether search is available. Use this to check \
         if a project directory is indexed and ready for codebase_search."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["project_id", "project_dir"],
            "properties": {
                "project_id": {
                    "type": "string",
                    "description": "Project identifier."
                },
                "project_dir": {
                    "type": "string",
                    "description": "Absolute path to the project directory."
                }
            }
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let project_id = require_str(&params, "project_id")?;
        let dir = require_str(&params, "project_dir")?;
        let project_dir = PathBuf::from(&dir);

        if !project_dir.is_dir() {
            return Ok(json!({
                "error": format!("directory does not exist: {dir}"),
            }));
        }

        match self.index.status(&project_id, &project_dir).await {
            Ok(status) => Ok(json!({
                "project_id": status.project_id,
                "total_files": status.total_files,
                "total_chunks": status.total_chunks,
                "last_sync_ms": status.last_sync_ms,
                "embedding_model": status.embedding_model,
                "backend": status.backend,
            })),
            Err(Error::BackendUnavailable(msg)) => Ok(json!({
                "project_id": project_id,
                "error": msg,
                "search_available": false,
            })),
            Err(e) => Err(anyhow::anyhow!("{e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// Registration helper
// ---------------------------------------------------------------------------

/// Register all code-index tools into a [`ToolRegistry`].
///
/// Only call this when the `qmd` feature is enabled — the tools require
/// a QMD-backed [`CodeIndex`].
pub fn register_tools(
    registry: &mut moltis_agents::tool_registry::ToolRegistry,
    index: Arc<CodeIndex>,
) {
    registry.register(Box::new(CodebaseSearchTool::new(Arc::clone(&index))));
    registry.register(Box::new(CodebasePeekTool::new(Arc::clone(&index))));
    registry.register(Box::new(CodebaseStatusTool::new(index)));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config_only_index() -> Arc<CodeIndex> {
        Arc::new(CodeIndex::config_only(CodeIndexConfig::default()))
    }

    #[tokio::test]
    async fn test_peek_lists_indexable_files() {
        let idx = make_config_only_index();
        let tool = CodebasePeekTool::new(Arc::clone(&idx));

        let repo_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_string_lossy()
            .to_string();

        let result = tool
            .execute(json!({ "project_dir": repo_dir }))
            .await
            .expect("peek should succeed on moltis repo");

        let total = result["total_files"].as_u64().unwrap();
        assert!(total > 0, "moltis repo has indexable files");
    }

    #[tokio::test]
    async fn test_peek_returns_error_for_nonexistent_dir() {
        let idx = make_config_only_index();
        let tool = CodebasePeekTool::new(idx);

        let result = tool
            .execute(json!({ "project_dir": "/nonexistent/path/that/does/not/exist" }))
            .await
            .expect("peek tool itself should not panic");

        assert!(result.get("error").is_some(), "should report directory error");
    }

    #[tokio::test]
    async fn test_search_requires_backend() {
        let idx = make_config_only_index();
        let tool = CodebaseSearchTool::new(idx);

        let result = tool
            .execute(json!({
                "project_id": "test-project",
                "query": "fn main"
            }))
            .await;

        // The tool wraps CodeIndex::search which returns BackendUnavailable.
        // AgentTool::execute returns Result<Value>, so the error propagates.
        assert!(result.is_err(), "config-only search should fail with BackendUnavailable");
    }

    #[tokio::test]
    async fn test_status_reports_config_only() {
        let idx = make_config_only_index();
        let tool = CodebaseStatusTool::new(idx);

        let repo_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_string_lossy()
            .to_string();

        let result = tool
            .execute(json!({
                "project_id": "test-project",
                "project_dir": repo_dir,
            }))
            .await
            .expect("status should succeed on moltis repo");

        // Config-only should report backend as "none (config-only)"
        let backend = result["backend"].as_str().unwrap_or("unknown");
        assert!(
            backend.contains("config-only") || backend == "none",
            "config-only status should report no search backend, got: {backend}"
        );
    }

    #[tokio::test]
    async fn test_status_nonexistent_dir() {
        let idx = make_config_only_index();
        let tool = CodebaseStatusTool::new(idx);

        let result = tool
            .execute(json!({
                "project_id": "test-project",
                "project_dir": "/nonexistent/path/that/does/not/exist",
            }))
            .await
            .expect("status tool itself should not panic");

        assert!(result.get("error").is_some(), "should report directory error");
    }

    #[test]
    fn test_parameter_schemas_are_valid_json() {
        // Verify that all tool schemas produce valid JSON objects.
        let search = CodebaseSearchTool::new(make_config_only_index());
        let schema = search.parameters_schema();
        assert!(schema.is_object());
        assert!(schema["required"].is_array());

        let peek = CodebasePeekTool::new(make_config_only_index());
        let schema = peek.parameters_schema();
        assert!(schema.is_object());
        assert!(schema["required"].is_array());

        let status = CodebaseStatusTool::new(make_config_only_index());
        let schema = status.parameters_schema();
        assert!(schema.is_object());
        assert!(schema["required"].is_array());
    }
}