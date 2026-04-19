/// Agent tools for memory search, retrieval, and persistence.
use std::{path::PathBuf, sync::Arc};

use {async_trait::async_trait, moltis_agents::tool_registry::AgentTool, serde_json::json};

use crate::{
    runtime::MemoryRuntime,
    writer::{remove_exact_text, validate_memory_path},
};

/// Tool: search memory with a natural language query.
pub struct MemorySearchTool {
    manager: Arc<dyn MemoryRuntime>,
}

impl MemorySearchTool {
    pub fn new(manager: Arc<dyn MemoryRuntime>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl AgentTool for MemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn description(&self) -> &str {
        "Search agent memory using hybrid vector + keyword search. Returns relevant chunks from daily logs and long-term memory files."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return",
                    "default": 5
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let query = params["query"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'query' parameter"))?;
        let limit = params["limit"].as_u64().unwrap_or(5) as usize;

        let results = self.manager.search(query, limit).await?;

        // Determine if we should include citations based on config and result set.
        let include_citations = crate::search::SearchResult::should_include_citations(
            &results,
            self.manager.citation_mode(),
        );

        let items: Vec<serde_json::Value> = results
            .iter()
            .map(|r| {
                let text = if include_citations {
                    r.text_with_citation()
                } else {
                    r.text.clone()
                };
                json!({
                    "chunk_id": r.chunk_id,
                    "path": r.path,
                    "source": r.source,
                    "start_line": r.start_line,
                    "end_line": r.end_line,
                    "score": r.score,
                    "text": text,
                    "citation": format!("{}#{}", r.path, r.start_line),
                })
            })
            .collect();

        Ok(json!({ "results": items, "citations_enabled": include_citations }))
    }
}

/// Tool: get a specific memory chunk by ID.
pub struct MemoryGetTool {
    manager: Arc<dyn MemoryRuntime>,
}

impl MemoryGetTool {
    pub fn new(manager: Arc<dyn MemoryRuntime>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl AgentTool for MemoryGetTool {
    fn name(&self) -> &str {
        "memory_get"
    }

    fn description(&self) -> &str {
        "Retrieve a specific memory chunk by its ID. Use this to get the full text of a chunk found via memory_search."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "chunk_id": {
                    "type": "string",
                    "description": "The chunk ID to retrieve"
                }
            },
            "required": ["chunk_id"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let chunk_id = params["chunk_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'chunk_id' parameter"))?;

        match self.manager.get_chunk(chunk_id).await? {
            Some(chunk) => Ok(json!({
                "chunk_id": chunk.id,
                "path": chunk.path,
                "source": chunk.source,
                "start_line": chunk.start_line,
                "end_line": chunk.end_line,
                "text": chunk.text,
            })),
            None => Ok(json!({
                "error": "chunk not found",
                "chunk_id": chunk_id,
            })),
        }
    }
}

/// Tool: save content to long-term memory files.
pub struct MemorySaveTool {
    manager: Arc<dyn MemoryRuntime>,
}

impl MemorySaveTool {
    pub fn new(manager: Arc<dyn MemoryRuntime>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl AgentTool for MemorySaveTool {
    fn name(&self) -> &str {
        "memory_save"
    }

    fn description(&self) -> &str {
        "Save content to long-term memory. Writes to MEMORY.md or memory/<name>.md. Content persists across sessions and is searchable via memory_search."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The content to save to memory"
                },
                "file": {
                    "type": "string",
                    "description": "Target file: MEMORY.md, memory.md, or memory/<name>.md",
                    "default": "MEMORY.md"
                },
                "append": {
                    "type": "boolean",
                    "description": "Append to existing file (true) or overwrite (false)",
                    "default": true
                }
            },
            "required": ["content"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let content = params["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'content' parameter"))?;
        let file = params["file"].as_str().unwrap_or("MEMORY.md");
        let append = params["append"].as_bool().unwrap_or(true);
        let checkpoint_id = checkpoint_memory_path(self.manager.as_ref(), file, "memory_save")
            .await?
            .id;

        let result = self.manager.write_memory(file, content, append).await?;

        Ok(json!({
            "saved": true,
            "path": file,
            "bytes_written": result.bytes_written,
            "checkpointId": result.checkpoint_id.or(Some(checkpoint_id)),
        }))
    }
}

/// Tool: delete or remove exact text from long-term memory files.
pub struct MemoryDeleteTool {
    manager: Arc<dyn MemoryRuntime>,
}

impl MemoryDeleteTool {
    pub fn new(manager: Arc<dyn MemoryRuntime>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl AgentTool for MemoryDeleteTool {
    fn name(&self) -> &str {
        "memory_delete"
    }

    fn description(&self) -> &str {
        "Forget saved memory by removing exact text from a memory file or deleting the file entirely. Updates the index immediately."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "file": {
                    "type": "string",
                    "description": "Target file: MEMORY.md, memory.md, or memory/<name>.md"
                },
                "text": {
                    "type": "string",
                    "description": "Exact text snippet to remove from the file. Required unless delete_file is true."
                },
                "delete_file": {
                    "type": "boolean",
                    "description": "Delete the whole file instead of removing exact text.",
                    "default": false
                },
                "all_matches": {
                    "type": "boolean",
                    "description": "Remove every exact match of text instead of only the first match.",
                    "default": false
                },
                "delete_if_empty": {
                    "type": "boolean",
                    "description": "Delete the file if removing text leaves only whitespace.",
                    "default": true
                }
            },
            "required": ["file"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let file = params["file"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'file' parameter"))?;
        let delete_file = params["delete_file"].as_bool().unwrap_or(false);
        let text = params["text"].as_str();
        let all_matches = params["all_matches"].as_bool().unwrap_or(false);
        let delete_if_empty = params["delete_if_empty"].as_bool().unwrap_or(true);

        if delete_file == text.is_some() {
            anyhow::bail!("provide either 'text' or delete_file=true");
        }

        let path = resolve_memory_tool_path(self.manager.as_ref(), file)?;
        let checkpoint_id = checkpoint_memory_path(self.manager.as_ref(), file, "memory_delete")
            .await?
            .id;

        if delete_file {
            let file_existed = tokio::fs::try_exists(&path).await?;
            if file_existed {
                tokio::fs::remove_file(&path).await?;
            }
            let index_removed = self.manager.remove_path(&path).await?;
            return Ok(json!({
                "deleted": true,
                "path": file,
                "file_deleted": file_existed,
                "file_existed": file_existed,
                "index_removed": index_removed,
                "bytes_written": 0,
                "checkpointId": checkpoint_id,
            }));
        }

        let existing = tokio::fs::read_to_string(&path).await.map_err(|error| {
            anyhow::anyhow!("failed to read memory file '{}': {error}", path.display())
        })?;
        let removal = remove_exact_text(&existing, text.unwrap_or_default(), all_matches)?;
        let file_deleted = delete_if_empty && removal.content.trim().is_empty();

        if file_deleted {
            tokio::fs::remove_file(&path).await?;
            self.manager.remove_path(&path).await?;
        } else {
            tokio::fs::write(&path, &removal.content).await?;
            self.manager.sync_path(&path).await?;
        }

        Ok(json!({
            "deleted": true,
            "path": file,
            "file_deleted": file_deleted,
            "matches_removed": removal.matches_removed,
            "bytes_written": if file_deleted { 0 } else { removal.content.len() },
            "checkpointId": checkpoint_id,
        }))
    }
}

fn resolve_memory_tool_path(manager: &dyn MemoryRuntime, file: &str) -> anyhow::Result<PathBuf> {
    let data_dir = manager
        .data_dir()
        .ok_or_else(|| anyhow::anyhow!("memory writes are disabled (no data_dir configured)"))?;
    Ok(validate_memory_path(data_dir, file)?)
}

async fn checkpoint_memory_path(
    manager: &dyn MemoryRuntime,
    file: &str,
    reason: &str,
) -> anyhow::Result<moltis_tools::checkpoints::CheckpointRecord> {
    let path = resolve_memory_tool_path(manager, file)?;
    let data_dir = manager
        .data_dir()
        .ok_or_else(|| anyhow::anyhow!("memory writes are disabled (no data_dir configured)"))?;
    let checkpoints = moltis_tools::checkpoints::CheckpointManager::new(data_dir.to_path_buf());
    checkpoints
        .checkpoint_path(&path, reason)
        .await
        .map_err(anyhow::Error::from)
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{
            config::MemoryConfig, embeddings::EmbeddingProvider, manager::MemoryManager,
            schema::run_migrations, store_sqlite::SqliteMemoryStore,
        },
        sqlx::SqlitePool,
        tempfile::TempDir,
    };

    /// Same keyword-based mock embedder used in manager tests.
    const KEYWORDS: [&str; 8] = [
        "rust", "python", "database", "memory", "search", "network", "cooking", "music",
    ];

    struct MockEmbedder;

    #[async_trait]
    impl EmbeddingProvider for MockEmbedder {
        async fn embed(&self, text: &str) -> crate::error::Result<Vec<f32>> {
            let lower = text.to_lowercase();
            Ok(KEYWORDS
                .iter()
                .map(|kw| {
                    if lower.contains(kw) {
                        1.0
                    } else {
                        0.0
                    }
                })
                .collect())
        }

        fn model_name(&self) -> &str {
            "mock-model"
        }

        fn dimensions(&self) -> usize {
            8
        }

        fn provider_key(&self) -> &str {
            "mock"
        }
    }

    /// Set up a memory manager in a temporary directory.
    ///
    /// Returns the Arc'd manager, the TempDir handle, and the data_dir path
    /// (which is `tmp.path()` — the root for MEMORY.md and memory/).
    async fn setup_manager() -> (Arc<MemoryManager>, TempDir) {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().to_path_buf();
        let mem_dir = data_dir.join("memory");
        std::fs::create_dir_all(&mem_dir).unwrap();

        let pool = SqlitePool::connect(":memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();

        let config = MemoryConfig {
            db_path: ":memory:".into(),
            data_dir: Some(data_dir),
            memory_dirs: vec![tmp.path().join("MEMORY.md"), mem_dir],
            chunk_size: 50,
            chunk_overlap: 10,
            vector_weight: 0.7,
            keyword_weight: 0.3,
            ..Default::default()
        };

        let store = Box::new(SqliteMemoryStore::new(pool));
        let embedder = Box::new(MockEmbedder);
        let manager = Arc::new(MemoryManager::new(config, store, embedder));
        (manager, tmp)
    }

    #[test]
    fn test_memory_search_tool_schema() {
        // Schema checks don't need a real manager — use a tokio runtime just to build one
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (manager, _tmp) = rt.block_on(setup_manager());
        let tool = MemorySearchTool::new(manager);
        assert_eq!(tool.name(), "memory_search");
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["query"].is_object());
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&json!("query"))
        );
    }

    #[test]
    fn test_memory_get_tool_schema() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (manager, _tmp) = rt.block_on(setup_manager());
        let tool = MemoryGetTool::new(manager);
        assert_eq!(tool.name(), "memory_get");
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["chunk_id"].is_object());
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&json!("chunk_id"))
        );
    }

    /// Execute memory_search via the tool interface and verify JSON output structure.
    #[tokio::test]
    async fn test_memory_search_tool_execute() {
        let (manager, tmp) = setup_manager().await;
        let mem_dir = tmp.path().join("memory");

        std::fs::write(
            mem_dir.join("note.md"),
            "Rust is a systems programming language with great memory safety.",
        )
        .unwrap();

        manager.sync().await.unwrap();

        let tool = MemorySearchTool::new(manager);
        let result = tool
            .execute(json!({ "query": "rust memory", "limit": 3 }))
            .await
            .unwrap();

        // Verify JSON structure
        let results = result["results"].as_array().unwrap();
        assert!(!results.is_empty(), "execute should return results");

        let first = &results[0];
        assert!(first["chunk_id"].is_string());
        assert!(first["path"].is_string());
        assert!(first["score"].is_f64());
        assert!(first["text"].is_string());
        assert!(first["start_line"].is_number());
        assert!(first["end_line"].is_number());

        // The text should contain what we wrote
        let text = first["text"].as_str().unwrap();
        assert!(
            text.contains("Rust"),
            "search result text should contain 'Rust', got: {text}"
        );
    }

    /// Execute memory_search with missing query — should return an error.
    #[tokio::test]
    async fn test_memory_search_tool_missing_query() {
        let (manager, _tmp) = setup_manager().await;
        let tool = MemorySearchTool::new(manager);
        let result = tool.execute(json!({})).await;
        assert!(result.is_err(), "missing query should produce an error");
    }

    /// Execute memory_get for an existing chunk.
    #[tokio::test]
    async fn test_memory_get_tool_execute() {
        let (manager, tmp) = setup_manager().await;
        let mem_dir = tmp.path().join("memory");

        std::fs::write(mem_dir.join("data.md"), "Some database content here.").unwrap();
        manager.sync().await.unwrap();

        // First search to find a chunk_id
        let search_tool = MemorySearchTool::new(manager.clone());
        let search_result = search_tool
            .execute(json!({ "query": "database", "limit": 1 }))
            .await
            .unwrap();
        let chunk_id = search_result["results"][0]["chunk_id"]
            .as_str()
            .unwrap()
            .to_string();

        // Now get that chunk
        let get_tool = MemoryGetTool::new(manager);
        let result = get_tool
            .execute(json!({ "chunk_id": chunk_id }))
            .await
            .unwrap();

        assert!(result["error"].is_null(), "should not have error");
        assert_eq!(result["chunk_id"].as_str().unwrap(), chunk_id);
        let text = result["text"].as_str().unwrap();
        assert!(
            text.contains("database"),
            "retrieved chunk should contain 'database', got: {text}"
        );
    }

    /// Execute memory_get for a non-existent chunk — should return error JSON (not a Rust error).
    #[tokio::test]
    async fn test_memory_get_tool_not_found() {
        let (manager, _tmp) = setup_manager().await;
        let tool = MemoryGetTool::new(manager);
        let result = tool
            .execute(json!({ "chunk_id": "nonexistent-id" }))
            .await
            .unwrap();

        assert_eq!(result["error"].as_str().unwrap(), "chunk not found");
        assert_eq!(result["chunk_id"].as_str().unwrap(), "nonexistent-id");
    }

    /// Execute memory_get with missing chunk_id — should return an error.
    #[tokio::test]
    async fn test_memory_get_tool_missing_param() {
        let (manager, _tmp) = setup_manager().await;
        let tool = MemoryGetTool::new(manager);
        let result = tool.execute(json!({})).await;
        assert!(result.is_err(), "missing chunk_id should produce an error");
    }

    /// Round-trip: sync → search via tool → get via tool → verify text matches.
    #[tokio::test]
    async fn test_tools_round_trip() {
        let (manager, tmp) = setup_manager().await;
        let mem_dir = tmp.path().join("memory");

        let original_text = "Cooking pasta with fresh herbs and olive oil is a delight.";
        std::fs::write(mem_dir.join("recipe.md"), original_text).unwrap();
        manager.sync().await.unwrap();

        let search_tool = MemorySearchTool::new(manager.clone());
        let get_tool = MemoryGetTool::new(manager.clone());

        // Search
        let search_result = search_tool
            .execute(json!({ "query": "cooking", "limit": 1 }))
            .await
            .unwrap();
        let results = search_result["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        let chunk_id = results[0]["chunk_id"].as_str().unwrap();

        // Get
        let get_result = get_tool
            .execute(json!({ "chunk_id": chunk_id }))
            .await
            .unwrap();
        let retrieved_text = get_result["text"].as_str().unwrap();

        assert_eq!(
            retrieved_text, original_text,
            "round-trip text should match original"
        );
    }

    // ---- MemorySaveTool tests ----

    #[test]
    fn test_memory_save_tool_schema() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (manager, _tmp) = rt.block_on(setup_manager());
        let tool = MemorySaveTool::new(manager);
        assert_eq!(tool.name(), "memory_save");
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["content"].is_object());
        assert!(schema["properties"]["file"].is_object());
        assert!(schema["properties"]["append"].is_object());
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&json!("content"))
        );
    }

    #[test]
    fn test_memory_delete_tool_schema() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (manager, _tmp) = rt.block_on(setup_manager());
        let tool = MemoryDeleteTool::new(manager);
        assert_eq!(tool.name(), "memory_delete");
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["file"].is_object());
        assert!(schema["properties"]["text"].is_object());
        assert!(schema["properties"]["delete_file"].is_object());
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&json!("file"))
        );
    }

    /// Default append mode: two writes produce both contents in the file.
    #[tokio::test]
    async fn test_memory_save_append_default() {
        let (manager, tmp) = setup_manager().await;
        let data_dir = tmp.path().to_path_buf();
        let tool = MemorySaveTool::new(manager.clone());

        let r1 = tool
            .execute(json!({ "content": "First memory about rust." }))
            .await
            .unwrap();
        assert_eq!(r1["saved"], json!(true));
        assert_eq!(r1["path"], json!("MEMORY.md"));
        assert!(r1["checkpointId"].is_string());

        let r2 = tool
            .execute(json!({ "content": "Second memory about database." }))
            .await
            .unwrap();
        assert_eq!(r2["saved"], json!(true));
        assert!(r2["checkpointId"].is_string());

        let content = std::fs::read_to_string(data_dir.join("MEMORY.md")).unwrap();
        assert!(content.contains("First memory"), "should have first write");
        assert!(
            content.contains("Second memory"),
            "should have second write"
        );
    }

    /// Overwrite mode: second write replaces the first.
    #[tokio::test]
    async fn test_memory_save_overwrite() {
        let (manager, tmp) = setup_manager().await;
        let data_dir = tmp.path().to_path_buf();
        let tool = MemorySaveTool::new(manager.clone());

        tool.execute(json!({ "content": "Original content about rust." }))
            .await
            .unwrap();

        tool.execute(json!({
            "content": "Replaced content about database.",
            "append": false
        }))
        .await
        .unwrap();

        let content = std::fs::read_to_string(data_dir.join("MEMORY.md")).unwrap();
        assert!(
            !content.contains("Original"),
            "overwrite should remove original"
        );
        assert!(content.contains("Replaced"), "overwrite should have new");
    }

    /// Custom file under memory/ subdirectory.
    #[tokio::test]
    async fn test_memory_save_custom_file() {
        let (manager, tmp) = setup_manager().await;
        let data_dir = tmp.path().to_path_buf();
        let tool = MemorySaveTool::new(manager.clone());

        let result = tool
            .execute(json!({
                "content": "Notes from 2024-01-15 about cooking.",
                "file": "memory/2024-01-15.md"
            }))
            .await
            .unwrap();

        assert_eq!(result["saved"], json!(true));
        assert_eq!(result["path"], json!("memory/2024-01-15.md"));

        let content =
            std::fs::read_to_string(data_dir.join("memory").join("2024-01-15.md")).unwrap();
        assert!(content.contains("Notes from 2024-01-15"));
    }

    /// Auto-creates memory/ directory if it doesn't exist.
    #[tokio::test]
    async fn test_memory_save_creates_memory_dir() {
        let (manager, tmp) = setup_manager().await;
        let data_dir = tmp.path().to_path_buf();
        // Remove the memory dir that setup_manager created
        std::fs::remove_dir_all(data_dir.join("memory")).unwrap();
        assert!(!data_dir.join("memory").exists());

        let tool = MemorySaveTool::new(manager.clone());
        tool.execute(json!({
            "content": "Content for new dir.",
            "file": "memory/notes.md"
        }))
        .await
        .unwrap();

        assert!(data_dir.join("memory").join("notes.md").exists());
    }

    /// Re-indexes after write so content is immediately searchable.
    #[tokio::test]
    async fn test_memory_save_reindexes() {
        let (manager, _tmp) = setup_manager().await;
        let save_tool = MemorySaveTool::new(manager.clone());
        let search_tool = MemorySearchTool::new(manager.clone());

        save_tool
            .execute(json!({
                "content": "The cooking recipe uses garlic and olive oil.",
                "file": "memory/recipe.md"
            }))
            .await
            .unwrap();

        let results = search_tool
            .execute(json!({ "query": "cooking", "limit": 5 }))
            .await
            .unwrap();

        let items = results["results"].as_array().unwrap();
        assert!(!items.is_empty(), "saved content should be searchable");
        assert!(
            items[0]["text"].as_str().unwrap().contains("cooking"),
            "search should find the saved text"
        );
    }

    /// Path traversal attempts are rejected.
    #[tokio::test]
    async fn test_memory_save_rejects_path_traversal() {
        let (manager, _tmp) = setup_manager().await;
        let tool = MemorySaveTool::new(manager.clone());

        for bad_path in &[
            "../etc/passwd",
            "memory/../../../etc/passwd",
            "memory/../../secret.md",
        ] {
            let result = tool
                .execute(json!({ "content": "test", "file": bad_path }))
                .await;
            assert!(result.is_err(), "should reject path traversal: {bad_path}");
        }
    }

    /// Absolute paths are rejected.
    #[tokio::test]
    async fn test_memory_save_rejects_absolute_paths() {
        let (manager, _tmp) = setup_manager().await;
        let tool = MemorySaveTool::new(manager.clone());

        let result = tool
            .execute(json!({ "content": "test", "file": "/etc/passwd" }))
            .await;
        assert!(result.is_err(), "should reject absolute paths");
    }

    /// Invalid file names are rejected.
    #[tokio::test]
    async fn test_memory_save_rejects_invalid_names() {
        let (manager, _tmp) = setup_manager().await;
        let tool = MemorySaveTool::new(manager.clone());

        let invalid = &[
            "memory/notes.txt",     // wrong extension
            "memory/.md",           // empty stem
            "memory/a b c.md",      // spaces in name
            "memory/sub/nested.md", // nested subdirectory
            "random.md",            // not MEMORY.md or memory/
            "foo/bar.md",           // not in allowed paths
        ];

        for name in invalid {
            let result = tool
                .execute(json!({ "content": "test", "file": name }))
                .await;
            assert!(result.is_err(), "should reject invalid name: {name}");
        }
    }

    /// Missing content parameter returns an error.
    #[tokio::test]
    async fn test_memory_save_missing_content() {
        let (manager, _tmp) = setup_manager().await;
        let tool = MemorySaveTool::new(manager.clone());

        let result = tool.execute(json!({})).await;
        assert!(result.is_err(), "missing content should produce an error");
    }

    /// Content exceeding the size limit is rejected.
    #[tokio::test]
    async fn test_memory_save_content_size_limit() {
        let (manager, _tmp) = setup_manager().await;
        let tool = MemorySaveTool::new(manager.clone());

        // 50 KB limit is enforced by MemoryManager's MemoryWriter impl
        let big = "x".repeat(50 * 1024 + 1);
        let result = tool.execute(json!({ "content": big })).await;
        assert!(result.is_err(), "oversized content should be rejected");

        let at_limit = "x".repeat(50 * 1024);
        let result = tool.execute(json!({ "content": at_limit })).await;
        assert!(result.is_ok(), "content at limit should succeed");
    }

    /// Full round-trip: save → search → get → verify text matches.
    #[tokio::test]
    async fn test_memory_save_round_trip() {
        let (manager, _tmp) = setup_manager().await;
        let save_tool = MemorySaveTool::new(manager.clone());
        let search_tool = MemorySearchTool::new(manager.clone());
        let get_tool = MemoryGetTool::new(manager.clone());

        let text = "Music from the jazz era is deeply expressive and soulful.";
        save_tool
            .execute(json!({ "content": text, "file": "memory/jazz.md" }))
            .await
            .unwrap();

        // Search
        let search_result = search_tool
            .execute(json!({ "query": "music", "limit": 1 }))
            .await
            .unwrap();
        let results = search_result["results"].as_array().unwrap();
        assert!(!results.is_empty(), "saved content should be searchable");
        let chunk_id = results[0]["chunk_id"].as_str().unwrap();

        // Get
        let get_result = get_tool
            .execute(json!({ "chunk_id": chunk_id }))
            .await
            .unwrap();
        let retrieved = get_result["text"].as_str().unwrap();
        assert!(
            retrieved.contains("jazz era"),
            "round-trip text should contain saved content, got: {retrieved}"
        );
    }

    #[tokio::test]
    async fn test_memory_delete_removes_exact_text_and_reindexes() {
        let (manager, tmp) = setup_manager().await;
        let data_dir = tmp.path().to_path_buf();
        std::fs::write(
            data_dir.join("MEMORY.md"),
            "User likes spicy food.\nUser likes board games.\n",
        )
        .unwrap();
        manager.sync().await.unwrap();

        let delete_tool = MemoryDeleteTool::new(manager.clone());
        let search_tool = MemorySearchTool::new(manager.clone());
        let result = delete_tool
            .execute(json!({
                "file": "MEMORY.md",
                "text": "User likes spicy food.\n",
            }))
            .await
            .unwrap();

        assert_eq!(result["deleted"], json!(true));
        assert_eq!(result["matches_removed"], json!(1));
        assert_eq!(result["file_deleted"], json!(false));
        assert!(result["checkpointId"].is_string());

        let updated = std::fs::read_to_string(data_dir.join("MEMORY.md")).unwrap();
        assert!(!updated.contains("spicy food"));
        assert!(updated.contains("board games"));

        let search = search_tool
            .execute(json!({ "query": "spicy food", "limit": 5 }))
            .await
            .unwrap();
        let items = search["results"].as_array().unwrap();
        assert!(
            items.iter().all(|item| !item["text"]
                .as_str()
                .unwrap_or_default()
                .contains("spicy food")),
            "removed memory should not remain searchable"
        );
    }

    #[tokio::test]
    async fn test_memory_delete_deletes_file_when_requested() {
        let (manager, tmp) = setup_manager().await;
        let data_dir = tmp.path().to_path_buf();
        std::fs::write(data_dir.join("memory").join("notes.md"), "temporary note").unwrap();
        manager.sync().await.unwrap();

        let delete_tool = MemoryDeleteTool::new(manager.clone());
        let result = delete_tool
            .execute(json!({
                "file": "memory/notes.md",
                "delete_file": true,
            }))
            .await
            .unwrap();

        assert_eq!(result["deleted"], json!(true));
        assert_eq!(result["file_deleted"], json!(true));
        assert!(result["checkpointId"].is_string());
        assert!(!data_dir.join("memory").join("notes.md").exists());
        let search = manager.search("temporary note", 5).await.unwrap();
        assert!(
            search.is_empty(),
            "deleted file should be removed from search results"
        );
    }

    #[tokio::test]
    async fn test_memory_delete_rejects_ambiguous_request() {
        let (manager, _tmp) = setup_manager().await;
        let tool = MemoryDeleteTool::new(manager);
        let result = tool
            .execute(json!({
                "file": "MEMORY.md",
                "text": "hello",
                "delete_file": true,
            }))
            .await;
        assert!(
            result.is_err(),
            "request should choose exactly one delete mode"
        );
    }
}
