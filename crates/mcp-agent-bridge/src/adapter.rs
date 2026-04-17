//! Bridge between MCP tools and the agent tool registry.
//!
//! Adapts [`McpToolBridge`](moltis_mcp::tool_bridge::McpToolBridge) (mcp crate)
//! to [`AgentTool`] (agents crate) and provides sync logic to register MCP tools
//! into a [`ToolRegistry`].

use std::sync::Arc;

use {async_trait::async_trait, tokio::sync::RwLock};

#[cfg(feature = "tracing")]
use tracing::info;

use {
    moltis_agents::tool_registry::{AgentTool, ToolRegistry},
    moltis_mcp::tool_bridge::{McpAgentTool, McpToolBridge},
};

/// Thin adapter that implements `AgentTool` (agents crate) by delegating to
/// `McpToolBridge` which implements `McpAgentTool` (mcp crate).
pub struct McpToolAdapter(pub McpToolBridge);

#[async_trait]
impl AgentTool for McpToolAdapter {
    fn name(&self) -> &str {
        McpAgentTool::name(&self.0)
    }

    fn description(&self) -> &str {
        McpAgentTool::description(&self.0)
    }

    fn parameters_schema(&self) -> serde_json::Value {
        McpAgentTool::parameters_schema(&self.0)
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        McpAgentTool::execute(&self.0, params)
            .await
            .map_err(anyhow::Error::from)
    }
}

/// Synchronize MCP tool bridges into the shared [`ToolRegistry`].
///
/// Removes all existing `mcp__*` tools and re-registers current bridges.
#[cfg_attr(feature = "tracing", tracing::instrument(skip_all, fields(tool_count)))]
pub async fn sync_mcp_tools(
    manager: &moltis_mcp::McpManager,
    registry: &Arc<RwLock<ToolRegistry>>,
) {
    let bridges = manager.tool_bridges().await;

    let mut reg = registry.write().await;

    // Remove all MCP-sourced tools before re-registering current ones.
    reg.unregister_mcp();

    // Register current bridges with their server name metadata.
    let count = bridges.len();
    for bridge in bridges {
        let server = bridge.server_name().to_string();
        reg.register_mcp(Box::new(McpToolAdapter(bridge)), server);
    }

    #[cfg(feature = "tracing")]
    if count > 0 {
        info!(tools = count, "MCP tools synced into tool registry");
    }
}

#[cfg(test)]
mod tests {
    use {super::*, moltis_mcp::McpRegistry};

    /// Minimal fake tool implementing AgentTool for testing sync logic.
    struct FakeTool(&'static str);

    #[async_trait]
    impl AgentTool for FakeTool {
        fn name(&self) -> &str {
            self.0
        }

        fn description(&self) -> &str {
            "fake"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }

        async fn execute(&self, _params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
            Ok(serde_json::json!({}))
        }
    }

    #[tokio::test]
    async fn test_sync_mcp_tools_empty_manager() {
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let reg = McpRegistry::load(&std::path::PathBuf::from("/nonexistent")).unwrap_or_default();
        let manager = moltis_mcp::McpManager::new_with_env_overrides(
            reg,
            std::collections::HashMap::new(),
            std::time::Duration::from_secs(30),
        );

        sync_mcp_tools(&manager, &registry).await;
        let reg_guard = registry.read().await;
        assert!(reg_guard.list_schemas().is_empty());
    }

    #[tokio::test]
    async fn test_sync_mcp_tools_removes_stale_tools() {
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let reg = McpRegistry::load(&std::path::PathBuf::from("/nonexistent")).unwrap_or_default();
        let manager = moltis_mcp::McpManager::new_with_env_overrides(
            reg,
            std::collections::HashMap::new(),
            std::time::Duration::from_secs(30),
        );

        // Manually register an MCP tool, then sync (which should clear it).
        {
            let mut reg_guard = registry.write().await;
            reg_guard.register_mcp(Box::new(FakeTool("stale_tool")), "stale_server".to_string());
        }

        sync_mcp_tools(&manager, &registry).await;
        let reg_guard = registry.read().await;
        assert!(
            reg_guard.list_schemas().is_empty(),
            "stale MCP tools should be removed"
        );
    }

    #[tokio::test]
    async fn test_sync_preserves_non_mcp_tools() {
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));
        let reg = McpRegistry::load(&std::path::PathBuf::from("/nonexistent")).unwrap_or_default();
        let manager = moltis_mcp::McpManager::new_with_env_overrides(
            reg,
            std::collections::HashMap::new(),
            std::time::Duration::from_secs(30),
        );

        // Register a non-MCP tool before sync — it should survive.
        {
            let mut reg_guard = registry.write().await;
            reg_guard.register(Box::new(FakeTool("builtin_tool")));
        }

        sync_mcp_tools(&manager, &registry).await;
        let reg_guard = registry.read().await;
        let schemas = reg_guard.list_schemas();
        assert_eq!(schemas.len(), 1, "non-MCP tool should survive sync");
        assert_eq!(schemas[0]["name"], "builtin_tool");
    }
}
