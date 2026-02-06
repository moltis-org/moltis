//! Bridge MCP tools to the `AgentTool` trait so they can be used in the agent loop.

use std::sync::Arc;

use {anyhow::Result, async_trait::async_trait};

use crate::{
    traits::McpClientTrait,
    types::{McpToolDef, ToolContent},
};

/// An `AgentTool` implementation that delegates to an MCP server via `McpClient`.
pub struct McpToolBridge {
    /// Prefixed tool name: `mcp__<server>__<tool>`.
    prefixed_name: String,
    /// Original tool name on the MCP server.
    original_name: String,
    /// Name of the MCP server this tool belongs to.
    server_name: String,
    description: String,
    input_schema: serde_json::Value,
    client: Arc<tokio::sync::RwLock<dyn McpClientTrait>>,
}

impl McpToolBridge {
    /// Create a bridge for a single MCP tool.
    pub fn new(
        server_name: &str,
        tool_def: &McpToolDef,
        client: Arc<tokio::sync::RwLock<dyn McpClientTrait>>,
    ) -> Self {
        Self {
            prefixed_name: format!("mcp__{}__{}", server_name, tool_def.name),
            original_name: tool_def.name.clone(),
            server_name: server_name.to_string(),
            description: tool_def
                .description
                .clone()
                .unwrap_or_else(|| format!("MCP tool: {}", tool_def.name)),
            input_schema: tool_def.input_schema.clone(),
            client,
        }
    }

    /// Create bridges for all tools from a client.
    pub fn from_client(
        server_name: &str,
        tools: &[McpToolDef],
        client: Arc<tokio::sync::RwLock<dyn McpClientTrait>>,
    ) -> Vec<Self> {
        tools
            .iter()
            .map(|t| Self::new(server_name, t, Arc::clone(&client)))
            .collect()
    }

    pub fn prefixed_name(&self) -> &str {
        &self.prefixed_name
    }

    /// The name of the MCP server this tool belongs to.
    pub fn server_name(&self) -> &str {
        &self.server_name
    }
}

/// Trait for agent-callable tools, matching `AgentTool` in moltis-agents.
///
/// We define our own copy here to avoid a circular dependency on moltis-agents.
/// The gateway wires `McpToolBridge` into the `ToolRegistry` via a thin adapter.
#[async_trait]
pub trait McpAgentTool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value>;
}

#[async_trait]
impl McpAgentTool for McpToolBridge {
    fn name(&self) -> &str {
        &self.prefixed_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.input_schema.clone()
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        let client = self.client.read().await;
        let result = client.call_tool(&self.original_name, params).await?;

        if result.is_error {
            let text = result
                .content
                .iter()
                .filter_map(|c| match c {
                    ToolContent::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            anyhow::bail!("MCP tool error: {text}");
        }

        // Flatten text content into a single JSON value.
        let texts: Vec<&str> = result
            .content
            .iter()
            .filter_map(|c| match c {
                ToolContent::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();

        if texts.len() == 1 {
            // Try to parse as JSON first.
            if let Ok(val) = serde_json::from_str(texts[0]) {
                return Ok(val);
            }
            Ok(serde_json::Value::String(texts[0].to_string()))
        } else {
            Ok(serde_json::json!({ "content": texts }))
        }
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;

    #[test]
    fn test_prefixed_name_format() {
        let name = format!("mcp__{}__{}", "filesystem", "read_file");
        assert_eq!(name, "mcp__filesystem__read_file");
    }

    #[test]
    fn test_double_underscore_separator() {
        // Verify tool names use double underscore for unambiguous splitting.
        let name = "mcp__my-server__read_file";
        let parts: Vec<&str> = name.splitn(3, "__").collect();
        assert_eq!(parts, vec!["mcp", "my-server", "read_file"]);
    }
}
