//! MCP client: manages the protocol handshake and tool interactions with a single MCP server.

use std::{collections::HashMap, sync::Arc};

use {
    anyhow::{Context, Result},
    tracing::{debug, info, warn},
};

#[cfg(feature = "metrics")]
use std::time::Instant;

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, gauge, histogram, labels, mcp as mcp_metrics};

use crate::{
    sse_transport::SseTransport,
    traits::{McpClientTrait, McpTransport},
    transport::StdioTransport,
    types::{
        ClientCapabilities, ClientInfo, InitializeParams, InitializeResult, McpToolDef,
        PROTOCOL_VERSION, ToolsCallParams, ToolsCallResult, ToolsListResult,
    },
};

/// State of an MCP client connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpClientState {
    /// Transport spawned, not yet initialized.
    Connected,
    /// `initialize` completed, `initialized` notification sent.
    Ready,
    /// Server process exited or was shut down.
    Closed,
}

/// An MCP client connected to a single server via stdio.
pub struct McpClient {
    server_name: String,
    transport: Arc<dyn McpTransport>,
    state: McpClientState,
    server_info: Option<InitializeResult>,
    tools: Vec<McpToolDef>,
}

impl McpClient {
    /// Spawn the server process and perform the MCP handshake (initialize + initialized).
    pub async fn connect(
        server_name: &str,
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<Self> {
        info!(server = %server_name, command = %command, args = ?args, "connecting to MCP server");
        let transport = StdioTransport::spawn(command, args, env).await?;

        let mut client = Self {
            server_name: server_name.into(),
            transport,
            state: McpClientState::Connected,
            server_info: None,
            tools: Vec::new(),
        };

        if let Err(e) = client.initialize().await {
            warn!(server = %server_name, error = %e, "MCP initialize handshake failed");
            return Err(e);
        }

        // Track MCP server connection
        #[cfg(feature = "metrics")]
        {
            counter!(mcp_metrics::SERVER_CONNECTIONS_TOTAL, labels::SERVER => server_name.to_string())
                .increment(1);
            gauge!(mcp_metrics::SERVERS_CONNECTED).increment(1.0);
        }

        Ok(client)
    }

    /// Connect to a remote MCP server over HTTP/SSE.
    pub async fn connect_sse(server_name: &str, url: &str) -> Result<Self> {
        info!(server = %server_name, url = %url, "connecting to MCP server via SSE");
        let transport = SseTransport::new(url)?;

        let mut client = Self {
            server_name: server_name.into(),
            transport,
            state: McpClientState::Connected,
            server_info: None,
            tools: Vec::new(),
        };

        if let Err(e) = client.initialize().await {
            warn!(server = %server_name, error = %e, "MCP SSE initialize handshake failed");
            return Err(e);
        }
        Ok(client)
    }

    async fn initialize(&mut self) -> Result<()> {
        let params = InitializeParams {
            protocol_version: PROTOCOL_VERSION.into(),
            capabilities: ClientCapabilities::default(),
            client_info: ClientInfo {
                name: "moltis".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
        };

        let resp = self
            .transport
            .request("initialize", Some(serde_json::to_value(&params)?))
            .await
            .context("MCP initialize request failed")?;

        let result: InitializeResult =
            serde_json::from_value(resp.result.context("MCP initialize returned no result")?)
                .context("failed to parse MCP initialize result")?;

        info!(
            server = %self.server_name,
            protocol = %result.protocol_version,
            server_name = %result.server_info.name,
            "MCP server initialized"
        );

        self.server_info = Some(result);

        // Send `initialized` notification to complete handshake.
        self.transport
            .notify("notifications/initialized", None)
            .await?;
        self.state = McpClientState::Ready;

        Ok(())
    }

    fn ensure_ready(&self) -> Result<()> {
        if self.state != McpClientState::Ready {
            anyhow::bail!(
                "MCP client for '{}' is not ready (state: {:?})",
                self.server_name,
                self.state
            );
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl McpClientTrait for McpClient {
    fn server_name(&self) -> &str {
        &self.server_name
    }

    fn state(&self) -> McpClientState {
        self.state
    }

    fn tools(&self) -> &[McpToolDef] {
        &self.tools
    }

    async fn list_tools(&mut self) -> Result<&[McpToolDef]> {
        self.ensure_ready()?;

        let resp = self.transport.request("tools/list", None).await?;
        let result: ToolsListResult =
            serde_json::from_value(resp.result.context("tools/list returned no result")?)?;

        debug!(
            server = %self.server_name,
            count = result.tools.len(),
            "fetched MCP tools"
        );

        self.tools = result.tools;
        Ok(&self.tools)
    }

    async fn call_tool(&self, name: &str, arguments: serde_json::Value) -> Result<ToolsCallResult> {
        self.ensure_ready()?;

        #[cfg(feature = "metrics")]
        let start = Instant::now();

        let params = ToolsCallParams {
            name: name.into(),
            arguments,
        };

        let result = self
            .transport
            .request("tools/call", Some(serde_json::to_value(&params)?))
            .await;

        match result {
            Ok(resp) => {
                let result: ToolsCallResult =
                    serde_json::from_value(resp.result.context("tools/call returned no result")?)?;

                #[cfg(feature = "metrics")]
                {
                    let duration = start.elapsed().as_secs_f64();

                    counter!(
                        mcp_metrics::TOOL_CALLS_TOTAL,
                        labels::SERVER => self.server_name.clone(),
                        labels::TOOL => name.to_string()
                    )
                    .increment(1);

                    histogram!(
                        mcp_metrics::TOOL_CALL_DURATION_SECONDS,
                        labels::SERVER => self.server_name.clone(),
                        labels::TOOL => name.to_string()
                    )
                    .record(duration);
                }

                Ok(result)
            },
            Err(e) => {
                #[cfg(feature = "metrics")]
                {
                    counter!(
                        mcp_metrics::TOOL_CALL_ERRORS_TOTAL,
                        labels::SERVER => self.server_name.clone(),
                        labels::TOOL => name.to_string()
                    )
                    .increment(1);
                }
                Err(e)
            },
        }
    }

    async fn is_alive(&self) -> bool {
        self.transport.is_alive().await
    }

    async fn shutdown(&mut self) {
        self.state = McpClientState::Closed;
        self.transport.kill().await;

        // Decrement connected servers gauge
        #[cfg(feature = "metrics")]
        {
            gauge!(mcp_metrics::SERVERS_CONNECTED).decrement(1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_state_debug() {
        assert_eq!(format!("{:?}", McpClientState::Connected), "Connected");
        assert_eq!(format!("{:?}", McpClientState::Ready), "Ready");
        assert_eq!(format!("{:?}", McpClientState::Closed), "Closed");
    }
}
