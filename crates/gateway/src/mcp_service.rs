//! Live MCP service implementation backed by `McpManager`.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use {
    async_trait::async_trait,
    serde_json::Value,
    tokio::sync::{Mutex as AsyncMutex, MutexGuard, RwLock},
    tracing::{info, warn},
};

use moltis_agents::tool_registry::ToolRegistry;

use crate::services::{McpService, ServiceError, ServiceResult};

mod repositories;

const MAX_CONCURRENT_MANAGED_REPOSITORY_OPERATIONS: usize = 4;

// Re-export pure parsing functions that now live in moltis-mcp.
pub(crate) use moltis_mcp::{merge_env_overrides, parse_server_config};

// Re-export sync_mcp_tools from the dedicated bridge crate.
pub(crate) use moltis_mcp_agent_bridge::sync_mcp_tools;

// ── Config parsing helper ───────────────────────────────────────────────────

// Extract an `McpServerConfig` from JSON params.
// ── LiveMcpService ──────────────────────────────────────────────────────────

/// Live MCP service delegating to `McpManager`.
pub struct LiveMcpService {
    manager: Arc<moltis_mcp::McpManager>,
    /// Shared tool registry for syncing MCP tools into the agent loop.
    /// Set after construction via `set_tool_registry`.
    tool_registry: RwLock<Option<Arc<RwLock<ToolRegistry>>>>,
    config_env_overrides: HashMap<String, String>,
    credential_store: RwLock<Option<Arc<crate::auth::CredentialStore>>>,
    data_dir: PathBuf,
    materializer: moltis_git_repositories::Materializer,
    repository_operations: Arc<Mutex<HashSet<moltis_mcp::ManagedRepositoryId>>>,
    managed_repository_mutations: AsyncMutex<u64>,
    _repository_lock: moltis_mcp::ManagedRepositoryLock,
}

impl LiveMcpService {
    pub fn new(
        manager: Arc<moltis_mcp::McpManager>,
        config_env_overrides: HashMap<String, String>,
        credential_store: Option<Arc<crate::auth::CredentialStore>>,
        data_dir: PathBuf,
        repository_lock: moltis_mcp::ManagedRepositoryLock,
    ) -> Self {
        Self::new_with_materializer(
            manager,
            config_env_overrides,
            credential_store,
            data_dir,
            moltis_git_repositories::Materializer::default(),
            repository_lock,
        )
    }

    fn new_with_materializer(
        manager: Arc<moltis_mcp::McpManager>,
        config_env_overrides: HashMap<String, String>,
        credential_store: Option<Arc<crate::auth::CredentialStore>>,
        data_dir: PathBuf,
        materializer: moltis_git_repositories::Materializer,
        repository_lock: moltis_mcp::ManagedRepositoryLock,
    ) -> Self {
        Self {
            manager,
            tool_registry: RwLock::new(None),
            config_env_overrides,
            credential_store: RwLock::new(credential_store),
            data_dir,
            materializer,
            repository_operations: Arc::new(Mutex::new(HashSet::new())),
            managed_repository_mutations: AsyncMutex::new(0),
            _repository_lock: repository_lock,
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

    async fn begin_managed_repository_mutation(&self) -> MutexGuard<'_, u64> {
        let mut generation = self.managed_repository_mutations.lock().await;
        *generation = generation.wrapping_add(1);
        generation
    }
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
        let config =
            parse_server_config(&params, None).map_err(|e| ServiceError::message(e.to_string()))?;
        self.refresh_manager_env_overrides().await;

        let mut suffix = 1_u32;
        let (final_name, add_result) = loop {
            let candidate = if suffix == 1 {
                name.to_string()
            } else {
                format!("{name}-{suffix}")
            };
            let result = self
                .manager
                .add_server_if_absent(candidate.clone(), config.clone(), true)
                .await;
            if !matches!(result, Ok(false)) {
                break (candidate, result);
            }
            suffix = suffix.saturating_add(1);
        };

        match add_result {
            Ok(true) => {
                info!(server = %final_name, "added MCP server via API");
                self.sync_tools_if_ready().await;
                Ok(serde_json::json!({ "ok": true, "name": final_name }))
            },
            Ok(false) => Err(ServiceError::message("MCP server name allocation failed")),
            Err(moltis_mcp::Error::Manager(moltis_mcp::McpManagerError::OAuthRequired {
                ..
            })) => {
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
            },
            Err(error) => Err(ServiceError::message(error)),
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
        let config = parse_server_config(&params, Some(&existing))
            .map_err(|e| ServiceError::message(e.to_string()))?;
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

    async fn repositories_list(&self, params: Value) -> ServiceResult {
        self.repositories_list_impl(params).await
    }

    async fn repositories_preview(&self, params: Value) -> ServiceResult {
        self.repositories_preview_impl(params).await
    }

    async fn repositories_install(&self, params: Value) -> ServiceResult {
        self.repositories_install_impl(params).await
    }

    async fn repositories_update_preview(&self, params: Value) -> ServiceResult {
        self.repositories_update_preview_impl(params).await
    }

    async fn repositories_update_apply(&self, params: Value) -> ServiceResult {
        self.repositories_update_apply_impl(params).await
    }

    async fn repositories_rollback(&self, params: Value) -> ServiceResult {
        self.repositories_rollback_impl(params).await
    }

    async fn repositories_remove(&self, params: Value) -> ServiceResult {
        self.repositories_remove_impl(params).await
    }

    async fn managed_approve(&self, params: Value) -> ServiceResult {
        self.managed_approve_impl(params).await
    }

    async fn git_credentials_list(&self, params: Value) -> ServiceResult {
        self.git_credentials_list_impl(params).await
    }

    async fn git_credentials_create(&self, params: Value) -> ServiceResult {
        self.git_credentials_create_impl(params).await
    }

    async fn git_credentials_update(&self, params: Value) -> ServiceResult {
        self.git_credentials_update_impl(params).await
    }

    async fn git_credentials_remove(&self, params: Value) -> ServiceResult {
        self.git_credentials_remove_impl(params).await
    }

    async fn managed_ssh_key_remove(&self, id: i64) -> ServiceResult {
        self.managed_ssh_key_remove_impl(id).await
    }

    async fn managed_ssh_target_remove(&self, id: i64) -> ServiceResult {
        self.managed_ssh_target_remove_impl(id).await
    }

    async fn update_request_timeout(&self, request_timeout_secs: u64) -> ServiceResult {
        self.manager.set_request_timeout_secs(request_timeout_secs);
        Ok(serde_json::json!({ "ok": true }))
    }
}
