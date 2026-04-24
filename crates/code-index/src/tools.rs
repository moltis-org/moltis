//! Agent tools for codebase indexing.
//!
//! Three tools exposed to the LLM agent:
//! - `codebase_search` — hybrid (keyword + vector) search across indexed code
//! - `codebase_peek` — list indexable files in a project directory
//! - `codebase_status` — report indexing status for a project

use std::{path::PathBuf, sync::Arc};

use {
    async_trait::async_trait, moltis_agents::tool_registry::AgentTool, moltis_tools::params,
    serde_json::json,
};

use crate::{CodeIndex, Error};

#[cfg(test)]
use crate::CodeIndexConfig;

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

    async fn execute(&self, params_value: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let project_id = params::require_str(&params_value, "project_id")?.to_string();
        let query = params::require_str(&params_value, "query")?.to_string();
        let limit = usize::try_from(params::u64_param(&params_value, "limit", 10)).unwrap_or(10);

        match self.index.search(&project_id, &query, limit).await {
            Ok(results) => {
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
            },
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

    async fn execute(&self, params_value: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let dir = params::require_str(&params_value, "project_dir")?.to_string();
        let project_dir = PathBuf::from(&dir);

        if !project_dir.is_dir() {
            return Ok(json!({
                "error": format!("directory does not exist: {dir}"),
            }));
        }

        let files = match self.index.list_indexable_files(&project_dir) {
            Ok(f) => f,
            Err(e) => {
                return Ok(json!({
                    "error": e.to_string(),
                }));
            },
        };

        let total_size: u64 = files.iter().map(|f| f.size).sum();

        let items: Vec<serde_json::Value> = files
            .iter()
            .map(|f| {
                json!({
                    "path": f.relative_path.to_string_lossy(),
                    "language": serde_json::to_value(f.language)
                        .unwrap_or(serde_json::Value::String("unknown".to_string())),
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

    async fn execute(&self, params_value: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let project_id = params::require_str(&params_value, "project_id")?.to_string();
        let dir = params::require_str(&params_value, "project_dir")?.to_string();
        let project_dir = PathBuf::from(&dir);

        if !project_dir.is_dir() {
            return Ok(json!({
                "error": format!("directory does not exist: {dir}"),
            }));
        }

        match self.index.status(&project_id).await {
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
/// Call this when any backend is configured (QMD or builtin).
/// Tools gracefully degrade to `BackendUnavailable` errors if
/// the backend is config-only.
pub fn register_tools(
    registry: &mut moltis_agents::tool_registry::ToolRegistry,
    index: Arc<CodeIndex>,
) {
    registry.register(Box::new(CodebaseSearchTool::new(Arc::clone(&index))));
    registry.register(Box::new(CodebasePeekTool::new(Arc::clone(&index))));
    registry.register(Box::new(CodebaseStatusTool::new(index)));
}

/// Register all code-index tools, wrapping each in a [`Fn`] before registration.
///
/// This allows callers to wrap tools with project-aware gating or other
/// middleware without duplicating the tool list.
pub fn register_tools_wrapped<W>(
    registry: &mut moltis_agents::tool_registry::ToolRegistry,
    index: Arc<CodeIndex>,
    wrap: W,
) where
    W: Fn(Box<dyn AgentTool>) -> Box<dyn AgentTool>,
{
    registry.register(wrap(Box::new(CodebaseSearchTool::new(Arc::clone(&index)))));
    registry.register(wrap(Box::new(CodebasePeekTool::new(Arc::clone(&index)))));
    registry.register(wrap(Box::new(CodebaseStatusTool::new(index))));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
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

        assert!(
            result.get("error").is_some(),
            "should report directory error"
        );
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
            .await
            .expect("search tool should return Ok even without backend");

        // Config-only search returns a structured error response, not Err.
        assert!(
            result.get("error").is_some(),
            "config-only search should report error"
        );
        assert!(
            result.get("search_available").is_some(),
            "config-only search should report search_available"
        );
        assert_eq!(result["search_available"], false);
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

        // Config-only should report search unavailable (BackendUnavailable error path).
        let search_available = result["search_available"].as_bool().unwrap_or(true);
        assert!(
            !search_available,
            "config-only status should report search_available=false, got: {result}"
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

        assert!(
            result.get("error").is_some(),
            "should report directory error"
        );
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
