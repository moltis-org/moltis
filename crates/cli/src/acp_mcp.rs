use std::{collections::HashMap, sync::Arc};

use {
    agent_client_protocol as acp,
    moltis_agents::tool_registry::ToolRegistry,
    moltis_mcp::{McpManager, McpRegistry, McpServerConfig, StdioLaunchOptions, TransportType},
    secrecy::Secret,
    tokio::sync::RwLock,
};

use moltis_acp::SessionSetup;

pub struct SessionMcpRuntime {
    manager: Arc<McpManager>,
    tools: Arc<RwLock<ToolRegistry>>,
}

impl SessionMcpRuntime {
    pub async fn start(setup: &SessionSetup) -> anyhow::Result<Option<Self>> {
        if setup.mcp_servers().is_empty() {
            return Ok(None);
        }

        let manager = Arc::new(McpManager::new(McpRegistry::new()));
        let options = StdioLaunchOptions {
            current_dir: Some(setup.cwd().to_path_buf()),
            inherit_parent_env: false,
        };
        for server in setup.mcp_servers() {
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
            if let Err(error) = manager
                .start_server_with_options(&server.name, &config, &options)
                .await
            {
                manager.shutdown_all().await;
                return Err(error.into());
            }
        }

        let tools = Arc::new(RwLock::new(ToolRegistry::new()));
        moltis_mcp_agent_bridge::sync_mcp_tools(&manager, &tools).await;
        Ok(Some(Self { manager, tools }))
    }

    pub fn tools(&self) -> Arc<RwLock<ToolRegistry>> {
        Arc::clone(&self.tools)
    }

    pub async fn shutdown(self) {
        self.manager.shutdown_all().await;
    }
}
