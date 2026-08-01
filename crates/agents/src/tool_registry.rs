use {
    anyhow::Result,
    async_trait::async_trait,
    moltis_config::schema::McpServerId,
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
    Mcp { server: McpServerId },
    /// Tool provided by a precompiled WASM component.
    Wasm { component_hash: [u8; 32] },
}

/// The least-trusted audience allowed to use a tool.
///
/// Tools are trusted-only unless their registration site explicitly opts into
/// public use. Third-party metadata must not be able to widen this host-owned
/// security boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolAudience {
    /// Safe for untrusted channel and webhook turns.
    Public,
    /// Available only to trusted owner/operator turns.
    #[default]
    Trusted,
}

impl ToolAudience {
    fn allows(self, required: Self) -> bool {
        matches!(self, Self::Trusted) || matches!(required, Self::Public)
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Trusted => "trusted",
        }
    }
}

/// Internal entry pairing a tool with host-owned metadata.
#[derive(Clone)]
pub(crate) struct ToolEntry {
    pub(crate) tool: Arc<dyn AgentTool>,
    pub(crate) source: ToolSource,
    pub(crate) audience: ToolAudience,
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

    /// Whether the registry has no static or lazily activated tools.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
            && self
                .activated
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .is_empty()
    }

    /// Register a trusted-only built-in tool.
    ///
    /// Name collisions are rejected so an unrelated implementation cannot
    /// inherit another tool's policy identity.
    pub fn register(&mut self, tool: Box<dyn AgentTool>) {
        self.register_builtin(tool, ToolAudience::Trusted);
    }

    /// Register a built-in tool reviewed as safe for untrusted input.
    pub fn register_public(&mut self, tool: Box<dyn AgentTool>) {
        self.register_builtin(tool, ToolAudience::Public);
    }

    fn register_builtin(&mut self, tool: Box<dyn AgentTool>, audience: ToolAudience) {
        self.insert(tool, ToolSource::Builtin, audience);
    }

    /// Register a trusted-only tool from an MCP server.
    pub fn register_mcp(&mut self, tool: Box<dyn AgentTool>, server: McpServerId) {
        self.insert(tool, ToolSource::Mcp { server }, ToolAudience::Trusted);
    }

    /// Register a trusted-only tool from a WASM component.
    pub fn register_wasm(&mut self, tool: Box<dyn AgentTool>, component_hash: [u8; 32]) {
        self.insert(
            tool,
            ToolSource::Wasm { component_hash },
            ToolAudience::Trusted,
        );
    }

    fn insert(&mut self, tool: Box<dyn AgentTool>, source: ToolSource, audience: ToolAudience) {
        let name = tool.name().to_string();
        if let Some(existing) = self.tools.get(&name) {
            warn!(
                tool = %name,
                old_source = ?existing.source,
                new_source = ?source,
                "tool name collision; rejecting new registration"
            );
            return;
        }
        self.tools.insert(name, ToolEntry {
            tool: Arc::from(tool),
            source,
            audience,
        });
    }

    /// Replace an existing tool by name, preserving its host-owned metadata.
    ///
    /// Returns `true` if an existing tool was replaced, `false` if this was a new entry.
    pub fn replace(&mut self, tool: Box<dyn AgentTool>) -> bool {
        let name = tool.name().to_string();
        let metadata = self
            .tools
            .get(&name)
            .map(|entry| (entry.source.clone(), entry.audience))
            .unwrap_or((ToolSource::Builtin, ToolAudience::Trusted));
        self.tools
            .insert(name, ToolEntry {
                tool: Arc::from(tool),
                source: metadata.0,
                audience: metadata.1,
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

    /// Return the [`ToolAudience`] for a tool by name.
    pub(crate) fn get_audience(&self, name: &str) -> Option<ToolAudience> {
        self.tools.get(name).map(|e| e.audience)
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

    pub fn list_schemas_allowed_by<F>(&self, mut predicate: F) -> Vec<serde_json::Value>
    where
        F: FnMut(&str) -> bool,
    {
        self.list_schemas()
            .into_iter()
            .filter(|schema| {
                schema
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(&mut predicate)
            })
            .collect()
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
            .map(|(name, entry)| (name.clone(), entry.clone()))
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
            .map(|(name, entry)| (name.clone(), entry.clone()))
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
            .map(|(name, entry)| (name.clone(), entry.clone()))
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
        self.clone_allowed_entries(|name, _| predicate(name))
    }

    /// Merge another registry while preserving each tool's host-owned metadata.
    pub fn extend_from(&mut self, other: &ToolRegistry) {
        self.tools.extend(
            other
                .tools
                .iter()
                .map(|(name, entry)| (name.clone(), entry.clone())),
        );
    }

    /// Clone the registry keeping only tools whose name and source metadata match `predicate`.
    pub fn clone_allowed_entries<F>(&self, mut predicate: F) -> ToolRegistry
    where
        F: FnMut(&str, &ToolSource) -> bool,
    {
        let tools = self
            .tools
            .iter()
            .filter(|(name, entry)| predicate(name, &entry.source))
            .map(|(name, entry)| (name.clone(), entry.clone()))
            .collect();
        ToolRegistry {
            tools,
            activated: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Clone the registry at an audience ceiling.
    ///
    /// A public caller receives only explicitly reviewed public tools. Trusted
    /// callers retain the complete registry. Name-based policy can narrow the
    /// result later but cannot restore entries removed here.
    pub fn clone_for_audience(&self, audience: ToolAudience) -> ToolRegistry {
        let tools = self
            .tools
            .iter()
            .filter(|(_, entry)| audience.allows(entry.audience))
            .map(|(name, entry)| (name.clone(), entry.clone()))
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
        "audience": e.audience.as_str(),
    });
    match &e.source {
        ToolSource::Builtin => {
            schema["source"] = serde_json::json!("builtin");
        },
        ToolSource::Mcp { server } => {
            schema["source"] = serde_json::json!("mcp");
            schema["mcpServer"] = serde_json::json!(server.as_str());
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
            McpServerId::from("github"),
        );
        registry.register_mcp(
            Box::new(DummyTool {
                name: "mcp__memory__store".to_string(),
            }),
            McpServerId::from("memory"),
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
            McpServerId::from("github"),
        );
        registry.register_mcp(
            Box::new(DummyTool {
                name: "mcp__memory__store".to_string(),
            }),
            McpServerId::from("memory"),
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
            McpServerId::from("github"),
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
        assert_eq!(builtin["audience"], "trusted");
        assert!(builtin.get("mcpServer").is_none() || builtin["mcpServer"].is_null());

        let mcp = schemas
            .iter()
            .find(|s| s["name"] == "mcp__github__search")
            .expect("mcp tool should exist");
        assert_eq!(mcp["source"], "mcp");
        assert_eq!(mcp["mcpServer"], "github");
        assert_eq!(mcp["audience"], "trusted");
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
    fn test_register_collision_keeps_original_metadata() {
        let mut registry = ToolRegistry::new();
        registry.register_public(Box::new(DummyTool {
            name: "Read".to_string(),
        }));
        registry.register_mcp(
            Box::new(DummyTool {
                name: "Read".to_string(),
            }),
            McpServerId::from("filesystem"),
        );
        assert_eq!(registry.list_names(), vec!["Read".to_string()]);
        assert_eq!(registry.get_source("Read"), Some(ToolSource::Builtin));
        assert_eq!(registry.get_audience("Read"), Some(ToolAudience::Public));
    }

    #[test]
    fn test_register_mcp_cannot_overwrite_builtin() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool {
            name: "Read".to_string(),
        }));
        registry.register_mcp(
            Box::new(DummyTool {
                name: "Read".to_string(),
            }),
            McpServerId::from("filesystem"),
        );
        let src = registry.get_source("Read").unwrap();
        assert_eq!(src, ToolSource::Builtin);
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

    #[test]
    fn public_audience_excludes_new_and_external_tools_by_default() {
        let mut registry = ToolRegistry::new();
        registry.register_public(Box::new(DummyTool {
            name: "calc".to_string(),
        }));
        registry.register(Box::new(DummyTool {
            name: "future_builtin".to_string(),
        }));
        registry.register_mcp(
            Box::new(DummyTool {
                name: "mcp__example__read".to_string(),
            }),
            McpServerId::from("example"),
        );
        registry.register_wasm(
            Box::new(DummyTool {
                name: "future_wasm".to_string(),
            }),
            [0xCD; 32],
        );

        let public = registry.clone_for_audience(ToolAudience::Public);
        assert_eq!(public.list_names(), vec!["calc".to_string()]);
        assert!(public.get("future_builtin").is_none());
        assert!(public.get("mcp__example__read").is_none());
        assert!(public.get("future_wasm").is_none());
    }

    #[test]
    fn trusted_audience_keeps_all_tools() {
        let mut registry = ToolRegistry::new();
        registry.register_public(Box::new(DummyTool {
            name: "calc".to_string(),
        }));
        registry.register(Box::new(DummyTool {
            name: "exec".to_string(),
        }));

        let trusted = registry.clone_for_audience(ToolAudience::Trusted);
        assert_eq!(trusted.list_names(), vec!["calc", "exec"]);
    }
}
