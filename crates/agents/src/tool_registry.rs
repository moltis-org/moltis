use {
    anyhow::Result,
    async_trait::async_trait,
    std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    },
    tracing::warn,
};

/// Agent-callable tool.
#[async_trait]
pub trait AgentTool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    /// Opportunistic post-start initialization hook.
    async fn warmup(&self) -> Result<()> {
        Ok(())
    }
    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value>;
}

/// Where a tool originates from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolSource {
    /// Built-in tool shipped with the binary.
    Builtin,
    /// Tool provided by an MCP server.
    Mcp { server: String },
    /// Tool provided by a precompiled WASM component.
    Wasm { component_hash: [u8; 32] },
}

/// Internal entry pairing a tool with its source metadata.
pub(crate) struct ToolEntry {
    pub(crate) tool: Arc<dyn AgentTool>,
    pub(crate) source: ToolSource,
}

/// Shared set of tools activated at runtime by [`ToolSearchTool`](crate::lazy_tools::ToolSearchTool).
///
/// Uses `std::sync::Mutex` (not tokio) because the lock is held for
/// microseconds — just a `HashMap` insert/lookup — and this keeps
/// `list_schemas()` usable from sync contexts.
pub(crate) type ActivatedTools = Arc<Mutex<HashMap<String, ToolEntry>>>;

/// Registry of available tools for an agent run.
///
/// Tools are stored as `Arc<dyn AgentTool>` so the registry can be cheaply
/// cloned (e.g. for sub-agents that need a filtered copy of the parent's tools).
pub struct ToolRegistry {
    tools: HashMap<String, ToolEntry>,
    /// Tools activated at runtime via lazy tool discovery (`tool_search`).
    /// Always present (empty when lazy mode is not in use).
    pub(crate) activated: ActivatedTools,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            activated: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register a built-in tool. Warns (and overwrites) on name collision.
    pub fn register(&mut self, tool: Box<dyn AgentTool>) {
        let name = tool.name().to_string();
        let new_source = ToolSource::Builtin;
        if let Some(existing) = self.tools.get(&name) {
            warn!(
                tool = %name,
                old_source = ?existing.source,
                new_source = ?new_source,
                "tool name collision — new registration overwrites existing entry"
            );
        }
        self.tools.insert(name, ToolEntry {
            tool: Arc::from(tool),
            source: new_source,
        });
    }

    /// Register a tool from an MCP server. Warns (and overwrites) on name collision.
    pub fn register_mcp(&mut self, tool: Box<dyn AgentTool>, server: String) {
        let name = tool.name().to_string();
        let new_source = ToolSource::Mcp { server };
        if let Some(existing) = self.tools.get(&name) {
            warn!(
                tool = %name,
                old_source = ?existing.source,
                new_source = ?new_source,
                "tool name collision — new registration overwrites existing entry"
            );
        }
        self.tools.insert(name, ToolEntry {
            tool: Arc::from(tool),
            source: new_source,
        });
    }

    /// Register a tool from a WASM component. Warns (and overwrites) on name collision.
    pub fn register_wasm(&mut self, tool: Box<dyn AgentTool>, component_hash: [u8; 32]) {
        let name = tool.name().to_string();
        let new_source = ToolSource::Wasm { component_hash };
        if let Some(existing) = self.tools.get(&name) {
            warn!(
                tool = %name,
                old_source = ?existing.source,
                new_source = ?new_source,
                "tool name collision — new registration overwrites existing entry"
            );
        }
        self.tools.insert(name, ToolEntry {
            tool: Arc::from(tool),
            source: new_source,
        });
    }

    /// Replace an existing tool by name, preserving its source metadata.
    ///
    /// Returns `true` if an existing tool was replaced, `false` if this was a new entry.
    pub fn replace(&mut self, tool: Box<dyn AgentTool>) -> bool {
        let name = tool.name().to_string();
        let source = self
            .tools
            .get(&name)
            .map(|entry| entry.source.clone())
            .unwrap_or(ToolSource::Builtin);
        self.tools
            .insert(name, ToolEntry {
                tool: Arc::from(tool),
                source,
            })
            .is_some()
    }

    pub fn unregister(&mut self, name: &str) -> bool {
        self.tools.remove(name).is_some()
    }

    /// Remove all MCP-sourced tools. Returns the number of tools removed.
    pub fn unregister_mcp(&mut self) -> usize {
        let before = self.tools.len();
        self.tools
            .retain(|_, entry| !matches!(entry.source, ToolSource::Mcp { .. }));
        before - self.tools.len()
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn AgentTool>> {
        if let Some(e) = self.tools.get(name) {
            return Some(Arc::clone(&e.tool));
        }
        let activated = self.activated.lock().unwrap_or_else(|e| e.into_inner());
        activated.get(name).map(|e| Arc::clone(&e.tool))
    }

    /// Return the [`ToolSource`] for a tool by name.
    pub(crate) fn get_source(&self, name: &str) -> Option<ToolSource> {
        self.tools.get(name).map(|e| e.source.clone())
    }

    pub fn list_schemas(&self) -> Vec<serde_json::Value> {
        let mut schemas: Vec<serde_json::Value> = self
            .tools
            .iter()
            .filter(|(name, _)| is_public_tool_name(name))
            .map(|(_, entry)| entry_to_schema(entry))
            .collect();

        let activated = self.activated.lock().unwrap_or_else(|e| e.into_inner());
        for (name, entry) in activated.iter() {
            if !self.tools.contains_key(name) && is_public_tool_name(name) {
                schemas.push(entry_to_schema(entry));
            }
        }
        schemas.sort_by(|left, right| {
            let left_name = left
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let right_name = right
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            left_name.cmp(right_name)
        });
        schemas
    }

    /// List registered tool names (static + activated).
    pub fn list_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .tools
            .keys()
            .filter(|name| is_public_tool_name(name))
            .cloned()
            .collect();
        let activated = self.activated.lock().unwrap_or_else(|e| e.into_inner());
        for name in activated.keys() {
            if !self.tools.contains_key(name) && is_public_tool_name(name) {
                names.push(name.clone());
            }
        }
        names.sort();
        names
    }

    /// Clone the registry, excluding tools whose names start with `prefix`.
    ///
    /// Sub-agent registries get a fresh (empty) activated set.
    pub fn clone_without_prefix(&self, prefix: &str) -> ToolRegistry {
        let tools = self
            .tools
            .iter()
            .filter(|(name, _)| !name.starts_with(prefix))
            .map(|(name, entry)| {
                (name.clone(), ToolEntry {
                    tool: Arc::clone(&entry.tool),
                    source: entry.source.clone(),
                })
            })
            .collect();
        ToolRegistry {
            tools,
            activated: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Clone the registry, excluding all MCP-sourced tools.
    pub fn clone_without_mcp(&self) -> ToolRegistry {
        let tools = self
            .tools
            .iter()
            .filter(|(_, entry)| !matches!(entry.source, ToolSource::Mcp { .. }))
            .map(|(name, entry)| {
                (name.clone(), ToolEntry {
                    tool: Arc::clone(&entry.tool),
                    source: entry.source.clone(),
                })
            })
            .collect();
        ToolRegistry {
            tools,
            activated: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Clone the registry, excluding tools whose names are in `exclude`.
    pub fn clone_without(&self, exclude: &[&str]) -> ToolRegistry {
        let tools = self
            .tools
            .iter()
            .filter(|(name, _)| !exclude.contains(&name.as_str()))
            .map(|(name, entry)| {
                (name.clone(), ToolEntry {
                    tool: Arc::clone(&entry.tool),
                    source: entry.source.clone(),
                })
            })
            .collect();
        ToolRegistry {
            tools,
            activated: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Clone the registry keeping only tools that match `predicate`.
    pub fn clone_allowed_by<F>(&self, mut predicate: F) -> ToolRegistry
    where
        F: FnMut(&str) -> bool,
    {
        let tools = self
            .tools
            .iter()
            .filter(|(name, _)| predicate(name))
            .map(|(name, entry)| {
                (name.clone(), ToolEntry {
                    tool: Arc::clone(&entry.tool),
                    source: entry.source.clone(),
                })
            })
            .collect();
        ToolRegistry {
            tools,
            activated: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

fn is_public_tool_name(name: &str) -> bool {
    !name.ends_with("_wasm")
}

fn entry_to_schema(e: &ToolEntry) -> serde_json::Value {
    let mut schema = serde_json::json!({
        "name": e.tool.name(),
        "description": e.tool.description(),
        "parameters": e.tool.parameters_schema(),
    });
    match &e.source {
        ToolSource::Builtin => {
            schema["source"] = serde_json::json!("builtin");
        },
        ToolSource::Mcp { server } => {
            schema["source"] = serde_json::json!("mcp");
            schema["mcpServer"] = serde_json::json!(server);
        },
        ToolSource::Wasm { component_hash } => {
            schema["source"] = serde_json::json!("wasm");
            schema["componentHash"] = serde_json::json!(hex_component_hash(*component_hash));
        },
    }
    schema
}

fn hex_component_hash(component_hash: [u8; 32]) -> String {
    let mut output = String::with_capacity(component_hash.len() * 2);
    for byte in component_hash {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    struct DummyTool {
        name: String,
    }

    #[async_trait]
    impl AgentTool for DummyTool {
        fn name(&self) -> &str {
            &self.name
        }

        fn description(&self) -> &str {
            "test"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }

        async fn execute(&self, _params: serde_json::Value) -> Result<serde_json::Value> {
            Ok(serde_json::json!({}))
        }
    }

    #[test]
    fn test_clone_without_prefix() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool {
            name: "exec".to_string(),
        }));
        registry.register(Box::new(DummyTool {
            name: "web_fetch".to_string(),
        }));
        registry.register(Box::new(DummyTool {
            name: "mcp__github_search".to_string(),
        }));
        registry.register(Box::new(DummyTool {
            name: "mcp__memory_store".to_string(),
        }));

        let filtered = registry.clone_without_prefix("mcp__");
        assert_eq!(filtered.list_schemas().len(), 2);
        assert!(filtered.get("exec").is_some());
        assert!(filtered.get("web_fetch").is_some());
        assert!(filtered.get("mcp__github_search").is_none());
        assert!(filtered.get("mcp__memory_store").is_none());
    }

    #[test]
    fn test_clone_without_prefix_no_match() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool {
            name: "exec".to_string(),
        }));
        registry.register(Box::new(DummyTool {
            name: "web_fetch".to_string(),
        }));

        let filtered = registry.clone_without_prefix("mcp__");
        assert_eq!(filtered.list_schemas().len(), 2);
    }

    #[test]
    fn test_clone_without_mcp() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool {
            name: "exec".to_string(),
        }));
        registry.register_mcp(
            Box::new(DummyTool {
                name: "mcp__github__search".to_string(),
            }),
            "github".to_string(),
        );
        registry.register_mcp(
            Box::new(DummyTool {
                name: "mcp__memory__store".to_string(),
            }),
            "memory".to_string(),
        );

        let filtered = registry.clone_without_mcp();
        assert_eq!(filtered.list_schemas().len(), 1);
        assert!(filtered.get("exec").is_some());
        assert!(filtered.get("mcp__github__search").is_none());
        assert!(filtered.get("mcp__memory__store").is_none());
    }

    #[test]
    fn test_unregister_mcp() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool {
            name: "exec".to_string(),
        }));
        registry.register_mcp(
            Box::new(DummyTool {
                name: "mcp__github__search".to_string(),
            }),
            "github".to_string(),
        );
        registry.register_mcp(
            Box::new(DummyTool {
                name: "mcp__memory__store".to_string(),
            }),
            "memory".to_string(),
        );

        let removed = registry.unregister_mcp();
        assert_eq!(removed, 2);
        assert_eq!(registry.list_schemas().len(), 1);
        assert!(registry.get("exec").is_some());
    }

    #[test]
    fn test_list_schemas_includes_source() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool {
            name: "exec".to_string(),
        }));
        registry.register_mcp(
            Box::new(DummyTool {
                name: "mcp__github__search".to_string(),
            }),
            "github".to_string(),
        );
        registry.register_wasm(
            Box::new(DummyTool {
                name: "calc_wasm".to_string(),
            }),
            [0xAB; 32],
        );

        let schemas = registry.list_schemas();
        let builtin = schemas
            .iter()
            .find(|s| s["name"] == "exec")
            .expect("exec should exist");
        assert_eq!(builtin["source"], "builtin");
        assert!(builtin.get("mcpServer").is_none() || builtin["mcpServer"].is_null());

        let mcp = schemas
            .iter()
            .find(|s| s["name"] == "mcp__github__search")
            .expect("mcp tool should exist");
        assert_eq!(mcp["source"], "mcp");
        assert_eq!(mcp["mcpServer"], "github");
    }

    #[test]
    fn test_list_names() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool {
            name: "exec".to_string(),
        }));
        registry.register(Box::new(DummyTool {
            name: "web_fetch".to_string(),
        }));

        let names = registry.list_names();
        assert_eq!(names, vec!["exec".to_string(), "web_fetch".to_string()]);
    }

    #[test]
    fn test_wasm_suffix_tools_are_hidden_from_public_lists() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool {
            name: "exec".to_string(),
        }));
        registry.register_wasm(
            Box::new(DummyTool {
                name: "web_search_wasm".to_string(),
            }),
            [0xAB; 32],
        );

        assert_eq!(registry.list_names(), vec!["exec".to_string()]);
        assert!(registry.get("web_search_wasm").is_some());

        let schemas = registry.list_schemas();
        assert!(
            schemas
                .iter()
                .all(|schema| schema["name"] != "web_search_wasm")
        );
    }

    #[test]
    fn test_list_schemas_are_sorted_by_name() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool {
            name: "zeta".to_string(),
        }));
        registry.register(Box::new(DummyTool {
            name: "alpha".to_string(),
        }));
        registry.register(Box::new(DummyTool {
            name: "mu".to_string(),
        }));

        let names: Vec<String> = registry
            .list_schemas()
            .into_iter()
            .filter_map(|schema| {
                schema
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .map(ToString::to_string)
            })
            .collect();

        assert_eq!(names, vec!["alpha", "mu", "zeta"]);
    }

    #[test]
    fn test_get_returns_cloned_tool_handle() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool {
            name: "exec".to_string(),
        }));
        assert!(registry.get("exec").is_some());
        assert!(registry.get("missing").is_none());
    }

    #[test]
    fn test_register_collision_overwrites_with_warning() {
        // The warn! output is emitted via tracing; we assert the overwrite
        // semantics and trust the log at runtime.
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool {
            name: "Read".to_string(),
        }));
        // Same name again — should overwrite, warn logged.
        registry.register(Box::new(DummyTool {
            name: "Read".to_string(),
        }));
        assert_eq!(registry.list_names(), vec!["Read".to_string()]);
    }

    #[test]
    fn test_register_mcp_overwriting_builtin_warns() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool {
            name: "Read".to_string(),
        }));
        registry.register_mcp(
            Box::new(DummyTool {
                name: "Read".to_string(),
            }),
            "filesystem".to_string(),
        );
        // Source should now be Mcp even though the builtin was registered first.
        let src = registry.get_source("Read").unwrap();
        assert!(matches!(src, ToolSource::Mcp { .. }));
    }

    #[test]
    fn test_clone_allowed_by() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool {
            name: "exec".to_string(),
        }));
        registry.register(Box::new(DummyTool {
            name: "web_fetch".to_string(),
        }));
        registry.register(Box::new(DummyTool {
            name: "session_state".to_string(),
        }));

        let filtered = registry.clone_allowed_by(|name| name.starts_with("web") || name == "exec");
        let names = filtered.list_names();
        assert_eq!(names, vec!["exec".to_string(), "web_fetch".to_string()]);
    }
}
