//! Agent-callable tools for querying and selecting remote nodes.
//!
//! Provides `nodes_list`, `nodes_describe`, and `nodes_select` tools that let
//! the agent programmatically interact with connected device nodes.

use std::sync::Arc;

use {
    async_trait::async_trait,
    serde::{Deserialize, Serialize},
};

use moltis_agents::tool_registry::AgentTool;

// ── Domain types ────────────────────────────────────────────────────────────

/// Serializable summary of a connected node, returned by the list/describe tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeInfo {
    pub node_id: String,
    pub display_name: Option<String>,
    pub platform: String,
    pub capabilities: Vec<String>,
    pub commands: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_ip: Option<String>,
    // Telemetry
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mem_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mem_available: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_usage: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uptime_secs: Option<u64>,
    pub services: Vec<String>,
    pub telemetry_stale: bool,
    // P1 fields (populated when provider discovery / richer telemetry lands)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk_available: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtimes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<NodeProviderInfo>,
}

/// A provider discovered on a remote node (e.g. ollama, openai env key).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeProviderInfo {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
}

// ── Provider trait ──────────────────────────────────────────────────────────

/// Abstraction that the gateway implements to supply node data to the tools
/// crate without a direct dependency.
///
/// Follows the same pattern as [`crate::exec::NodeExecProvider`].
#[async_trait]
pub trait NodeInfoProvider: Send + Sync {
    /// List all currently connected nodes.
    async fn list_nodes(&self) -> Vec<NodeInfo>;

    /// Describe a single node by id or display name.
    async fn describe_node(&self, node_ref: &str) -> Option<NodeInfo>;

    /// Assign (or clear) a node for a chat session.
    /// `node_ref` is an id or display name; `None` clears the assignment.
    async fn set_session_node(
        &self,
        session_key: &str,
        node_ref: Option<&str>,
    ) -> anyhow::Result<Option<String>>;

    /// Resolve a node reference (id or display name) to a canonical node_id.
    async fn resolve_node_id(&self, node_ref: &str) -> Option<String>;
}

// ── NodesListTool ───────────────────────────────────────────────────────────

pub struct NodesListTool {
    provider: Arc<dyn NodeInfoProvider>,
}

impl NodesListTool {
    pub fn new(provider: Arc<dyn NodeInfoProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl AgentTool for NodesListTool {
    fn name(&self) -> &str {
        "nodes_list"
    }

    fn description(&self) -> &str {
        "List all connected remote device nodes with their platform, capabilities, and telemetry."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false,
        })
    }

    async fn execute(&self, _params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let nodes = self.provider.list_nodes().await;
        Ok(serde_json::to_value(&nodes)?)
    }
}

// ── NodesDescribeTool ───────────────────────────────────────────────────────

pub struct NodesDescribeTool {
    provider: Arc<dyn NodeInfoProvider>,
}

impl NodesDescribeTool {
    pub fn new(provider: Arc<dyn NodeInfoProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl AgentTool for NodesDescribeTool {
    fn name(&self) -> &str {
        "nodes_describe"
    }

    fn description(&self) -> &str {
        "Get detailed information about a specific connected node by id or name."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "node": {
                    "type": "string",
                    "description": "Node id or display name",
                },
            },
            "required": ["node"],
            "additionalProperties": false,
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let node_ref = params
            .get("node")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter 'node'"))?;

        match self.provider.describe_node(node_ref).await {
            Some(info) => Ok(serde_json::to_value(&info)?),
            None => Ok(serde_json::json!({ "error": format!("node '{}' not found", node_ref) })),
        }
    }
}

// ── NodesSelectTool ─────────────────────────────────────────────────────────

pub struct NodesSelectTool {
    provider: Arc<dyn NodeInfoProvider>,
}

impl NodesSelectTool {
    pub fn new(provider: Arc<dyn NodeInfoProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl AgentTool for NodesSelectTool {
    fn name(&self) -> &str {
        "nodes_select"
    }

    fn description(&self) -> &str {
        "Set or clear the default remote node for the current chat session. \
         Pass a node id or name to target it, or null/omit to revert to local execution."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "node": {
                    "type": ["string", "null"],
                    "description": "Node id or display name, or null to clear",
                },
                "_session_key": {
                    "type": "string",
                    "description": "Session key (injected by the runtime)",
                },
            },
            "additionalProperties": false,
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let session_key = params
            .get("_session_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing '_session_key' in tool context"))?;

        let node_ref = params.get("node").and_then(|v| v.as_str());

        let resolved = self
            .provider
            .set_session_node(session_key, node_ref)
            .await?;

        Ok(serde_json::json!({
            "ok": true,
            "node_id": resolved,
        }))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    // ── Mock provider ───────────────────────────────────────────────────────

    struct MockNodeInfoProvider {
        nodes: Vec<NodeInfo>,
        selected: std::sync::Mutex<Option<String>>,
    }

    impl MockNodeInfoProvider {
        fn new(nodes: Vec<NodeInfo>) -> Self {
            Self {
                nodes,
                selected: std::sync::Mutex::new(None),
            }
        }

        fn sample_node(id: &str, name: Option<&str>, platform: &str) -> NodeInfo {
            NodeInfo {
                node_id: id.to_string(),
                display_name: name.map(String::from),
                platform: platform.to_string(),
                capabilities: vec!["system.run".to_string()],
                commands: vec!["system.run".to_string()],
                remote_ip: Some("192.168.1.10".to_string()),
                mem_total: Some(8_589_934_592),
                mem_available: Some(4_294_967_296),
                cpu_count: Some(4),
                cpu_usage: Some(25.0),
                uptime_secs: Some(3600),
                services: vec![],
                telemetry_stale: false,
                disk_total: None,
                disk_available: None,
                runtimes: vec![],
                providers: vec![],
            }
        }
    }

    #[async_trait]
    impl NodeInfoProvider for MockNodeInfoProvider {
        async fn list_nodes(&self) -> Vec<NodeInfo> {
            self.nodes.clone()
        }

        async fn describe_node(&self, node_ref: &str) -> Option<NodeInfo> {
            let lower = node_ref.to_lowercase();
            self.nodes
                .iter()
                .find(|n| {
                    n.node_id == node_ref
                        || n.display_name
                            .as_ref()
                            .is_some_and(|name| name.to_lowercase() == lower)
                })
                .cloned()
        }

        async fn set_session_node(
            &self,
            _session_key: &str,
            node_ref: Option<&str>,
        ) -> anyhow::Result<Option<String>> {
            let resolved = match node_ref {
                Some(r) => self.resolve_node_id(r).await,
                None => None,
            };
            *self.selected.lock().unwrap_or_else(|e| e.into_inner()) = resolved.clone();
            Ok(resolved)
        }

        async fn resolve_node_id(&self, node_ref: &str) -> Option<String> {
            let lower = node_ref.to_lowercase();
            self.nodes
                .iter()
                .find(|n| {
                    n.node_id == node_ref
                        || n.display_name
                            .as_ref()
                            .is_some_and(|name| name.to_lowercase() == lower)
                })
                .map(|n| n.node_id.clone())
        }
    }

    fn two_nodes() -> Vec<NodeInfo> {
        vec![
            MockNodeInfoProvider::sample_node("node-1", Some("MacBook"), "macos"),
            MockNodeInfoProvider::sample_node("node-2", Some("Pi"), "linux"),
        ]
    }

    // ── Tool schema tests ───────────────────────────────────────────────────

    #[test]
    fn tool_schemas_valid() {
        let provider: Arc<dyn NodeInfoProvider> = Arc::new(MockNodeInfoProvider::new(vec![]));

        let list_tool = NodesListTool::new(Arc::clone(&provider));
        assert_eq!(list_tool.name(), "nodes_list");
        let schema = list_tool.parameters_schema();
        assert_eq!(schema["type"], "object");

        let describe_tool = NodesDescribeTool::new(Arc::clone(&provider));
        assert_eq!(describe_tool.name(), "nodes_describe");
        let schema = describe_tool.parameters_schema();
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("node"))
        );

        let select_tool = NodesSelectTool::new(Arc::clone(&provider));
        assert_eq!(select_tool.name(), "nodes_select");
        let schema = select_tool.parameters_schema();
        assert!(schema["properties"]["node"].is_object());
    }

    // ── List tool tests ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn nodes_list_returns_all_connected() {
        let provider: Arc<dyn NodeInfoProvider> = Arc::new(MockNodeInfoProvider::new(two_nodes()));
        let tool = NodesListTool::new(provider);

        let result = tool.execute(serde_json::json!({})).await.unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["nodeId"], "node-1");
        assert_eq!(arr[1]["nodeId"], "node-2");
    }

    #[tokio::test]
    async fn nodes_list_empty() {
        let provider: Arc<dyn NodeInfoProvider> = Arc::new(MockNodeInfoProvider::new(vec![]));
        let tool = NodesListTool::new(provider);

        let result = tool.execute(serde_json::json!({})).await.unwrap();
        let arr = result.as_array().unwrap();
        assert!(arr.is_empty());
    }

    // ── Describe tool tests ─────────────────────────────────────────────────

    #[tokio::test]
    async fn nodes_describe_by_id() {
        let provider: Arc<dyn NodeInfoProvider> = Arc::new(MockNodeInfoProvider::new(two_nodes()));
        let tool = NodesDescribeTool::new(provider);

        let result = tool
            .execute(serde_json::json!({ "node": "node-1" }))
            .await
            .unwrap();
        assert_eq!(result["nodeId"], "node-1");
        assert_eq!(result["displayName"], "MacBook");
    }

    #[tokio::test]
    async fn nodes_describe_by_name() {
        let provider: Arc<dyn NodeInfoProvider> = Arc::new(MockNodeInfoProvider::new(two_nodes()));
        let tool = NodesDescribeTool::new(provider);

        let result = tool
            .execute(serde_json::json!({ "node": "pi" }))
            .await
            .unwrap();
        assert_eq!(result["nodeId"], "node-2");
    }

    #[tokio::test]
    async fn nodes_describe_not_found() {
        let provider: Arc<dyn NodeInfoProvider> = Arc::new(MockNodeInfoProvider::new(two_nodes()));
        let tool = NodesDescribeTool::new(provider);

        let result = tool
            .execute(serde_json::json!({ "node": "nonexistent" }))
            .await
            .unwrap();
        assert!(result["error"].as_str().unwrap().contains("not found"));
    }

    // ── Select tool tests ───────────────────────────────────────────────────

    #[tokio::test]
    async fn nodes_select_assigns_node() {
        let mock = Arc::new(MockNodeInfoProvider::new(two_nodes()));
        let provider: Arc<dyn NodeInfoProvider> = Arc::clone(&mock) as _;
        let tool = NodesSelectTool::new(provider);

        let result = tool
            .execute(serde_json::json!({
                "node": "MacBook",
                "_session_key": "test-session",
            }))
            .await
            .unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(result["node_id"], "node-1");
        assert_eq!(*mock.selected.lock().unwrap(), Some("node-1".to_string()));
    }

    #[tokio::test]
    async fn nodes_select_clear_assignment() {
        let mock = Arc::new(MockNodeInfoProvider::new(two_nodes()));
        let provider: Arc<dyn NodeInfoProvider> = Arc::clone(&mock) as _;
        let tool = NodesSelectTool::new(provider);

        // First assign
        tool.execute(serde_json::json!({
            "node": "node-1",
            "_session_key": "test-session",
        }))
        .await
        .unwrap();

        // Clear
        let result = tool
            .execute(serde_json::json!({
                "node": null,
                "_session_key": "test-session",
            }))
            .await
            .unwrap();
        assert_eq!(result["ok"], true);
        assert!(result["node_id"].is_null());
        assert_eq!(*mock.selected.lock().unwrap(), None);
    }

    #[tokio::test]
    async fn nodes_select_missing_session_key() {
        let provider: Arc<dyn NodeInfoProvider> = Arc::new(MockNodeInfoProvider::new(two_nodes()));
        let tool = NodesSelectTool::new(provider);

        let result = tool.execute(serde_json::json!({ "node": "node-1" })).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("_session_key"));
    }
}
