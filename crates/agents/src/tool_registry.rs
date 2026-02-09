use {
    anyhow::Result,
    async_trait::async_trait,
    std::{collections::HashMap, sync::Arc},
};

/// Agent-callable tool.
#[async_trait]
pub trait AgentTool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value>;
}

/// Where a tool originates from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolSource {
    /// Built-in tool shipped with the binary.
    Builtin,
    /// Tool provided by an MCP server.
    Mcp { server: String },
}

/// Internal entry pairing a tool with its source metadata.
struct ToolEntry {
    tool: Arc<dyn AgentTool>,
    source: ToolSource,
}

/// Registry of available tools for an agent run.
///
/// Tools are stored as `Arc<dyn AgentTool>` so the registry can be cheaply
/// cloned (e.g. for sub-agents that need a filtered copy of the parent's tools).
pub struct ToolRegistry {
    tools: HashMap<String, ToolEntry>,
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
        }
    }

    /// Register a built-in tool.
    pub fn register(&mut self, tool: Box<dyn AgentTool>) {
        let name = tool.name().to_string();
        self.tools.insert(name, ToolEntry {
            tool: Arc::from(tool),
            source: ToolSource::Builtin,
        });
    }

    /// Register a tool from an MCP server.
    pub fn register_mcp(&mut self, tool: Box<dyn AgentTool>, server: String) {
        let name = tool.name().to_string();
        self.tools.insert(name, ToolEntry {
            tool: Arc::from(tool),
            source: ToolSource::Mcp { server },
        });
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

    pub fn get(&self, name: &str) -> Option<&dyn AgentTool> {
        self.tools.get(name).map(|e| e.tool.as_ref())
    }

    pub fn list_schemas(&self) -> Vec<serde_json::Value> {
        self.tools
            .values()
            .map(|e| {
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
                }
                schema
            })
            .collect()
    }

    /// List registered tool names.
    pub fn list_names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    /// Clone the registry, excluding tools whose names start with `prefix`.
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
        ToolRegistry { tools }
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
        ToolRegistry { tools }
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
        ToolRegistry { tools }
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
        ToolRegistry { tools }
    }
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

        let mut names = registry.list_names();
        names.sort();
        assert_eq!(names, vec!["exec".to_string(), "web_fetch".to_string()]);
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
        let mut names = filtered.list_names();
        names.sort();
        assert_eq!(names, vec!["exec".to_string(), "web_fetch".to_string()]);
    }
}
