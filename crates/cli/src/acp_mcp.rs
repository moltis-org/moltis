use std::{collections::HashMap, sync::Arc};

use {
    agent_client_protocol as acp,
    moltis_agents::tool_registry::ToolRegistry,
    moltis_mcp::{McpManager, McpRegistry, McpServerConfig, StdioLaunchOptions, TransportType},
    secrecy::Secret,
    tokio::sync::RwLock,
};

use moltis_acp::{SessionKey, SessionSetup};

const MAX_PROVIDER_TOOL_NAME_BYTES: usize = 64;

pub struct SessionMcpRuntime {
    manager: Arc<McpManager>,
    tools: Arc<RwLock<ToolRegistry>>,
}

fn runtime_server_name(namespace: &str, index: usize) -> String {
    format!("acp_{namespace}_{index}")
}

fn runtime_namespace(key: &SessionKey) -> String {
    key.as_str()
        .strip_prefix("acp:")
        .unwrap_or_else(|| key.as_str())
        .bytes()
        .filter(u8::is_ascii_alphanumeric)
        .take(12)
        .map(char::from)
        .collect()
}

impl SessionMcpRuntime {
    pub async fn start(key: &SessionKey, setup: &SessionSetup) -> anyhow::Result<Option<Self>> {
        if setup.mcp_servers().is_empty() {
            return Ok(None);
        }

        let manager = Arc::new(McpManager::new(McpRegistry::new()));
        let options = StdioLaunchOptions {
            current_dir: Some(setup.cwd().to_path_buf()),
            inherit_parent_env: false,
        };
        let namespace = runtime_namespace(key);
        for (index, server) in setup.mcp_servers().iter().enumerate() {
            let acp::McpServer::Stdio(server) = server else {
                manager.shutdown_all().await;
                anyhow::bail!("only stdio MCP servers are supported");
            };
            let command = server.command.to_str().map(str::to_owned);
            let Some(command) = command else {
                manager.shutdown_all().await;
                anyhow::bail!("MCP command path is not valid UTF-8");
            };
            let config = McpServerConfig {
                command,
                args: server.args.clone(),
                env: server
                    .env
                    .iter()
                    .map(|variable| (variable.name.clone(), Secret::new(variable.value.clone())))
                    .collect::<HashMap<_, _>>(),
                enabled: true,
                transport: TransportType::Stdio,
                ..McpServerConfig::default()
            };
            // Client MCP identities must not collide with configured servers or
            // satisfy an agent preset's allowlist by borrowing a trusted name.
            let runtime_name = runtime_server_name(&namespace, index);
            if let Err(error) = manager
                .start_server_with_options(&runtime_name, &config, &options)
                .await
            {
                manager.shutdown_all().await;
                return Err(error.into());
            }
        }

        let tools = Arc::new(RwLock::new(ToolRegistry::new()));
        moltis_mcp_agent_bridge::sync_mcp_tools(&manager, &tools).await;
        let oversized_tool = {
            let registry = tools.read().await;
            registry.list_schemas().into_iter().find_map(|schema| {
                schema
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .filter(|name| name.len() > MAX_PROVIDER_TOOL_NAME_BYTES)
                    .map(str::to_owned)
            })
        };
        if let Some(name) = oversized_tool {
            manager.shutdown_all().await;
            anyhow::bail!(
                "client MCP tool name exceeds {MAX_PROVIDER_TOOL_NAME_BYTES} bytes: {name}"
            );
        }
        Ok(Some(Self { manager, tools }))
    }

    pub fn tools(&self) -> Arc<RwLock<ToolRegistry>> {
        Arc::clone(&self.tools)
    }

    pub async fn shutdown(self) {
        self.manager.shutdown_all().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_mcp_servers_receive_session_local_names() {
        let first_key = SessionKey::new("acp:01234567-89ab-cdef-0123-456789abcdef");
        let second_key = SessionKey::new("acp:abcdef01-2345-6789-abcd-ef0123456789");
        let first_namespace = runtime_namespace(&first_key);
        assert_eq!(first_namespace, runtime_namespace(&first_key));
        let first = runtime_server_name(&first_namespace, 0);
        let second = runtime_server_name(&runtime_namespace(&second_key), 0);
        assert_eq!(first, "acp_0123456789ab_0");
        assert_ne!(first, second);
        assert_ne!(first, "github");
        assert!(first.len() <= 18);
    }
}
