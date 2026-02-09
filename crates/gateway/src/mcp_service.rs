//! Live MCP service implementation backed by `McpManager`.

use std::sync::Arc;

use {
    anyhow::Result, async_trait::async_trait, serde_json::Value, tokio::sync::RwLock, tracing::info,
};

use {
    moltis_agents::tool_registry::{AgentTool, ToolRegistry},
    moltis_mcp::tool_bridge::{McpAgentTool, McpToolBridge},
};

use crate::services::{McpService, ServiceResult};

// ── McpToolAdapter: bridge McpAgentTool → AgentTool ─────────────────────────

/// Thin adapter that implements `AgentTool` (agents crate) by delegating to
/// `McpToolBridge` which implements `McpAgentTool` (mcp crate).
struct McpToolAdapter(McpToolBridge);

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

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        McpAgentTool::execute(&self.0, params).await
    }
}

// ── Sync helper ─────────────────────────────────────────────────────────────

/// Synchronize MCP tool bridges into the shared `ToolRegistry`.
///
/// Removes all existing `mcp__*` tools and re-registers current bridges.
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

    if count > 0 {
        info!(tools = count, "MCP tools synced into tool registry");
    }
}

// ── Config parsing helper ───────────────────────────────────────────────────

/// Extract an `McpServerConfig` from JSON params (used by both `add` and `update`).
fn parse_server_config(params: &Value) -> Result<moltis_mcp::McpServerConfig, String> {
    let command = params
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing 'command' parameter".to_string())?;
    let args: Vec<String> = params
        .get("args")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let env: std::collections::HashMap<String, String> = params
        .get("env")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let enabled = params
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let transport = match params
        .get("transport")
        .and_then(|v| v.as_str())
        .unwrap_or("stdio")
    {
        "sse" => moltis_mcp::TransportType::Sse,
        _ => moltis_mcp::TransportType::Stdio,
    };
    let url = params.get("url").and_then(|v| v.as_str()).map(String::from);

    Ok(moltis_mcp::McpServerConfig {
        command: command.into(),
        args,
        env,
        enabled,
        transport,
        url,
    })
}

// ── LiveMcpService ──────────────────────────────────────────────────────────

/// Live MCP service delegating to `McpManager`.
pub struct LiveMcpService {
    manager: Arc<moltis_mcp::McpManager>,
    /// Shared tool registry for syncing MCP tools into the agent loop.
    /// Set after construction via `set_tool_registry`.
    tool_registry: RwLock<Option<Arc<RwLock<ToolRegistry>>>>,
}

impl LiveMcpService {
    pub fn new(manager: Arc<moltis_mcp::McpManager>) -> Self {
        Self {
            manager,
            tool_registry: RwLock::new(None),
        }
    }

    /// Store a reference to the shared tool registry so MCP mutations
    /// can automatically sync tools.
    pub async fn set_tool_registry(&self, registry: Arc<RwLock<ToolRegistry>>) {
        *self.tool_registry.write().await = Some(registry);
    }

    /// Sync MCP tools into the shared tool registry (if set).
    pub async fn sync_tools_if_ready(&self) {
        let maybe_reg = self.tool_registry.read().await.clone();
        if let Some(reg) = maybe_reg {
            sync_mcp_tools(&self.manager, &reg).await;
        }
    }

    /// Access the underlying manager.
    pub fn manager(&self) -> &Arc<moltis_mcp::McpManager> {
        &self.manager
    }
}

#[async_trait]
impl McpService for LiveMcpService {
    async fn list(&self) -> ServiceResult {
        let statuses = self.manager.status_all().await;
        serde_json::to_value(&statuses).map_err(|e| e.to_string())
    }

    async fn add(&self, params: Value) -> ServiceResult {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'name' parameter".to_string())?;
        let config = parse_server_config(&params)?;

        // If a server with this name already exists, append a numeric suffix.
        let final_name = {
            let reg = self.manager.registry_snapshot().await;
            let mut candidate = name.to_string();
            let mut n = 2u32;
            while reg.servers.contains_key(&candidate) {
                candidate = format!("{name}-{n}");
                n += 1;
            }
            candidate
        };

        info!(server = %final_name, "adding MCP server via API");
        self.manager
            .add_server(final_name.clone(), config, true)
            .await
            .map_err(|e| e.to_string())?;

        self.sync_tools_if_ready().await;

        Ok(serde_json::json!({ "ok": true, "name": final_name }))
    }

    async fn remove(&self, params: Value) -> ServiceResult {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'name' parameter".to_string())?;

        let removed = self
            .manager
            .remove_server(name)
            .await
            .map_err(|e| e.to_string())?;

        self.sync_tools_if_ready().await;

        Ok(serde_json::json!({ "removed": removed }))
    }

    async fn enable(&self, params: Value) -> ServiceResult {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'name' parameter".to_string())?;

        self.manager
            .enable_server(name)
            .await
            .map_err(|e| e.to_string())?;

        self.sync_tools_if_ready().await;

        Ok(serde_json::json!({ "enabled": true }))
    }

    async fn disable(&self, params: Value) -> ServiceResult {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'name' parameter".to_string())?;

        let ok = self
            .manager
            .disable_server(name)
            .await
            .map_err(|e| e.to_string())?;

        self.sync_tools_if_ready().await;

        Ok(serde_json::json!({ "disabled": ok }))
    }

    async fn status(&self, params: Value) -> ServiceResult {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'name' parameter".to_string())?;

        match self.manager.status(name).await {
            Some(s) => serde_json::to_value(&s).map_err(|e| e.to_string()),
            None => Err(format!("MCP server '{name}' not found")),
        }
    }

    async fn tools(&self, params: Value) -> ServiceResult {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'name' parameter".to_string())?;

        match self.manager.server_tools(name).await {
            Some(tools) => serde_json::to_value(&tools).map_err(|e| e.to_string()),
            None => Err(format!("MCP server '{name}' not found or not running")),
        }
    }

    async fn restart(&self, params: Value) -> ServiceResult {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'name' parameter".to_string())?;

        self.manager
            .restart_server(name)
            .await
            .map_err(|e| e.to_string())?;

        self.sync_tools_if_ready().await;

        Ok(serde_json::json!({ "ok": true }))
    }

    async fn update(&self, params: Value) -> ServiceResult {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'name' parameter".to_string())?;
        let config = parse_server_config(&params)?;

        self.manager
            .update_server(name, config)
            .await
            .map_err(|e| e.to_string())?;

        self.sync_tools_if_ready().await;

        Ok(serde_json::json!({ "ok": true }))
    }
}

#[cfg(test)]
mod tests {
    use {super::*, moltis_mcp::McpRegistry};

    #[tokio::test]
    async fn test_sync_mcp_tools_empty_manager() {
        let manager = moltis_mcp::McpManager::new(McpRegistry::new());
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));

        sync_mcp_tools(&manager, &registry).await;

        let reg = registry.read().await;
        assert!(reg.list_schemas().is_empty());
    }

    #[tokio::test]
    async fn test_sync_mcp_tools_removes_stale_tools() {
        let manager = moltis_mcp::McpManager::new(McpRegistry::new());
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));

        // Manually register a fake MCP tool to simulate a stale entry.
        {
            let mut reg = registry.write().await;
            reg.register_mcp(
                Box::new(FakeTool("mcp__old__tool".into())),
                "old".to_string(),
            );
        }

        // Sync should remove it since there are no running MCP servers.
        sync_mcp_tools(&manager, &registry).await;

        let reg = registry.read().await;
        assert!(reg.get("mcp__old__tool").is_none());
    }

    #[tokio::test]
    async fn test_sync_preserves_non_mcp_tools() {
        let manager = moltis_mcp::McpManager::new(McpRegistry::new());
        let registry = Arc::new(RwLock::new(ToolRegistry::new()));

        {
            let mut reg = registry.write().await;
            reg.register(Box::new(FakeTool("exec".into())));
        }

        sync_mcp_tools(&manager, &registry).await;

        let reg = registry.read().await;
        assert!(reg.get("exec").is_some());
    }

    /// Minimal AgentTool implementation for testing.
    struct FakeTool(String);

    #[async_trait]
    impl AgentTool for FakeTool {
        fn name(&self) -> &str {
            &self.0
        }

        fn description(&self) -> &str {
            "fake"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }

        async fn execute(&self, _params: serde_json::Value) -> Result<serde_json::Value> {
            Ok(serde_json::json!({}))
        }
    }
}
