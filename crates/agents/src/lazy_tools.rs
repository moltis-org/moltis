use {
    anyhow::Result,
    async_trait::async_trait,
    std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    },
    tracing::info,
};

use crate::tool_registry::{AgentTool, ToolRegistry};

// ── ToolSearchTool ──────────────────────────────────────────────────────────

/// Discovers tools from the full registry without loading schemas into the prompt.
///
/// Keyword search (`{ "query": "..." }`) returns name + description only (no
/// parameter schemas) for up to 15 results sorted by relevance.
///
/// Exact name lookup (`{ "name": "exec" }`) returns the full parameter schema
/// **and activates the tool** in the lazy registry so the model can call it
/// directly on the next turn without going through any proxy.
pub struct ToolSearchTool {
    /// The full tool registry to search through.
    full_registry: Arc<ToolRegistry>,
    /// Shared with the lazy `ToolRegistry` wrapper. On exact-name lookup, the
    /// found tool is inserted here so the lazy registry exposes it natively.
    activated: Arc<Mutex<HashMap<String, Arc<dyn AgentTool>>>>,
}

impl ToolSearchTool {
    pub fn new(
        full_registry: Arc<ToolRegistry>,
        activated: Arc<Mutex<HashMap<String, Arc<dyn AgentTool>>>>,
    ) -> Self {
        Self { full_registry, activated }
    }
}

#[async_trait]
impl AgentTool for ToolSearchTool {
    fn name(&self) -> &str {
        "tool_search"
    }

    fn description(&self) -> &str {
        "Search available tools by keyword or get full parameter schema by exact name."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keyword to match against tool names and descriptions"
                },
                "name": {
                    "type": "string",
                    "description": "Exact tool name to retrieve its full parameter schema"
                }
            }
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        // Exact name lookup: return full schema and activate the tool so the
        // model can call it directly on the next runner iteration.
        if let Some(name) = params.get("name").and_then(|v| v.as_str()) {
            return match self.full_registry.get_arc(name) {
                Some(tool) => {
                    // Activate: add the tool to the shared activated map so the
                    // lazy registry exposes it natively on the next LLM turn.
                    self.activated
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(name.to_string(), Arc::clone(&tool));
                    info!(tool = %name, "lazy: tool activated for direct call");
                    Ok(serde_json::json!({
                        "name": tool.name(),
                        "description": tool.description(),
                        "parameters": tool.parameters_schema(),
                        "hint": "Tool activated. Call it directly by name on the next turn.",
                    }))
                },
                None => Ok(serde_json::json!({ "error": format!("unknown tool: {name}") })),
            };
        }

        // Keyword search: name + description only, no schemas.
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();

        if query.len() < 2 {
            return Ok(serde_json::json!({
                "error": "query too short, provide at least 2 characters"
            }));
        }

        let mut scored: Vec<(usize, serde_json::Value)> = Vec::new();

        for tool_name in self.full_registry.list_names() {
            let Some(tool) = self.full_registry.get(&tool_name) else {
                continue;
            };
            let n = tool.name().to_lowercase();
            let d = tool.description().to_lowercase();

            let score = if n == query {
                100
            } else if n.contains(&query) {
                50
            } else if query
                .split_whitespace()
                .any(|w| n.contains(w) || d.contains(w))
            {
                10
            } else {
                0
            };

            if score > 0 {
                scored.push((
                    score,
                    serde_json::json!({
                        "name": tool.name(),
                        "description": tool.description(),
                    }),
                ));
            }
        }

        scored.sort_by(|a, b| b.0.cmp(&a.0));
        let total = scored.len();
        let limit = 15;
        let tools: Vec<serde_json::Value> =
            scored.into_iter().take(limit).map(|(_, v)| v).collect();

        Ok(serde_json::json!({
            "total": total,
            "returned": tools.len(),
            "tools": tools,
        }))
    }
}

// ── Public constructors ─────────────────────────────────────────────────────

/// Wrap a fully-filtered `ToolRegistry` behind a single `tool_search` meta-tool.
///
/// The returned registry contains only `tool_search`. It holds an `Arc` to the
/// original registry so all real tools remain searchable. When the model calls
/// `tool_search` with an exact tool name, that tool is **activated**: added to
/// the lazy registry's `activated` map so it appears natively in the tool list
/// on the next runner iteration. The model can then call it directly.
///
/// This eliminates the need for a `tool_use` proxy — real tool calls go through
/// the runner's normal dispatch path, ensuring hook enforcement without any
/// double-dispatch complexity.
pub fn wrap_registry_lazy(full: ToolRegistry) -> ToolRegistry {
    let full_arc = Arc::new(full);
    let mut lazy = ToolRegistry::new();
    // Share the lazy registry's activated map with ToolSearchTool so search
    // results immediately appear in lazy.list_schemas() and lazy.get_arc().
    let activated = Arc::clone(&lazy.activated);
    lazy.register(Box::new(ToolSearchTool::new(full_arc, activated)));
    lazy
}



// ── Tests ───────────────────────────────────────────────────────────────────

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    struct StubTool {
        tool_name: String,
        tool_desc: String,
    }

    #[async_trait]
    impl AgentTool for StubTool {
        fn name(&self) -> &str {
            &self.tool_name
        }

        fn description(&self) -> &str {
            &self.tool_desc
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "value": { "type": "string" }
                }
            })
        }

        async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value> {
            Ok(serde_json::json!({
                "tool": self.tool_name,
                "received": params,
            }))
        }
    }

    fn make_stub(name: &str, desc: &str) -> Box<dyn AgentTool> {
        Box::new(StubTool {
            tool_name: name.to_string(),
            tool_desc: desc.to_string(),
        })
    }

    fn sample_registry() -> ToolRegistry {
        let mut reg = ToolRegistry::new();
        reg.register(make_stub("exec", "Run shell commands in sandbox"));
        reg.register(make_stub("web_search", "Search the web"));
        reg.register(make_stub("web_fetch", "Fetch a URL"));
        reg.register(make_stub("memory_search", "Search long-term memory"));
        reg.register(make_stub("memory_save", "Save to long-term memory"));
        reg.register(make_stub(
            "mcp__github__create_issue",
            "Create a GitHub issue",
        ));
        reg
    }

    // ── wrap_registry_lazy ──────────────────────────────────────────────

    #[test]
    fn wrap_registry_lazy_produces_one_tool() {
        let full = sample_registry();
        let lazy = wrap_registry_lazy(full);
        let names = lazy.list_names();
        assert_eq!(names.len(), 1);
        assert!(names.contains(&"tool_search".to_string()));
    }

    // ── tool_search: keyword mode ───────────────────────────────────────

    #[tokio::test]
    async fn search_keyword_returns_name_and_description_only() {
        let full = sample_registry();
        let search =
            ToolSearchTool::new(Arc::new(full), Arc::new(Mutex::new(HashMap::new())));
        let result = search
            .execute(serde_json::json!({ "query": "memory" }))
            .await
            .unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 2);
        for tool in tools {
            assert!(tool.get("name").is_some());
            assert!(tool.get("description").is_some());
            assert!(
                tool.get("parameters").is_none(),
                "keyword search must not leak schemas"
            );
        }
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(names.contains(&"memory_search"));
        assert!(names.contains(&"memory_save"));
    }

    #[tokio::test]
    async fn search_scores_exact_match_highest() {
        let full = sample_registry();
        let search =
            ToolSearchTool::new(Arc::new(full), Arc::new(Mutex::new(HashMap::new())));
        let result = search
            .execute(serde_json::json!({ "query": "exec" }))
            .await
            .unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert_eq!(tools[0]["name"].as_str(), Some("exec"));
    }

    #[tokio::test]
    async fn search_multiword_query() {
        let full = sample_registry();
        let search =
            ToolSearchTool::new(Arc::new(full), Arc::new(Mutex::new(HashMap::new())));
        let result = search
            .execute(serde_json::json!({ "query": "github issue" }))
            .await
            .unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert!(
            tools
                .iter()
                .any(|t| t["name"].as_str() == Some("mcp__github__create_issue"))
        );
    }

    #[tokio::test]
    async fn search_short_query_rejected() {
        let full = sample_registry();
        let search =
            ToolSearchTool::new(Arc::new(full), Arc::new(Mutex::new(HashMap::new())));
        let result = search
            .execute(serde_json::json!({ "query": "x" }))
            .await
            .unwrap();
        assert!(result.get("error").is_some());
    }

    // ── tool_search: exact name mode ────────────────────────────────────

    #[tokio::test]
    async fn search_exact_name_returns_full_schema() {
        let full = sample_registry();
        let search =
            ToolSearchTool::new(Arc::new(full), Arc::new(Mutex::new(HashMap::new())));
        let result = search
            .execute(serde_json::json!({ "name": "exec" }))
            .await
            .unwrap();
        assert_eq!(result["name"].as_str(), Some("exec"));
        assert!(result.get("parameters").is_some());
        assert!(result["parameters"].get("properties").is_some());
    }

    #[tokio::test]
    async fn search_exact_name_activates_tool() {
        let full = sample_registry();
        let activated = Arc::new(Mutex::new(HashMap::new()));
        let search = ToolSearchTool::new(Arc::new(full), Arc::clone(&activated));
        let _ = search
            .execute(serde_json::json!({ "name": "exec" }))
            .await
            .unwrap();
        let map = activated.lock().unwrap();
        assert!(map.contains_key("exec"), "exec should be in the activated map");
    }

    #[tokio::test]
    async fn search_unknown_name() {
        let full = sample_registry();
        let search =
            ToolSearchTool::new(Arc::new(full), Arc::new(Mutex::new(HashMap::new())));
        let result = search
            .execute(serde_json::json!({ "name": "nonexistent" }))
            .await
            .unwrap();
        assert!(result.get("error").is_some());
    }

    // ── tool_search: context size safety ────────────────────────────────

    #[tokio::test]
    async fn search_result_stays_compact_with_many_tools() {
        let mut reg = ToolRegistry::new();
        for i in 0..200 {
            reg.register(make_stub(
                &format!("mcp__server{i}__action"),
                &format!("Perform action on server {i} with complex parameters"),
            ));
        }

        let search =
            ToolSearchTool::new(Arc::new(reg), Arc::new(Mutex::new(HashMap::new())));
        let result = search
            .execute(serde_json::json!({ "query": "action" }))
            .await
            .unwrap();

        let returned = result["returned"].as_u64().unwrap();
        assert!(returned <= 15, "returned {returned} tools, expected <= 15");
        assert_eq!(result["total"].as_u64().unwrap(), 200);

        let tools = result["tools"].as_array().unwrap();
        for tool in tools {
            assert!(tool.get("parameters").is_none());
        }

        // Rough token estimate: 1 token ≈ 4 chars.
        let json_str = serde_json::to_string(&result).unwrap();
        let estimated_tokens = json_str.len() / 4;
        assert!(
            estimated_tokens < 600,
            "search result is ~{estimated_tokens} tokens, must be under 600"
        );
    }
}
