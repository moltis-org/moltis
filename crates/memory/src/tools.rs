/// Agent tools for memory search and retrieval.
use std::sync::Arc;

use {async_trait::async_trait, moltis_agents::tool_registry::AgentTool, serde_json::json};

use crate::manager::MemoryManager;

/// Tool: search memory with a natural language query.
pub struct MemorySearchTool {
    manager: Arc<MemoryManager>,
}

impl MemorySearchTool {
    pub fn new(manager: Arc<MemoryManager>) -> Self {
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
    manager: Arc<MemoryManager>,
}

impl MemoryGetTool {
    pub fn new(manager: Arc<MemoryManager>) -> Self {
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

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{
            config::MemoryConfig, embeddings::EmbeddingProvider, schema::run_migrations,
            store_sqlite::SqliteMemoryStore,
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
        async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
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

    async fn setup_manager() -> (Arc<MemoryManager>, TempDir) {
        let tmp = TempDir::new().unwrap();
        let mem_dir = tmp.path().join("memory");
        std::fs::create_dir_all(&mem_dir).unwrap();

        let pool = SqlitePool::connect(":memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();

        let config = MemoryConfig {
            db_path: ":memory:".into(),
            memory_dirs: vec![mem_dir],
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
        let search_tool = MemorySearchTool::new(Arc::clone(&manager));
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

        let search_tool = MemorySearchTool::new(Arc::clone(&manager));
        let get_tool = MemoryGetTool::new(Arc::clone(&manager));

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
}
