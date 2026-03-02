//! McpManager: lifecycle management for multiple MCP server connections.

use std::{collections::HashMap, sync::Arc};

use {
    tokio::sync::RwLock,
    tracing::{info, warn},
};

use crate::{
    auth::{McpAuthState, McpOAuthOverride, McpOAuthProvider, SharedAuthProvider},
    client::{McpClient, McpClientState},
    error::{Context, Error, Result},
    registry::{McpOAuthConfig, McpRegistry, McpServerConfig, TransportType},
    tool_bridge::McpToolBridge,
    traits::McpClientTrait,
    types::{McpManagerError, McpToolDef, McpTransportError},
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
    /// OAuth authentication state (only for SSE servers with auth).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_state: Option<McpAuthState>,
    /// Pending OAuth URL to open in browser (when auth_state is awaiting_browser).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_url: Option<String>,
}

/// Mutable state behind the single `RwLock` on [`McpManager`].
pub struct McpManagerInner {
    pub clients: HashMap<String, Arc<RwLock<dyn McpClientTrait>>>,
    pub tools: HashMap<String, Vec<McpToolDef>>,
    pub registry: McpRegistry,
    /// OAuth auth providers for SSE servers, keyed by server name.
    pub auth_providers: HashMap<String, SharedAuthProvider>,
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
                auth_providers: HashMap::new(),
            }),
        }
    }

    fn build_auth_provider(
        name: &str,
        url: &str,
        oauth: Option<&McpOAuthConfig>,
    ) -> SharedAuthProvider {
        let provider = if let Some(ov) = oauth {
            McpOAuthProvider::new(name, url).with_oauth_override(McpOAuthOverride {
                client_id: ov.client_id.clone(),
                auth_url: ov.auth_url.clone(),
                token_url: ov.token_url.clone(),
                scopes: ov.scopes.clone(),
            })
        } else {
            McpOAuthProvider::new(name, url)
        };
        Arc::new(provider)
    }

    fn should_attempt_auth_connection(
        has_existing_auth_provider: bool,
        has_oauth_override: bool,
        has_stored_token: bool,
    ) -> bool {
        has_existing_auth_provider || has_oauth_override || has_stored_token
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
    ///
    /// For SSE servers: attempts unauthenticated first. On 401 Unauthorized,
    /// stores auth context and returns `McpManagerError::OAuthRequired`.
    pub async fn start_server(&self, name: &str, config: &McpServerConfig) -> Result<()> {
        // Shut down existing connection if any.
        self.stop_server(name).await;

        // Network work happens outside the lock.
        let (client, auth_provider) = match config.transport {
            TransportType::Sse => {
                let url = config
                    .url
                    .as_deref()
                    .with_context(|| format!("SSE transport for '{name}' requires a url"))?;

                // Check if we already have an auth provider (from a previous connection).
                let existing_auth = {
                    let inner = self.inner.read().await;
                    inner.auth_providers.get(name).cloned()
                };

                let has_existing_auth_provider = existing_auth.is_some();
                let auth_provider = existing_auth
                    .unwrap_or_else(|| Self::build_auth_provider(name, url, config.oauth.as_ref()));

                // If we have a stored token, prefer auth transport immediately.
                // This avoids forced re-auth at process start for OAuth-backed servers.
                let has_stored_token = if has_existing_auth_provider {
                    false
                } else {
                    auth_provider.access_token().await?.is_some()
                };

                if Self::should_attempt_auth_connection(
                    has_existing_auth_provider,
                    config.oauth.is_some(),
                    has_stored_token,
                ) {
                    let client =
                        McpClient::connect_sse_with_auth(name, url, auth_provider.clone()).await?;
                    (client, Some(auth_provider))
                } else {
                    // No hint that auth is needed yet, probe unauthenticated first.
                    match McpClient::connect_sse(name, url).await {
                        Ok(client) => (client, None),
                        Err(e) => {
                            // Check if it's a 401 Unauthorized.
                            if let Error::Transport(McpTransportError::Unauthorized {
                                www_authenticate,
                            }) = &e
                            {
                                info!(
                                    server = %name,
                                    "SSE server requires auth"
                                );

                                // Mark auth as required and persist challenge metadata.
                                let auth_ok = auth_provider
                                    .handle_unauthorized(www_authenticate.as_deref())
                                    .await?;

                                if !auth_ok {
                                    let mut inner = self.inner.write().await;
                                    inner.auth_providers.insert(name.to_string(), auth_provider);
                                    return Err(McpManagerError::OAuthRequired {
                                        server: name.to_string(),
                                    }
                                    .into());
                                }

                                // Retry with auth.
                                let client = McpClient::connect_sse_with_auth(
                                    name,
                                    url,
                                    auth_provider.clone(),
                                )
                                .await?;
                                (client, Some(auth_provider))
                            } else {
                                return Err(e);
                            }
                        },
                    }
                }
            },
            TransportType::Stdio => {
                let client =
                    McpClient::connect(name, &config.command, &config.args, &config.env).await?;
                (client, None)
            },
        };

        // Fetch tools.
        let mut client = client;
        let tool_defs = client.list_tools().await?.to_vec();
        info!(
            server = %name,
            tools = tool_defs.len(),
            "MCP server started with tools"
        );

        // Atomic insert of client, tools, and auth provider.
        let client: Arc<RwLock<dyn McpClientTrait>> = Arc::new(RwLock::new(client));
        let mut inner = self.inner.write().await;
        inner.clients.insert(name.to_string(), client);
        inner.tools.insert(name.to_string(), tool_defs);

        if let Some(auth) = auth_provider {
            inner.auth_providers.insert(name.to_string(), auth);
        }

        Ok(())
    }

    /// Stop a server connection.
    pub async fn stop_server(&self, name: &str) {
        // Atomically remove client and tools, then drop the lock before async shutdown.
        // Keep auth_providers for potential reconnection.
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

    /// Start OAuth for an SSE server and return the browser authorization URL.
    pub async fn oauth_start_server(&self, name: &str, redirect_uri: &str) -> Result<String> {
        let config =
            {
                let inner = self.inner.read().await;
                inner.registry.get(name).cloned().ok_or_else(|| {
                    McpManagerError::ServerNotFound {
                        server: name.to_string(),
                    }
                })?
            };

        if !matches!(config.transport, TransportType::Sse) {
            return Err(McpManagerError::NotSseTransport {
                server: name.to_string(),
            }
            .into());
        }

        let url = config
            .url
            .as_deref()
            .ok_or_else(|| McpManagerError::MissingSseUrl {
                server: name.to_string(),
            })?;

        let existing_auth = {
            let inner = self.inner.read().await;
            inner.auth_providers.get(name).cloned()
        };
        let has_existing_auth_provider = existing_auth.is_some();
        let auth_provider = existing_auth
            .unwrap_or_else(|| Self::build_auth_provider(name, url, config.oauth.as_ref()));

        if !has_existing_auth_provider {
            let mut inner = self.inner.write().await;
            inner
                .auth_providers
                .insert(name.to_string(), auth_provider.clone());
        }

        auth_provider
            .start_oauth(redirect_uri, None)
            .await?
            .with_context(|| format!("MCP server '{name}' does not support OAuth"))
    }

    /// Complete an OAuth callback by matching state across MCP auth providers.
    ///
    /// Returns the server name whose OAuth flow was completed.
    pub async fn oauth_complete_callback(&self, state: &str, code: &str) -> Result<String> {
        let providers: Vec<(String, SharedAuthProvider)> = {
            let inner = self.inner.read().await;
            inner
                .auth_providers
                .iter()
                .map(|(name, provider)| (name.clone(), provider.clone()))
                .collect()
        };

        for (name, provider) in providers {
            if provider.complete_oauth(state, code).await? {
                self.restart_server(&name).await?;
                return Ok(name);
            }
        }

        Err(McpManagerError::OAuthStateNotFound.into())
    }

    /// Trigger re-authentication for an SSE server.
    pub async fn reauth_server(&self, name: &str, redirect_uri: &str) -> Result<String> {
        self.oauth_start_server(name, redirect_uri).await
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
                    McpClientState::Authenticating => "authenticating",
                    McpClientState::Closed => "stopped",
                }
            } else {
                "stopped"
            };

            let auth_state = inner.auth_providers.get(name).map(|a| a.auth_state());
            let auth_url = inner
                .auth_providers
                .get(name)
                .and_then(|a| a.pending_auth_url());

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
                auth_state,
                auth_url,
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
        inner.auth_providers.remove(name);
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

    #[test]
    fn test_should_attempt_auth_connection_with_existing_provider() {
        assert!(McpManager::should_attempt_auth_connection(
            true, false, false
        ));
    }

    #[test]
    fn test_should_attempt_auth_connection_with_oauth_override() {
        assert!(McpManager::should_attempt_auth_connection(
            false, true, false
        ));
    }

    #[test]
    fn test_should_attempt_auth_connection_with_stored_token() {
        assert!(McpManager::should_attempt_auth_connection(
            false, false, true
        ));
    }

    #[test]
    fn test_should_attempt_auth_connection_without_auth_signals() {
        assert!(!McpManager::should_attempt_auth_connection(
            false, false, false
        ));
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
        assert!(statuses[0].auth_state.is_none());
    }

    #[tokio::test]
    async fn test_reauth_server_no_auth_provider() {
        let mgr = McpManager::new(McpRegistry::new());
        let result = mgr
            .reauth_server("nonexistent", "https://example.com/auth/callback")
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_oauth_start_server_requires_sse_transport() {
        let mut reg = McpRegistry::new();
        reg.servers.insert("stdio".into(), McpServerConfig {
            command: "echo".into(),
            transport: TransportType::Stdio,
            ..Default::default()
        });
        let mgr = McpManager::new(reg);
        let err = mgr
            .oauth_start_server("stdio", "https://example.com/auth/callback")
            .await
            .expect_err("expected oauth start to fail for stdio transport");
        assert!(matches!(
            err,
            Error::Manager(McpManagerError::NotSseTransport { .. })
        ));
    }

    #[tokio::test]
    async fn test_oauth_complete_callback_unknown_state() {
        let mgr = McpManager::new(McpRegistry::new());
        let err = mgr
            .oauth_complete_callback("unknown-state", "code")
            .await
            .expect_err("expected unknown state to fail");
        assert!(matches!(
            err,
            Error::Manager(McpManagerError::OAuthStateNotFound)
        ));
    }
}
