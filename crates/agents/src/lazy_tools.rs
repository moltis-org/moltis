//! Lazy tool registry: model discovers tools via `tool_search` instead of
//! receiving all schemas upfront.
//!
//! When `registry_mode = "lazy"` is set in config, [`wrap_registry_lazy`]
//! replaces the full registry with one containing only `tool_search`.
//! The model calls `tool_search(query="…")` to find tools and
//! `tool_search(name="tool_name")` to activate them (get the full schema).
//! Activated tools appear in subsequent iterations via the runner's
//! per-iteration `list_schemas()` call.

use std::sync::Arc;

use {anyhow::Result, async_trait::async_trait, tracing::debug};

use crate::tool_registry::{ActivatedTools, AgentTool, ToolEntry, ToolRegistry, ToolSource};

/// Maximum number of results returned by a keyword search.
const MAX_SEARCH_RESULTS: usize = 15;

/// Meta-tool that lets the model discover and activate tools from the full registry.
pub struct ToolSearchTool {
    /// The original full registry (read-only).
    full_registry: Arc<ToolRegistry>,
    /// Shared activated set — same Arc as the wrapper registry's `activated` field.
    activated: ActivatedTools,
}

impl ToolSearchTool {
    fn keyword_search(&self, query: &str) -> Vec<(String, String, u32)> {
        let query_lower = query.to_lowercase();
        let query_words: Vec<&str> = query_lower.split_whitespace().collect();

        let mut results: Vec<(String, String, u32)> = Vec::new();

        for name in self.full_registry.list_names() {
            let name_lower = name.to_lowercase();
            if let Some(tool) = self.full_registry.get(&name) {
                let desc = tool.description().to_string();
                let desc_lower = desc.to_lowercase();

                let score = if name_lower == query_lower {
                    100
                } else if name_lower.contains(&query_lower) {
                    50
                } else {
                    let word_matches = query_words
                        .iter()
                        .filter(|w| name_lower.contains(*w) || desc_lower.contains(*w))
                        .count();
                    if word_matches > 0 {
                        (word_matches as u32) * 10
                    } else {
                        0
                    }
                };

                if score > 0 {
                    results.push((name, desc, score));
                }
            }
        }

        results.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
        results.truncate(MAX_SEARCH_RESULTS);
        results
    }

    fn activate_tool(&self, name: &str) -> serde_json::Value {
        let Some(tool) = self.full_registry.get(name) else {
            return self.unknown_tool_response(name);
        };
        let source = self
            .full_registry
            .get_source(name)
            .unwrap_or(ToolSource::Builtin);

        let schema = tool.parameters_schema();
        let description = tool.description().to_string();

        // Insert into activated map, preserving the original source metadata.
        let mut activated = self.activated.lock().unwrap_or_else(|e| e.into_inner());
        activated.insert(name.to_string(), ToolEntry { tool, source });

        debug!(tool = name, "tool activated via tool_search");

        serde_json::json!({
            "activated": true,
            "name": name,
            "description": description,
            "parameters": schema,
            "hint": format!("Tool `{name}` is now available. Call it directly on your next turn.")
        })
    }

    fn unknown_tool_response(&self, name: &str) -> serde_json::Value {
        let suggestions: Vec<serde_json::Value> = self
            .keyword_search(name)
            .into_iter()
            .map(|(name, desc, _score)| {
                serde_json::json!({
                    "name": name,
                    "description": desc
                })
            })
            .collect();
        let mcp_servers: Vec<String> = self
            .full_registry
            .list_schemas()
            .into_iter()
            .filter_map(|schema| {
                (schema.get("source").and_then(serde_json::Value::as_str) == Some("mcp"))
                    .then(|| schema.get("mcpServer")?.as_str().map(str::to_string))
                    .flatten()
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();

        serde_json::json!({
            "activated": false,
            "name": name,
            "error": format!("unknown tool: {name}"),
            "suggestions": suggestions,
            "mcpServers": mcp_servers,
            "hint": "The `name` field must be an exact tool name from search results or Available Tools. Skills are not tools. For connected MCP servers, search with query set to the server or domain name, then activate the exact `mcp__<server>__<tool>` name."
        })
    }
}

#[async_trait]
impl AgentTool for ToolSearchTool {
    fn name(&self) -> &str {
        "tool_search"
    }

    fn description(&self) -> &str {
        "Search for available tools by keyword, or activate a specific tool by exact name. \
         Use `query` to find tools (returns name + description, max 15 results). \
         Use `name` to activate a tool and get its full parameter schema."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keyword to search tool names and descriptions"
                },
                "name": {
                    "type": "string",
                    "description": "Exact tool name to activate (returns full schema)"
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        let name = params.get("name").and_then(|v| v.as_str());
        let query = params.get("query").and_then(|v| v.as_str());

        match (name, query) {
            (Some(name), _) => {
                // Activate a specific tool by name.
                Ok(self.activate_tool(name))
            },
            (None, Some(query)) => {
                // Keyword search.
                let results = self.keyword_search(query);
                let items: Vec<serde_json::Value> = results
                    .into_iter()
                    .map(|(name, desc, _score)| {
                        serde_json::json!({
                            "name": name,
                            "description": desc
                        })
                    })
                    .collect();
                Ok(serde_json::json!({
                    "results": items,
                    "hint": "To use a tool, call tool_search again with `name` set to the exact tool name."
                }))
            },
            (None, None) => Err(anyhow::anyhow!(
                "Provide either `name` (to activate a tool) or `query` (to search)."
            )),
        }
    }
}

/// Wrap a full tool registry for lazy mode.
///
/// Returns a new registry containing only `tool_search`. The model discovers
/// and activates tools from `full` via that meta-tool. Activated tools
/// appear in `list_schemas()` on the next runner iteration.
pub fn wrap_registry_lazy(full: ToolRegistry) -> ToolRegistry {
    let full = Arc::new(full);
    let mut lazy_registry = ToolRegistry::new();

    // Share the lazy registry's activated set with ToolSearchTool.
    let activated = Arc::clone(&lazy_registry.activated);

    let search_tool = ToolSearchTool {
        full_registry: full,
        activated,
    };
    lazy_registry.register(Box::new(search_tool));
    lazy_registry
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    struct DummyTool {
        tool_name: String,
        tool_desc: String,
    }

    impl DummyTool {
        fn new(name: &str, desc: &str) -> Self {
            Self {
                tool_name: name.to_string(),
                tool_desc: desc.to_string(),
            }
        }
    }

    #[async_trait]
    impl AgentTool for DummyTool {
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
                    "input": { "type": "string" }
                }
            })
        }

        async fn execute(&self, _params: serde_json::Value) -> Result<serde_json::Value> {
            Ok(serde_json::json!({ "ok": true }))
        }
    }

    fn build_full_registry() -> ToolRegistry {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool::new("exec", "Execute a shell command")));
        registry.register(Box::new(DummyTool::new(
            "web_fetch",
            "Fetch a URL and return its content",
        )));
        registry.register(Box::new(DummyTool::new(
            "memory_search",
            "Search long-term memory for relevant information",
        )));
        registry.register(Box::new(DummyTool::new(
            "memory_save",
            "Save information to long-term memory",
        )));
        registry.register(Box::new(DummyTool::new(
            "memory_forget",
            "Forget information from long-term memory using natural language",
        )));
        registry.register(Box::new(DummyTool::new(
            "memory_delete",
            "Delete information from long-term memory",
        )));
        registry.register(Box::new(DummyTool::new(
            "browser_navigate",
            "Navigate browser to a URL",
        )));
        registry.register_mcp(
            Box::new(DummyTool::new(
                "mcp__vmcp-ha__homeassistant_ha_get_state",
                "Get current status, state, and attributes from Home Assistant",
            )),
            "vmcp-ha".to_string(),
        );
        registry
    }

    #[test]
    fn wrap_registry_lazy_contains_only_tool_search() {
        let full = build_full_registry();
        assert_eq!(full.list_names().len(), 8);

        let lazy = wrap_registry_lazy(full);
        let names = lazy.list_names();
        assert_eq!(names.len(), 1);
        assert!(names.contains(&"tool_search".to_string()));
    }

    #[tokio::test]
    async fn keyword_search_returns_matching_tools() {
        let full = build_full_registry();
        let lazy = wrap_registry_lazy(full);
        let search_tool = lazy.get("tool_search").unwrap();

        let result = search_tool
            .execute(serde_json::json!({ "query": "memory" }))
            .await
            .unwrap();

        let results = result["results"].as_array().unwrap();
        assert_eq!(results.len(), 4);
        let names: Vec<&str> = results
            .iter()
            .map(|r| r["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"memory_search"));
        assert!(names.contains(&"memory_save"));
        assert!(names.contains(&"memory_forget"));
        assert!(names.contains(&"memory_delete"));
    }

    #[tokio::test]
    async fn keyword_search_by_description() {
        let full = build_full_registry();
        let lazy = wrap_registry_lazy(full);
        let search_tool = lazy.get("tool_search").unwrap();

        let result = search_tool
            .execute(serde_json::json!({ "query": "shell" }))
            .await
            .unwrap();

        let results = result["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["name"], "exec");
    }

    #[tokio::test]
    async fn activate_tool_adds_to_registry() {
        let full = build_full_registry();
        let lazy = wrap_registry_lazy(full);

        // Before activation, only tool_search is visible.
        assert_eq!(lazy.list_schemas().len(), 1);
        assert!(lazy.get("exec").is_none());

        let search_tool = lazy.get("tool_search").unwrap();
        let result = search_tool
            .execute(serde_json::json!({ "name": "exec" }))
            .await
            .unwrap();

        assert_eq!(result["activated"], true);
        assert_eq!(result["name"], "exec");
        assert!(result["parameters"].is_object());

        // After activation, exec is visible.
        assert_eq!(lazy.list_schemas().len(), 2);
        assert!(lazy.get("exec").is_some());
    }

    #[tokio::test]
    async fn activate_unknown_tool_returns_recovery_response() {
        let full = build_full_registry();
        let lazy = wrap_registry_lazy(full);
        let search_tool = lazy.get("tool_search").unwrap();

        let result = search_tool
            .execute(serde_json::json!({ "name": "nonexistent" }))
            .await
            .unwrap();

        assert_eq!(result["activated"], false);
        assert_eq!(result["name"], "nonexistent");
        assert_eq!(result["error"], "unknown tool: nonexistent");
        assert!(
            result["hint"]
                .as_str()
                .unwrap()
                .contains("Skills are not tools")
        );
    }

    #[tokio::test]
    async fn activate_unknown_tool_returns_search_suggestions() {
        let full = build_full_registry();
        let lazy = wrap_registry_lazy(full);
        let search_tool = lazy.get("tool_search").unwrap();

        let result = search_tool
            .execute(serde_json::json!({ "name": "memory" }))
            .await
            .unwrap();

        let suggestions = result["suggestions"].as_array().unwrap();
        let names: Vec<&str> = suggestions
            .iter()
            .map(|r| r["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"memory_search"));
        assert!(names.contains(&"memory_save"));
    }

    #[tokio::test]
    async fn activate_skill_name_gives_mcp_recovery_hint() {
        let full = build_full_registry();
        let lazy = wrap_registry_lazy(full);
        let search_tool = lazy.get("tool_search").unwrap();

        let result = search_tool
            .execute(serde_json::json!({ "name": "mcporter", "query": "null" }))
            .await
            .unwrap();

        assert_eq!(result["activated"], false);
        assert_eq!(result["error"], "unknown tool: mcporter");
        assert_eq!(result["mcpServers"], serde_json::json!(["vmcp-ha"]));
        assert!(
            result["hint"]
                .as_str()
                .unwrap()
                .contains("mcp__<server>__<tool>")
        );
    }

    #[tokio::test]
    async fn search_finds_homeassistant_mcp_tools() {
        let full = build_full_registry();
        let lazy = wrap_registry_lazy(full);
        let search_tool = lazy.get("tool_search").unwrap();

        let result = search_tool
            .execute(serde_json::json!({ "query": "homeassistant" }))
            .await
            .unwrap();

        let names: Vec<&str> = result["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"mcp__vmcp-ha__homeassistant_ha_get_state"));
    }

    #[tokio::test]
    async fn no_params_returns_error() {
        let full = build_full_registry();
        let lazy = wrap_registry_lazy(full);
        let search_tool = lazy.get("tool_search").unwrap();

        let result = search_tool.execute(serde_json::json!({})).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Provide either"));
    }

    #[tokio::test]
    async fn name_takes_priority_over_query() {
        let full = build_full_registry();
        let lazy = wrap_registry_lazy(full);
        let search_tool = lazy.get("tool_search").unwrap();

        // When both name and query are provided, name (activation) takes priority.
        let result = search_tool
            .execute(serde_json::json!({ "name": "exec", "query": "memory" }))
            .await
            .unwrap();

        assert_eq!(result["activated"], true);
        assert_eq!(result["name"], "exec");
    }

    #[test]
    fn search_results_capped_at_max() {
        let mut registry = ToolRegistry::new();
        for i in 0..20 {
            registry.register(Box::new(DummyTool::new(
                &format!("tool_{i}"),
                "a matching description",
            )));
        }

        let lazy = wrap_registry_lazy(registry);
        let search = ToolSearchTool {
            full_registry: Arc::clone(
                // Access the full registry via the search tool.
                // We need to create a new one for this test.
                &{
                    let mut r = ToolRegistry::new();
                    for i in 0..20 {
                        r.register(Box::new(DummyTool::new(
                            &format!("tool_{i}"),
                            "a matching description",
                        )));
                    }
                    Arc::new(r)
                },
            ),
            activated: lazy.activated.clone(),
        };

        let results = search.keyword_search("matching");
        assert!(results.len() <= MAX_SEARCH_RESULTS);
    }

    #[tokio::test]
    async fn search_no_match_returns_empty() {
        let full = build_full_registry();
        let lazy = wrap_registry_lazy(full);
        let search_tool = lazy.get("tool_search").unwrap();

        let result = search_tool
            .execute(serde_json::json!({ "query": "zzzznonexistent" }))
            .await
            .unwrap();

        let results = result["results"].as_array().unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn activated_tool_is_executable() {
        let full = build_full_registry();
        let lazy = wrap_registry_lazy(full);
        let search_tool = lazy.get("tool_search").unwrap();

        // Activate exec.
        search_tool
            .execute(serde_json::json!({ "name": "exec" }))
            .await
            .unwrap();

        // Now get and execute it through the lazy registry.
        let exec_tool = lazy.get("exec").unwrap();
        let result = exec_tool
            .execute(serde_json::json!({ "input": "hello" }))
            .await
            .unwrap();

        assert_eq!(result["ok"], true);
    }
}
