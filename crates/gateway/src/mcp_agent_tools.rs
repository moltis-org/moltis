//! Agent tools for MCP server management.
//!
//! These tools let the agent manage MCP servers directly without needing
//! sandbox network access or the `moltis-ctl` CLI.

use std::sync::Arc;

use {
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    serde_json::{Map, Value, json},
};

use crate::services::McpService;

// ── mcp_list ────────────────────────────────────────────────────────────────

pub struct McpListTool {
    service: Arc<dyn McpService>,
}

impl McpListTool {
    pub fn new(service: Arc<dyn McpService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl AgentTool for McpListTool {
    fn name(&self) -> &str {
        "mcp_list"
    }

    fn description(&self) -> &str {
        "List all configured MCP servers with their connection status, transport type, and enabled state."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _params: Value) -> anyhow::Result<Value> {
        self.service
            .list()
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
}

// ── mcp_add ─────────────────────────────────────────────────────────────────

pub struct McpAddTool {
    service: Arc<dyn McpService>,
}

impl McpAddTool {
    pub fn new(service: Arc<dyn McpService>) -> Self {
        Self { service }
    }
}

fn normalize_env(mut params: Value) -> anyhow::Result<Value> {
    let Some(env) = params.get_mut("env") else {
        return Ok(params);
    };
    if env.is_null() {
        return Ok(params);
    }
    let entries = env
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("'env' must be an array of name/value entries"))?;
    let mut normalized = Map::new();
    for entry in entries {
        let name = entry
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| anyhow::anyhow!("each 'env' entry must have a non-empty name"))?;
        let value = entry
            .get("value")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("each 'env' entry must have a string value"))?;
        if normalized.contains_key(name) {
            return Err(anyhow::anyhow!("duplicate 'env' entry name '{name}'"));
        }
        normalized.insert(name.to_string(), Value::String(value.to_string()));
    }
    *env = Value::Object(normalized);
    Ok(params)
}

#[async_trait]
impl AgentTool for McpAddTool {
    fn name(&self) -> &str {
        "mcp_add"
    }

    fn description(&self) -> &str {
        "Add a new MCP server. For stdio transport, provide command and args. For remote transports (sse, streamable-http), provide url."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Server name (identifier for mcp-servers.json)"
                },
                "command": {
                    "type": "string",
                    "description": "Command to run (stdio transport)"
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Command arguments"
                },
                "transport": {
                    "type": "string",
                    "enum": ["stdio", "sse", "streamable-http"],
                    "description": "Transport type (default: stdio)"
                },
                "url": {
                    "type": "string",
                    "description": "URL for remote transports (sse, streamable-http)"
                },
                "env": {
                    "type": "array",
                    "description": "Environment variables as name/value entries.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string", "description": "Environment variable name" },
                            "value": { "type": "string", "description": "Environment variable value" }
                        },
                        "required": ["name", "value"]
                    }
                },
                "display_name": {
                    "type": "string",
                    "description": "Human-readable display name"
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let params = normalize_env(params)?;
        self.service
            .add(params)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
}

// ── mcp_remove ──────────────────────────────────────────────────────────────

pub struct McpRemoveTool {
    service: Arc<dyn McpService>,
}

impl McpRemoveTool {
    pub fn new(service: Arc<dyn McpService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl AgentTool for McpRemoveTool {
    fn name(&self) -> &str {
        "mcp_remove"
    }

    fn description(&self) -> &str {
        "Remove an MCP server by name."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Server name to remove"
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        self.service
            .remove(params)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
}

// ── mcp_status ──────────────────────────────────────────────────────────────

pub struct McpStatusTool {
    service: Arc<dyn McpService>,
}

impl McpStatusTool {
    pub fn new(service: Arc<dyn McpService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl AgentTool for McpStatusTool {
    fn name(&self) -> &str {
        "mcp_status"
    }

    fn description(&self) -> &str {
        "Show detailed status for a specific MCP server including connection state and errors."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Server name to check"
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        self.service
            .status(params)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
}

// ── mcp_restart ─────────────────────────────────────────────────────────────

pub struct McpRestartTool {
    service: Arc<dyn McpService>,
}

impl McpRestartTool {
    pub fn new(service: Arc<dyn McpService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl AgentTool for McpRestartTool {
    fn name(&self) -> &str {
        "mcp_restart"
    }

    fn description(&self) -> &str {
        "Restart an MCP server. Useful after configuration changes."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Server name to restart"
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        self.service
            .restart(params)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
}

#[allow(clippy::unwrap_used)]
#[cfg(test)]
mod tests {
    use moltis_service_traits::NoopMcpService;

    use super::*;

    #[test]
    fn env_schema_uses_name_value_entries() {
        let tool = McpAddTool::new(Arc::new(NoopMcpService));
        let schema = tool.parameters_schema();
        let env = &schema["properties"]["env"];
        let items = &env["items"];

        assert_eq!(env["type"], "array");
        assert_eq!(items["type"], "object");
        assert_eq!(items["properties"]["name"]["type"], "string");
        assert_eq!(items["properties"]["value"]["type"], "string");
        assert_eq!(items["required"], json!(["name", "value"]));
    }

    #[test]
    fn normalizes_env_entries_to_object() {
        let params = normalize_env(json!({
            "name": "example",
            "env": [
                { "name": "API_TOKEN", "value": "secret" },
                { "name": "REGION", "value": "eu" }
            ]
        }))
        .unwrap();

        assert_eq!(params["env"]["API_TOKEN"], "secret");
        assert_eq!(params["env"]["REGION"], "eu");
    }

    #[test]
    fn rejects_object_env_input() {
        let error = normalize_env(json!({"env": {"API_TOKEN": "secret"}})).unwrap_err();
        assert!(error.to_string().contains("must be an array"));
    }

    #[test]
    fn rejects_duplicate_env_entry_names() {
        let error = normalize_env(json!({
            "env": [
                { "name": "API_TOKEN", "value": "first" },
                { "name": "API_TOKEN", "value": "second" }
            ]
        }))
        .unwrap_err();
        assert!(error.to_string().contains("duplicate 'env' entry name"));
    }
}
