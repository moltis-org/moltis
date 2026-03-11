//! Live MCP service implementation backed by `McpManager`.

use std::{collections::HashMap, sync::Arc};

use {
    anyhow::Result,
    async_trait::async_trait,
    secrecy::{ExposeSecret, Secret},
    serde_json::Value,
    tokio::sync::RwLock,
    tracing::{info, warn},
};

use {
    moltis_agents::tool_registry::{AgentTool, ToolRegistry},
    moltis_mcp::tool_bridge::{McpAgentTool, McpToolBridge},
};

use crate::services::{McpService, ServiceError, ServiceResult};

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

    fn parameters_schema(&self) -> Value {
        McpAgentTool::parameters_schema(&self.0)
    }

    async fn execute(&self, params: Value) -> Result<Value> {
        McpAgentTool::execute(&self.0, params)
            .await
            .map_err(anyhow::Error::from)
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

/// Extract an `McpServerConfig` from JSON params.
///
/// For updates, omitted fields inherit from `existing`.
fn parse_server_config(
    params: &Value,
    existing: Option<&moltis_mcp::McpServerConfig>,
) -> Result<moltis_mcp::McpServerConfig, ServiceError> {
    let transport = match params.get("transport").and_then(|v| v.as_str()) {
        Some("sse") => moltis_mcp::TransportType::Sse,
        Some(_) => moltis_mcp::TransportType::Stdio,
        None => existing
            .map(|cfg| cfg.transport)
            .unwrap_or(moltis_mcp::TransportType::Stdio),
    };

    let command = params
        .get("command")
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| existing.map(|cfg| cfg.command.clone()))
        .unwrap_or_default();

    if matches!(transport, moltis_mcp::TransportType::Stdio) && command.trim().is_empty() {
        return Err(ServiceError::message("missing 'command' parameter"));
    }

    let args: Vec<String> = if params.get("args").is_some() {
        params
            .get("args")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default()
    } else {
        existing.map(|cfg| cfg.args.clone()).unwrap_or_default()
    };

    let enabled = params
        .get("enabled")
        .and_then(|v| v.as_bool())
        .or_else(|| existing.map(|cfg| cfg.enabled))
        .unwrap_or(true);

    let url = if params.get("url").is_some() {
        if params.get("url").is_some_and(Value::is_null) {
            None
        } else {
            params
                .get("url")
                .and_then(|v| v.as_str())
                .map(|value| Secret::new(value.to_string()))
        }
    } else {
        existing.and_then(|cfg| cfg.url.clone())
    };

    let headers = if matches!(transport, moltis_mcp::TransportType::Sse) {
        if params.get("headers").is_some() {
            parse_secret_string_map(params.get("headers").unwrap_or(&Value::Null))
        } else {
            existing.map(|cfg| cfg.headers.clone()).unwrap_or_default()
        }
    } else {
        HashMap::new()
    };

    let env = if matches!(transport, moltis_mcp::TransportType::Sse) {
        HashMap::new()
    } else if params.get("env").is_some() {
        parse_string_map(params.get("env").unwrap_or(&Value::Null))
    } else {
        existing.map(|cfg| cfg.env.clone()).unwrap_or_default()
    };

    if matches!(transport, moltis_mcp::TransportType::Sse)
        && url
            .as_ref()
            .map(ExposeSecret::expose_secret)
            .is_none_or(|candidate| candidate.trim().is_empty())
    {
        return Err(ServiceError::message(
            "missing 'url' parameter for 'sse' transport",
        ));
    }

    let oauth = if let Some(v) = params.get("oauth") {
        if v.is_null() {
            None
        } else {
            let client_id = v
                .get("client_id")
                .and_then(|val| val.as_str())
                .ok_or_else(|| ServiceError::message("missing 'oauth.client_id' parameter"))?
                .to_string();
            let auth_url = v
                .get("auth_url")
                .and_then(|val| val.as_str())
                .ok_or_else(|| ServiceError::message("missing 'oauth.auth_url' parameter"))?
                .to_string();
            let token_url = v
                .get("token_url")
                .and_then(|val| val.as_str())
                .ok_or_else(|| ServiceError::message("missing 'oauth.token_url' parameter"))?
                .to_string();
            let scopes: Vec<String> = v
                .get("scopes")
                .and_then(|s| serde_json::from_value(s.clone()).ok())
                .unwrap_or_default();
            Some(moltis_mcp::registry::McpOAuthConfig {
                client_id,
                auth_url,
                token_url,
                scopes,
            })
        }
    } else {
        existing.and_then(|cfg| cfg.oauth.clone())
    };

    Ok(moltis_mcp::McpServerConfig {
        command,
        args,
        env,
        enabled,
        transport,
        url: if matches!(transport, moltis_mcp::TransportType::Sse) {
            url
        } else {
            None
        },
        headers,
        oauth,
    })
}

fn parse_string_map(value: &Value) -> HashMap<String, String> {
    serde_json::from_value(value.clone()).unwrap_or_default()
}

fn parse_secret_string_map(value: &Value) -> HashMap<String, Secret<String>> {
    parse_string_map(value)
        .into_iter()
        .map(|(key, value)| (key, Secret::new(value)))
        .collect()
}

// ── LiveMcpService ──────────────────────────────────────────────────────────

/// Live MCP service delegating to `McpManager`.
pub struct LiveMcpService {
    manager: Arc<moltis_mcp::McpManager>,
    /// Shared tool registry for syncing MCP tools into the agent loop.
    /// Set after construction via `set_tool_registry`.
    tool_registry: RwLock<Option<Arc<RwLock<ToolRegistry>>>>,
    config_env_overrides: HashMap<String, String>,
    credential_store: RwLock<Option<Arc<crate::auth::CredentialStore>>>,
}

impl LiveMcpService {
    pub fn new(
        manager: Arc<moltis_mcp::McpManager>,
        config_env_overrides: HashMap<String, String>,
        credential_store: Option<Arc<crate::auth::CredentialStore>>,
    ) -> Self {
        Self {
            manager,
            tool_registry: RwLock::new(None),
            config_env_overrides,
            credential_store: RwLock::new(credential_store),
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

    pub async fn set_credential_store(&self, credential_store: Arc<crate::auth::CredentialStore>) {
        *self.credential_store.write().await = Some(credential_store);
    }

    async fn refresh_manager_env_overrides(&self) {
        let credential_store = self.credential_store.read().await.clone();
        let env_overrides = if let Some(store) = credential_store {
            match store.get_all_env_values().await {
                Ok(db_env_vars) => merge_env_overrides(&self.config_env_overrides, db_env_vars),
                Err(error) => {
                    warn!(%error, "failed to refresh MCP env overrides from credential store");
                    self.config_env_overrides.clone()
                },
            }
        } else {
            self.config_env_overrides.clone()
        };

        self.manager.set_env_overrides(env_overrides).await;
    }
}

fn merge_env_overrides(
    base_overrides: &HashMap<String, String>,
    additional: Vec<(String, String)>,
) -> HashMap<String, String> {
    // Config `[env]` values stay authoritative so checked-in config cannot be
    // silently shadowed by mutable UI-managed entries from the credential store.
    let mut merged = base_overrides.clone();
    for (key, value) in additional {
        if key.trim().is_empty() || value.trim().is_empty() {
            continue;
        }
        merged.entry(key).or_insert(value);
    }
    merged
}

#[async_trait]
impl McpService for LiveMcpService {
    async fn list(&self) -> ServiceResult {
        let statuses = self.manager.status_all().await;
        Ok(serde_json::to_value(&statuses)?)
    }

    async fn add(&self, params: Value) -> ServiceResult {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'name' parameter".to_string())?;
        let redirect_uri = params
            .get("redirectUri")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToOwned::to_owned);
        let config = parse_server_config(&params, None)?;
        self.refresh_manager_env_overrides().await;

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
        match self
            .manager
            .add_server(final_name.clone(), config, true)
            .await
        {
            Ok(_) => {
                self.sync_tools_if_ready().await;
                Ok(serde_json::json!({ "ok": true, "name": final_name }))
            },
            Err(e) => {
                if matches!(
                    e,
                    moltis_mcp::Error::Manager(moltis_mcp::McpManagerError::OAuthRequired { .. })
                ) {
                    if let Some(uri) = redirect_uri {
                        let auth_url = self
                            .manager
                            .oauth_start_server(&final_name, &uri)
                            .await
                            .map_err(ServiceError::message)?;
                        Ok(serde_json::json!({
                            "ok": true,
                            "name": final_name,
                            "oauthPending": true,
                            "authUrl": auth_url
                        }))
                    } else {
                        Ok(serde_json::json!({
                            "ok": true,
                            "name": final_name,
                            "oauthPending": true
                        }))
                    }
                } else {
                    Err(ServiceError::message(e))
                }
            },
        }
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
            .map_err(ServiceError::message)?;

        self.sync_tools_if_ready().await;

        Ok(serde_json::json!({ "removed": removed }))
    }

    async fn enable(&self, params: Value) -> ServiceResult {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'name' parameter".to_string())?;
        let redirect_uri = params
            .get("redirectUri")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToOwned::to_owned);
        self.refresh_manager_env_overrides().await;

        match self.manager.enable_server(name).await {
            Ok(_) => {
                self.sync_tools_if_ready().await;
                Ok(serde_json::json!({ "enabled": true }))
            },
            Err(e) => {
                if matches!(
                    e,
                    moltis_mcp::Error::Manager(moltis_mcp::McpManagerError::OAuthRequired { .. })
                ) {
                    if let Some(uri) = redirect_uri {
                        let auth_url = self
                            .manager
                            .oauth_start_server(name, &uri)
                            .await
                            .map_err(ServiceError::message)?;
                        Ok(serde_json::json!({
                            "enabled": false,
                            "oauthPending": true,
                            "authUrl": auth_url
                        }))
                    } else {
                        Ok(serde_json::json!({
                            "enabled": false,
                            "oauthPending": true
                        }))
                    }
                } else {
                    Err(ServiceError::message(e))
                }
            },
        }
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
            .map_err(ServiceError::message)?;

        self.sync_tools_if_ready().await;

        Ok(serde_json::json!({ "disabled": ok }))
    }

    async fn status(&self, params: Value) -> ServiceResult {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'name' parameter".to_string())?;

        match self.manager.status(name).await {
            Some(s) => Ok(serde_json::to_value(&s)?),
            None => Err(format!("MCP server '{name}' not found").into()),
        }
    }

    async fn tools(&self, params: Value) -> ServiceResult {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'name' parameter".to_string())?;

        match self.manager.server_tools(name).await {
            Some(tools) => Ok(serde_json::to_value(&tools)?),
            None => Err(format!("MCP server '{name}' not found or not running").into()),
        }
    }

    async fn restart(&self, params: Value) -> ServiceResult {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'name' parameter".to_string())?;
        self.refresh_manager_env_overrides().await;

        self.manager
            .restart_server(name)
            .await
            .map_err(ServiceError::message)?;

        self.sync_tools_if_ready().await;

        Ok(serde_json::json!({ "ok": true }))
    }

    async fn update(&self, params: Value) -> ServiceResult {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'name' parameter".to_string())?;
        let existing = self
            .manager
            .registry_snapshot()
            .await
            .servers
            .get(name)
            .cloned()
            .ok_or_else(|| format!("MCP server '{name}' not found"))?;
        let config = parse_server_config(&params, Some(&existing))?;
        self.refresh_manager_env_overrides().await;

        self.manager
            .update_server(name, config)
            .await
            .map_err(ServiceError::message)?;

        self.sync_tools_if_ready().await;

        Ok(serde_json::json!({ "ok": true }))
    }

    async fn reauth(&self, params: Value) -> ServiceResult {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'name' parameter".to_string())?;
        let redirect_uri = params
            .get("redirectUri")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| "missing 'redirectUri' parameter".to_string())?;
        self.refresh_manager_env_overrides().await;

        let auth_url = self
            .manager
            .reauth_server(name, redirect_uri)
            .await
            .map_err(ServiceError::message)?;

        Ok(serde_json::json!({
            "ok": true,
            "oauthPending": true,
            "authUrl": auth_url
        }))
    }

    async fn oauth_start(&self, params: Value) -> ServiceResult {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'name' parameter".to_string())?;
        let redirect_uri = params
            .get("redirectUri")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| "missing 'redirectUri' parameter".to_string())?;
        self.refresh_manager_env_overrides().await;

        let auth_url = self
            .manager
            .oauth_start_server(name, redirect_uri)
            .await
            .map_err(ServiceError::message)?;

        Ok(serde_json::json!({
            "ok": true,
            "oauthPending": true,
            "authUrl": auth_url
        }))
    }

    async fn oauth_complete(&self, params: Value) -> ServiceResult {
        let state = params
            .get("state")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'state' parameter".to_string())?;
        let code = params
            .get("code")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'code' parameter".to_string())?;
        self.refresh_manager_env_overrides().await;

        let server_name = self
            .manager
            .oauth_complete_callback(state, code)
            .await
            .map_err(ServiceError::message)?;

        self.sync_tools_if_ready().await;

        Ok(serde_json::json!({
            "ok": true,
            "name": server_name
        }))
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        moltis_mcp::McpRegistry,
        secrecy::{ExposeSecret, Secret},
    };

    #[test]
    fn parse_server_config_allows_sse_without_command() {
        let cfg = parse_server_config(
            &serde_json::json!({
                "transport": "sse",
                "url": "https://mcp.linear.app/mcp",
                "enabled": true
            }),
            None,
        );
        assert!(
            cfg.is_ok(),
            "expected SSE config to parse without command, got: {cfg:?}"
        );
        let Ok(cfg) = cfg else {
            panic!("SSE config unexpectedly failed to parse");
        };

        assert!(matches!(cfg.transport, moltis_mcp::TransportType::Sse));
        assert_eq!(cfg.command, "");
        assert_eq!(
            cfg.url
                .as_ref()
                .map(ExposeSecret::expose_secret)
                .map(String::as_str),
            Some("https://mcp.linear.app/mcp")
        );
    }

    #[test]
    fn parse_server_config_requires_command_for_stdio() {
        let err = parse_server_config(
            &serde_json::json!({
                "transport": "stdio",
                "args": ["-y", "@modelcontextprotocol/server-filesystem"]
            }),
            None,
        )
        .err();

        assert_eq!(
            err.as_ref().map(ToString::to_string).as_deref(),
            Some("missing 'command' parameter")
        );
    }

    #[test]
    fn parse_server_config_requires_url_for_sse() {
        let err = parse_server_config(
            &serde_json::json!({
                "transport": "sse",
            }),
            None,
        )
        .err();

        assert_eq!(
            err.as_ref().map(ToString::to_string).as_deref(),
            Some("missing 'url' parameter for 'sse' transport")
        );
    }

    #[test]
    fn parse_server_config_update_preserves_existing_sse_fields() {
        let existing = moltis_mcp::McpServerConfig {
            transport: moltis_mcp::TransportType::Sse,
            url: Some(Secret::new("https://mcp.linear.app/mcp".to_string())),
            ..Default::default()
        };

        let cfg = parse_server_config(
            &serde_json::json!({
                "enabled": false
            }),
            Some(&existing),
        );
        assert!(
            cfg.is_ok(),
            "expected parser to preserve SSE defaults from existing config, got: {cfg:?}"
        );
        let Ok(cfg) = cfg else {
            panic!("failed to parse update with inherited SSE config");
        };

        assert!(matches!(cfg.transport, moltis_mcp::TransportType::Sse));
        assert_eq!(
            cfg.url
                .as_ref()
                .map(ExposeSecret::expose_secret)
                .map(String::as_str),
            Some("https://mcp.linear.app/mcp")
        );
        assert!(!cfg.enabled);
    }

    #[test]
    fn parse_server_config_update_preserves_oauth_when_omitted() {
        let existing = moltis_mcp::McpServerConfig {
            transport: moltis_mcp::TransportType::Sse,
            url: Some(Secret::new("https://mcp.linear.app/mcp".to_string())),
            oauth: Some(moltis_mcp::McpOAuthConfig {
                client_id: "linear-client".to_string(),
                auth_url: "https://linear.app/oauth/authorize".to_string(),
                token_url: "https://api.linear.app/oauth/token".to_string(),
                scopes: vec!["read".to_string(), "write".to_string()],
            }),
            ..Default::default()
        };

        let cfg = parse_server_config(
            &serde_json::json!({
                "transport": "sse"
            }),
            Some(&existing),
        );
        assert!(
            cfg.is_ok(),
            "expected parser to preserve existing oauth fields, got: {cfg:?}"
        );
        let Ok(cfg) = cfg else {
            panic!("failed to parse update while preserving oauth");
        };

        assert!(cfg.oauth.is_some(), "expected oauth to be preserved");
        let Some(oauth) = cfg.oauth else {
            panic!("oauth missing after parse");
        };
        assert_eq!(oauth.client_id, "linear-client");
        assert_eq!(oauth.auth_url, "https://linear.app/oauth/authorize");
        assert_eq!(oauth.token_url, "https://api.linear.app/oauth/token");
        assert_eq!(oauth.scopes, vec!["read".to_string(), "write".to_string()]);
    }

    #[test]
    fn parse_server_config_preserves_and_replaces_sse_headers() {
        let existing = moltis_mcp::McpServerConfig {
            transport: moltis_mcp::TransportType::Sse,
            url: Some(Secret::new("https://mcp.linear.app/mcp".to_string())),
            headers: HashMap::from([(
                "Authorization".to_string(),
                Secret::new("Bearer old-secret".to_string()),
            )]),
            ..Default::default()
        };

        let preserved = parse_server_config(
            &serde_json::json!({
                "transport": "sse"
            }),
            Some(&existing),
        );
        assert!(
            preserved.is_ok(),
            "expected header preservation, got: {preserved:?}"
        );
        let Ok(preserved) = preserved else {
            panic!("header preservation unexpectedly failed");
        };
        assert_eq!(preserved.headers.len(), 1);
        assert_eq!(
            preserved
                .headers
                .get("Authorization")
                .map(ExposeSecret::expose_secret)
                .map(String::as_str),
            Some("Bearer old-secret")
        );

        let replaced = parse_server_config(
            &serde_json::json!({
                "transport": "sse",
                "headers": {
                    "X-Workspace": "team-alpha"
                }
            }),
            Some(&existing),
        );
        assert!(
            replaced.is_ok(),
            "expected header replacement, got: {replaced:?}"
        );
        let Ok(replaced) = replaced else {
            panic!("header replacement unexpectedly failed");
        };
        assert_eq!(replaced.headers.len(), 1);
        assert_eq!(
            replaced
                .headers
                .get("X-Workspace")
                .map(ExposeSecret::expose_secret)
                .map(String::as_str),
            Some("team-alpha")
        );
    }

    #[test]
    fn parse_server_config_allows_clearing_sse_headers() {
        let existing = moltis_mcp::McpServerConfig {
            transport: moltis_mcp::TransportType::Sse,
            url: Some(Secret::new("https://mcp.linear.app/mcp".to_string())),
            headers: HashMap::from([(
                "Authorization".to_string(),
                Secret::new("Bearer old-secret".to_string()),
            )]),
            ..Default::default()
        };

        let cleared = parse_server_config(
            &serde_json::json!({
                "transport": "sse",
                "headers": {}
            }),
            Some(&existing),
        );
        assert!(
            cleared.is_ok(),
            "expected header clearing, got: {cleared:?}"
        );
        let Ok(cleared) = cleared else {
            panic!("header clearing unexpectedly failed");
        };
        assert!(cleared.headers.is_empty());
    }

    #[test]
    fn merge_env_overrides_keeps_config_values_authoritative() {
        let base = HashMap::from([
            ("OPENAI_API_KEY".to_string(), "config-openai".to_string()),
            ("BRAVE_API_KEY".to_string(), "config-brave".to_string()),
        ]);

        let merged = merge_env_overrides(&base, vec![
            ("OPENAI_API_KEY".to_string(), "ui-openai".to_string()),
            (
                "PERPLEXITY_API_KEY".to_string(),
                "ui-perplexity".to_string(),
            ),
        ]);

        assert_eq!(
            merged.get("OPENAI_API_KEY").map(String::as_str),
            Some("config-openai")
        );
        assert_eq!(
            merged.get("PERPLEXITY_API_KEY").map(String::as_str),
            Some("ui-perplexity")
        );
        assert_eq!(
            merged.get("BRAVE_API_KEY").map(String::as_str),
            Some("config-brave")
        );
    }

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

        fn parameters_schema(&self) -> Value {
            serde_json::json!({})
        }

        async fn execute(&self, _params: Value) -> Result<Value> {
            Ok(serde_json::json!({}))
        }
    }
}
