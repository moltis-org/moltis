//! Agent tools for MCP server management.
//!
//! These tools let the agent manage MCP servers directly without needing
//! sandbox network access or the `moltis-ctl` CLI.

use std::sync::Arc;

use {
    async_trait::async_trait,
    moltis_agents::tool_registry::AgentTool,
    serde_json::{Value, json},
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
                    "type": "object",
                    "description": "Environment variables as key-value pairs"
                },
                "display_name": {
                    "type": "string",
                    "description": "Human-readable display name"
                }
            }
        })
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
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
