//! McpManager: lifecycle management for multiple MCP server connections.

use std::{collections::HashMap, sync::Arc};

use {
    anyhow::{Context, Result},
    tokio::sync::RwLock,
    tracing::{info, warn},
};

use crate::{
    client::{McpClient, McpClientState},
    registry::{McpRegistry, McpServerConfig},
    tool_bridge::McpToolBridge,
    traits::McpClientTrait,
    types::McpToolDef,
};

/// Status of a managed MCP server.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ServerStatus {
    pub name: String,
    pub state: String,
    pub enabled: bool,
    pub tool_count: usize,
    pub server_info: Option<String>,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub transport: crate::registry::TransportType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Mutable state behind the single `RwLock` on [`McpManager`].
pub struct McpManagerInner {
    pub clients: HashMap<String, Arc<RwLock<dyn McpClientTrait>>>,
    pub tools: HashMap<String, Vec<McpToolDef>>,
    pub registry: McpRegistry,
}

/// Manages the lifecycle of multiple MCP server connections.
pub struct McpManager {
    pub inner: RwLock<McpManagerInner>,
}

impl McpManager {
    pub fn new(registry: McpRegistry) -> Self {
        Self {
            inner: RwLock::new(McpManagerInner {
                clients: HashMap::new(),
                tools: HashMap::new(),
                registry,
            }),
        }
    }

    /// Start all enabled servers from the registry.
    pub async fn start_enabled(&self) -> Vec<String> {
        let enabled: Vec<(String, McpServerConfig)> = {
            let inner = self.inner.read().await;
            inner
                .registry
                .enabled_servers()
                .into_iter()
                .map(|(name, cfg)| (name.to_string(), cfg.clone()))
                .collect()
        };

        let mut started = Vec::new();
        for (name, config) in enabled {
            match self.start_server(&name, &config).await {
                Ok(()) => started.push(name),
                Err(e) => warn!(server = %name, error = %e, "failed to start MCP server"),
            }
        }
        started
    }

    /// Start a single server connection.
    pub async fn start_server(&self, name: &str, config: &McpServerConfig) -> Result<()> {
        use crate::registry::TransportType;

        // Shut down existing connection if any.
        self.stop_server(name).await;

        // Network work happens outside the lock.
        let mut client = match config.transport {
            TransportType::Sse => {
                let url = config
                    .url
                    .as_deref()
                    .with_context(|| format!("SSE transport for '{name}' requires a url"))?;
                McpClient::connect_sse(name, url).await?
            },
            TransportType::Stdio => {
                McpClient::connect(name, &config.command, &config.args, &config.env).await?
            },
        };

        // Fetch tools.
        let tool_defs = client.list_tools().await?.to_vec();
        info!(
            server = %name,
            tools = tool_defs.len(),
            "MCP server started with tools"
        );

        // Atomic insert of both client and tools.
        let client: Arc<RwLock<dyn McpClientTrait>> = Arc::new(RwLock::new(client));
        let mut inner = self.inner.write().await;
        inner.clients.insert(name.to_string(), client);
        inner.tools.insert(name.to_string(), tool_defs);

        Ok(())
    }

    /// Stop a server connection.
    pub async fn stop_server(&self, name: &str) {
        // Atomically remove client and tools, then drop the lock before async shutdown.
        let client = {
            let mut inner = self.inner.write().await;
            inner.tools.remove(name);
            inner.clients.remove(name)
        };
        if let Some(client) = client {
            let mut c = client.write().await;
            c.shutdown().await;
        }
    }

    /// Restart a server.
    pub async fn restart_server(&self, name: &str) -> Result<()> {
        let config = {
            let inner = self.inner.read().await;
            inner
                .registry
                .get(name)
                .cloned()
                .with_context(|| format!("MCP server '{name}' not found in registry"))?
        };
        self.start_server(name, &config).await
    }

    /// Get the status of all configured servers.
    pub async fn status_all(&self) -> Vec<ServerStatus> {
        let inner = self.inner.read().await;

        let mut statuses = Vec::new();
        for (name, config) in &inner.registry.servers {
            let state = if let Some(client) = inner.clients.get(name) {
                let c = client.read().await;
                match c.state() {
                    McpClientState::Ready => {
                        if c.is_alive().await {
                            "running"
                        } else {
                            "dead"
                        }
                    },
                    McpClientState::Connected => "connecting",
                    McpClientState::Closed => "stopped",
                }
            } else {
                "stopped"
            };

            statuses.push(ServerStatus {
                name: name.clone(),
                state: state.into(),
                enabled: config.enabled,
                tool_count: inner.tools.get(name).map_or(0, |t| t.len()),
                server_info: None,
                command: config.command.clone(),
                args: config.args.clone(),
                env: config.env.clone(),
                transport: config.transport,
                url: config.url.clone(),
            });
        }
        statuses
    }

    /// Get the status of a single server.
    pub async fn status(&self, name: &str) -> Option<ServerStatus> {
        self.status_all().await.into_iter().find(|s| s.name == name)
    }

    /// Get tool bridges for all running servers (for registration into ToolRegistry).
    pub async fn tool_bridges(&self) -> Vec<McpToolBridge> {
        let inner = self.inner.read().await;
        let mut bridges = Vec::new();

        for (name, client) in inner.clients.iter() {
            if let Some(tool_defs) = inner.tools.get(name) {
                bridges.extend(McpToolBridge::from_client(
                    name,
                    tool_defs,
                    Arc::clone(client),
                ));
            }
        }

        bridges
    }

    /// Get tools for a specific server.
    pub async fn server_tools(&self, name: &str) -> Option<Vec<McpToolDef>> {
        self.inner.read().await.tools.get(name).cloned()
    }

    // ── Registry operations ─────────────────────────────────────────

    /// Add a server to the registry and optionally start it.
    pub async fn add_server(
        &self,
        name: String,
        config: McpServerConfig,
        start: bool,
    ) -> Result<()> {
        let enabled = config.enabled;
        {
            let mut inner = self.inner.write().await;
            inner.registry.add(name.clone(), config.clone())?;
        }
        if start && enabled {
            self.start_server(&name, &config).await?;
        }
        Ok(())
    }

    /// Remove a server from the registry and stop it.
    pub async fn remove_server(&self, name: &str) -> Result<bool> {
        self.stop_server(name).await;
        let mut inner = self.inner.write().await;
        inner.registry.remove(name)
    }

    /// Enable a server and start it.
    pub async fn enable_server(&self, name: &str) -> Result<bool> {
        let config = {
            let mut inner = self.inner.write().await;
            if !inner.registry.enable(name)? {
                return Ok(false);
            }
            inner.registry.get(name).cloned()
        };
        if let Some(config) = config {
            self.start_server(name, &config).await?;
        }
        Ok(true)
    }

    /// Disable a server and stop it.
    pub async fn disable_server(&self, name: &str) -> Result<bool> {
        self.stop_server(name).await;
        let mut inner = self.inner.write().await;
        inner.registry.disable(name)
    }

    /// Get a snapshot of the registry for serialization.
    pub async fn registry_snapshot(&self) -> McpRegistry {
        self.inner.read().await.registry.clone()
    }

    /// Update a server's configuration and restart it if running.
    pub async fn update_server(&self, name: &str, config: McpServerConfig) -> Result<()> {
        let was_running = {
            let inner = self.inner.read().await;
            inner.clients.contains_key(name)
        };
        {
            let mut inner = self.inner.write().await;
            let enabled = inner.registry.get(name).is_none_or(|c| c.enabled);
            let mut new_config = config;
            new_config.enabled = enabled;
            inner.registry.add(name.to_string(), new_config)?;
        }
        if was_running {
            self.restart_server(name).await?;
        }
        Ok(())
    }

    /// Shut down all servers.
    pub async fn shutdown_all(&self) {
        let names: Vec<String> = self.inner.read().await.clients.keys().cloned().collect();
        for name in names {
            self.stop_server(&name).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manager_creation() {
        let reg = McpRegistry::new();
        let _mgr = McpManager::new(reg);
    }

    #[tokio::test]
    async fn test_status_all_empty() {
        let mgr = McpManager::new(McpRegistry::new());
        let statuses = mgr.status_all().await;
        assert!(statuses.is_empty());
    }

    #[tokio::test]
    async fn test_tool_bridges_empty() {
        let mgr = McpManager::new(McpRegistry::new());
        let bridges = mgr.tool_bridges().await;
        assert!(bridges.is_empty());
    }

    #[tokio::test]
    async fn test_status_shows_stopped_for_configured_but_not_started() {
        let mut reg = McpRegistry::new();
        reg.servers.insert("test".into(), McpServerConfig {
            command: "echo".into(),
            ..Default::default()
        });
        let mgr = McpManager::new(reg);

        let statuses = mgr.status_all().await;
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].state, "stopped");
        assert!(statuses[0].enabled);
    }
}
