//! MCP client: manages the protocol handshake and tool interactions with a single MCP server.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use {
    secrecy::Secret,
    tokio::sync::RwLock,
    tracing::{debug, info, warn},
};

#[cfg(feature = "metrics")]
use std::time::Instant;

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, gauge, histogram, labels, mcp as mcp_metrics};

use crate::{
    auth::SharedAuthProvider,
    error::{Context, Error, Result},
    legacy_sse_transport::LegacySseTransport,
    remote::ResolvedRemoteConfig,
    sse_transport::SseTransport,
    traits::{McpClientTrait, McpTransport},
    transport::{StdioLaunchOptions, StdioTransport},
    types::{
        ClientCapabilities, ClientInfo, InitializeParams, InitializeResult, JsonRpcResponse,
        McpToolDef, McpTransportError, PROTOCOL_VERSION, ToolsCallParams, ToolsCallResult,
        ToolsListResult,
    },
};

/// State of an MCP client connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpClientState {
    /// Transport spawned, not yet initialized.
    Connected,
    /// `initialize` completed, `initialized` notification sent.
    Ready,
    /// OAuth authentication in progress (waiting for browser).
    Authenticating,
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
    session_recovery: SessionRecovery,
}

struct SessionRecovery {
    gate: RwLock<()>,
    generation: AtomicU64,
    last_attempt: AtomicU64,
}

impl Default for SessionRecovery {
    fn default() -> Self {
        Self {
            gate: RwLock::new(()),
            generation: AtomicU64::new(0),
            last_attempt: AtomicU64::new(u64::MAX),
        }
    }
}

impl McpClient {
    /// Spawn the server process and perform the MCP handshake (initialize + initialized).
    pub async fn connect(
        server_name: &str,
        command: &str,
        args: &[String],
        env: &HashMap<String, Secret<String>>,
        request_timeout: Duration,
    ) -> Result<Self> {
        Self::connect_with_options(
            server_name,
            command,
            args,
            env,
            request_timeout,
            &StdioLaunchOptions::default(),
        )
        .await
    }

    pub async fn connect_with_options(
        server_name: &str,
        command: &str,
        args: &[String],
        env: &HashMap<String, Secret<String>>,
        request_timeout: Duration,
        options: &StdioLaunchOptions,
    ) -> Result<Self> {
        info!(server = %server_name, command = %command, arg_count = args.len(), "connecting to MCP server");
        let transport =
            StdioTransport::spawn_with_options(command, args, env, request_timeout, options)
                .await?;

        let mut client = Self {
            server_name: server_name.into(),
            transport,
            state: McpClientState::Connected,
            server_info: None,
            tools: Vec::new(),
            session_recovery: SessionRecovery::default(),
        };

        if let Err(e) = client.initialize().await {
            warn!(server = %server_name, "MCP initialize handshake failed");
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

    /// Connect to a remote MCP server over HTTP/SSE or Streamable HTTP.
    pub async fn connect_sse(
        server_name: &str,
        remote: &ResolvedRemoteConfig,
        request_timeout: Duration,
    ) -> Result<Self> {
        info!(
            server = %server_name,
            url = %remote.display_url(),
            "connecting to remote MCP server"
        );
        let transport = SseTransport::new_with_remote(remote.clone(), request_timeout)?;

        let mut client = Self {
            server_name: server_name.into(),
            transport,
            state: McpClientState::Connected,
            server_info: None,
            tools: Vec::new(),
            session_recovery: SessionRecovery::default(),
        };

        if let Err(e) = client.initialize().await {
            warn!(server = %server_name, "remote MCP initialize handshake failed");
            return Err(e);
        }
        Ok(client)
    }

    /// Connect to a remote MCP server using the legacy SSE transport.
    ///
    /// The legacy SSE protocol first GETs the SSE endpoint to discover the
    /// message URL (with sessionId), then POSTs JSON-RPC messages to it.
    pub async fn connect_legacy_sse(
        server_name: &str,
        remote: &ResolvedRemoteConfig,
        request_timeout: Duration,
    ) -> Result<Self> {
        info!(
            server = %server_name,
            url = %remote.display_url(),
            "connecting to MCP server via legacy SSE"
        );
        let transport = LegacySseTransport::new_with_remote(remote.clone(), request_timeout)?;

        let mut client = Self {
            server_name: server_name.into(),
            transport,
            state: McpClientState::Connected,
            server_info: None,
            tools: Vec::new(),
            session_recovery: SessionRecovery::default(),
        };

        if let Err(e) = client.initialize().await {
            warn!(server = %server_name, "legacy SSE MCP initialize handshake failed");
            return Err(e);
        }

        #[cfg(feature = "metrics")]
        {
            counter!(mcp_metrics::SERVER_CONNECTIONS_TOTAL, labels::SERVER => server_name.to_string())
                .increment(1);
            gauge!(mcp_metrics::SERVERS_CONNECTED).increment(1.0);
        }

        Ok(client)
    }

    /// Connect to a remote MCP server using the legacy SSE transport with an OAuth auth provider.
    pub async fn connect_legacy_sse_with_auth(
        server_name: &str,
        remote: &ResolvedRemoteConfig,
        auth: SharedAuthProvider,
        request_timeout: Duration,
    ) -> Result<Self> {
        info!(
            server = %server_name,
            url = %remote.display_url(),
            "connecting to MCP server via legacy SSE (with auth)"
        );
        let transport =
            LegacySseTransport::with_auth_remote(remote.clone(), auth, request_timeout)?;

        let mut client = Self {
            server_name: server_name.into(),
            transport,
            state: McpClientState::Connected,
            server_info: None,
            tools: Vec::new(),
            session_recovery: SessionRecovery::default(),
        };

        if let Err(e) = client.initialize().await {
            warn!(server = %server_name, "legacy SSE (auth) MCP initialize handshake failed");
            return Err(e);
        }

        #[cfg(feature = "metrics")]
        {
            counter!(mcp_metrics::SERVER_CONNECTIONS_TOTAL, labels::SERVER => server_name.to_string())
                .increment(1);
            gauge!(mcp_metrics::SERVERS_CONNECTED).increment(1.0);
        }

        Ok(client)
    }

    /// Connect to a remote MCP server over HTTP/SSE or Streamable HTTP with an OAuth auth provider.
    pub async fn connect_sse_with_auth(
        server_name: &str,
        remote: &ResolvedRemoteConfig,
        auth: SharedAuthProvider,
        request_timeout: Duration,
    ) -> Result<Self> {
        info!(
            server = %server_name,
            url = %remote.display_url(),
            "connecting to remote MCP server (with auth)"
        );
        let transport = SseTransport::with_auth_remote(remote.clone(), auth, request_timeout)?;

        let mut client = Self {
            server_name: server_name.into(),
            transport,
            state: McpClientState::Connected,
            server_info: None,
            tools: Vec::new(),
            session_recovery: SessionRecovery::default(),
        };

        if let Err(e) = client.initialize().await {
            warn!(server = %server_name, "MCP SSE (auth) initialize handshake failed");
            return Err(e);
        }

        #[cfg(feature = "metrics")]
        {
            counter!(mcp_metrics::SERVER_CONNECTIONS_TOTAL, labels::SERVER => server_name.to_string())
                .increment(1);
            gauge!(mcp_metrics::SERVERS_CONNECTED).increment(1.0);
        }

        Ok(client)
    }

    async fn initialize(&mut self) -> Result<()> {
        let result = self.initialize_transport().await?;
        self.server_info = Some(result);

        // Send `initialized` notification to complete handshake.
        self.transport
            .notify("notifications/initialized", None)
            .await?;
        self.state = McpClientState::Ready;

        Ok(())
    }

    async fn initialize_transport(&self) -> Result<InitializeResult> {
        let params = InitializeParams {
            protocol_version: PROTOCOL_VERSION.into(),
            capabilities: ClientCapabilities::default(),
            client_info: ClientInfo {
                name: "moltis".into(),
                version: moltis_config::VERSION.into(),
            },
        };

        let resp = match self
            .transport
            .request("initialize", Some(serde_json::to_value(&params)?))
            .await
        {
            Ok(r) => r,
            // Preserve 401 so the manager can trigger the OAuth re-auth flow.
            Err(e @ Error::Transport(McpTransportError::Unauthorized { .. })) => return Err(e),
            Err(e) => {
                return Err(Error::message(format!(
                    "MCP initialize request failed: {e}"
                )));
            },
        };

        let result: InitializeResult =
            serde_json::from_value(resp.result.context("MCP initialize returned no result")?)
                .context("failed to parse MCP initialize result")?;

        info!(
            server = %self.server_name,
            "MCP server initialized"
        );

        Ok(result)
    }

    async fn request_with_session_recovery(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<JsonRpcResponse> {
        let generation;
        let result;
        {
            let _gate = self.session_recovery.gate.read().await;
            generation = self.session_recovery.generation.load(Ordering::Acquire);
            result = self.transport.request(method, params.clone()).await;
        }

        match result {
            Err(Error::Transport(McpTransportError::SessionLost)) => {
                self.recover_session(generation).await?;
                let _gate = self.session_recovery.gate.read().await;
                self.transport.request(method, params).await
            },
            result => result,
        }
    }

    async fn recover_session(&self, generation: u64) -> Result<()> {
        let _gate = self.session_recovery.gate.write().await;
        if self.session_recovery.generation.load(Ordering::Acquire) != generation {
            return Ok(());
        }
        if self.session_recovery.last_attempt.load(Ordering::Acquire) == generation {
            return Ok(());
        }

        self.session_recovery
            .last_attempt
            .store(generation, Ordering::Release);
        self.initialize_transport().await?;
        self.transport
            .notify("notifications/initialized", None)
            .await?;
        self.session_recovery
            .generation
            .store(generation.wrapping_add(1), Ordering::Release);

        Ok(())
    }

    fn ensure_ready(&self) -> Result<()> {
        if self.state != McpClientState::Ready {
            return Err(Error::message(format!(
                "MCP client for '{}' is not ready (state: {:?})",
                self.server_name, self.state
            )));
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

        let resp = self
            .request_with_session_recovery("tools/list", None)
            .await?;
        let mut result: ToolsListResult =
            serde_json::from_value(resp.result.context("tools/list returned no result")?)?;
        result
            .tools
            .sort_by(|left, right| left.name.cmp(&right.name));

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
            .request_with_session_recovery("tools/call", Some(serde_json::to_value(&params)?))
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
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    use super::*;

    use {
        async_trait::async_trait,
        serde_json::{Value, json},
    };

    use crate::{error::Result, traits::McpTransport, types::JsonRpcResponse};

    struct StubTransport {
        result: Value,
    }

    struct SessionRecoveryTransport {
        initialize_calls: AtomicUsize,
        tool_calls: AtomicUsize,
        initial_losses: AtomicUsize,
        session_ready: AtomicBool,
        recovery_succeeds: bool,
        initial_loss_barrier: tokio::sync::Barrier,
    }

    impl SessionRecoveryTransport {
        fn new(recovery_barrier: usize, recovery_succeeds: bool) -> Self {
            Self {
                initialize_calls: AtomicUsize::new(0),
                tool_calls: AtomicUsize::new(0),
                initial_losses: AtomicUsize::new(0),
                session_ready: AtomicBool::new(false),
                recovery_succeeds,
                initial_loss_barrier: tokio::sync::Barrier::new(recovery_barrier),
            }
        }

        fn response(result: Value) -> JsonRpcResponse {
            JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id: json!(1),
                result: Some(result),
                error: None,
            }
        }
    }

    #[async_trait]
    impl McpTransport for SessionRecoveryTransport {
        async fn request(&self, method: &str, _params: Option<Value>) -> Result<JsonRpcResponse> {
            match method {
                "initialize" => {
                    self.initialize_calls.fetch_add(1, Ordering::SeqCst);
                    Ok(Self::response(json!({
                        "protocolVersion": PROTOCOL_VERSION,
                        "capabilities": {},
                        "serverInfo": {"name": "test"}
                    })))
                },
                "tools/call" => {
                    self.tool_calls.fetch_add(1, Ordering::SeqCst);
                    if !self.session_ready.load(Ordering::SeqCst) {
                        let loss = self.initial_losses.fetch_add(1, Ordering::SeqCst);
                        if loss < 2 {
                            self.initial_loss_barrier.wait().await;
                        }
                        Err(McpTransportError::SessionLost.into())
                    } else {
                        Ok(Self::response(json!({
                            "content": [],
                            "isError": false
                        })))
                    }
                },
                _ => Ok(Self::response(json!({}))),
            }
        }

        async fn notify(&self, method: &str, _params: Option<Value>) -> Result<()> {
            if method == "notifications/initialized" {
                self.session_ready
                    .store(self.recovery_succeeds, Ordering::SeqCst);
            }
            Ok(())
        }

        async fn is_alive(&self) -> bool {
            true
        }

        async fn kill(&self) {}
    }

    fn session_recovery_client(transport: Arc<SessionRecoveryTransport>) -> McpClient {
        McpClient {
            server_name: "test".into(),
            transport,
            state: McpClientState::Ready,
            server_info: None,
            tools: Vec::new(),
            session_recovery: SessionRecovery::default(),
        }
    }

    #[async_trait]
    impl McpTransport for StubTransport {
        async fn request(&self, _method: &str, _params: Option<Value>) -> Result<JsonRpcResponse> {
            Ok(JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id: json!(1),
                result: Some(self.result.clone()),
                error: None,
            })
        }

        async fn notify(&self, _method: &str, _params: Option<Value>) -> Result<()> {
            Ok(())
        }

        async fn is_alive(&self) -> bool {
            true
        }

        async fn kill(&self) {}
    }

    #[test]
    fn test_client_state_debug() {
        assert_eq!(format!("{:?}", McpClientState::Connected), "Connected");
        assert_eq!(format!("{:?}", McpClientState::Ready), "Ready");
        assert_eq!(
            format!("{:?}", McpClientState::Authenticating),
            "Authenticating"
        );
        assert_eq!(format!("{:?}", McpClientState::Closed), "Closed");
    }

    #[tokio::test]
    async fn test_list_tools_sorts_by_name() {
        let transport = Arc::new(StubTransport {
            result: json!({
                "tools": [
                    {"name": "zeta", "description": "Z", "inputSchema": {"type": "object"}},
                    {"name": "alpha", "description": "A", "inputSchema": {"type": "object"}},
                    {"name": "mu", "description": "M", "inputSchema": {"type": "object"}}
                ]
            }),
        });
        let mut client = McpClient {
            server_name: "test".into(),
            transport,
            state: McpClientState::Ready,
            server_info: None,
            tools: Vec::new(),
            session_recovery: SessionRecovery::default(),
        };

        let tools = client.list_tools().await.unwrap();
        let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "mu", "zeta"]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_concurrent_session_loss_reinitializes_once() {
        let transport = Arc::new(SessionRecoveryTransport::new(2, true));
        let client = Arc::new(RwLock::new(session_recovery_client(Arc::clone(&transport))));

        let first = {
            let client = Arc::clone(&client);
            tokio::spawn(async move { client.read().await.call_tool("first", json!({})).await })
        };
        let second = {
            let client = Arc::clone(&client);
            tokio::spawn(async move { client.read().await.call_tool("second", json!({})).await })
        };

        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();

        assert_eq!(transport.initialize_calls.load(Ordering::SeqCst), 1);
        assert_eq!(transport.tool_calls.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn test_session_recovery_failure_is_attempted_once() {
        let transport = Arc::new(SessionRecoveryTransport::new(1, false));
        let client = session_recovery_client(Arc::clone(&transport));

        let result = client.call_tool("test", json!({})).await;

        assert!(matches!(
            result,
            Err(Error::Transport(McpTransportError::SessionLost))
        ));
        assert_eq!(transport.initialize_calls.load(Ordering::SeqCst), 1);
        assert_eq!(transport.tool_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_session_recovery_state_is_isolated_per_client() {
        let first_transport = Arc::new(SessionRecoveryTransport::new(1, true));
        let second_transport = Arc::new(SessionRecoveryTransport::new(1, true));
        let first = session_recovery_client(Arc::clone(&first_transport));
        let second = session_recovery_client(Arc::clone(&second_transport));

        let (first_result, second_result) = tokio::join!(
            first.call_tool("first", json!({})),
            second.call_tool("second", json!({})),
        );

        first_result.unwrap();
        second_result.unwrap();
        assert_eq!(first_transport.initialize_calls.load(Ordering::SeqCst), 1);
        assert_eq!(second_transport.initialize_calls.load(Ordering::SeqCst), 1);
    }
}
