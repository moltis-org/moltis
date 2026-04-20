//! Legacy SSE transport for MCP servers (deprecated 2024-11-05 spec).
//!
//! Some MCP servers (Baserow, NocoDB, lox-mcp) implement the legacy SSE
//! transport where:
//! 1. Client GETs the SSE endpoint to receive `event: endpoint` with a message URL
//! 2. Client POSTs JSON-RPC messages to that discovered URL
//!
//! This differs from the newer "Streamable HTTP" transport where the client
//! POSTs directly to the configured URL.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use {
    reqwest::{Client, header::HeaderMap},
    secrecy::{ExposeSecret, Secret},
    tokio::sync::RwLock,
    tracing::{debug, info, warn},
    url::Url,
};

use crate::{
    auth::SharedAuthProvider,
    error::{Context, Error, Result},
    remote::{ResolvedRemoteConfig, sanitize_reqwest_error},
    traits::McpTransport,
    types::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, McpTransportError},
};

/// Timeout for the initial SSE endpoint discovery GET request.
const SSE_ENDPOINT_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(30);

/// Legacy SSE transport for MCP servers.
///
/// Connects via GET to discover the message endpoint, then POSTs JSON-RPC
/// messages to that endpoint.
pub struct LegacySseTransport {
    client: Client,
    /// The SSE endpoint URL (where we GET to discover the message URL).
    sse_url: Secret<String>,
    display_url: String,
    default_headers: HeaderMap,
    next_id: AtomicU64,
    auth: Option<SharedAuthProvider>,
    /// The discovered message endpoint URL (populated after SSE handshake).
    message_url: RwLock<Option<String>>,
    request_timeout: Duration,
}

impl LegacySseTransport {
    pub fn new(url: &str) -> Result<Arc<Self>> {
        Self::new_with_timeout(url, Duration::from_secs(60))
    }

    pub fn new_with_timeout(url: &str, request_timeout: Duration) -> Result<Arc<Self>> {
        let remote = ResolvedRemoteConfig::from_server_config(
            &crate::registry::McpServerConfig {
                transport: crate::registry::TransportType::Sse,
                url: Some(Secret::new(url.to_string())),
                ..Default::default()
            },
            &std::collections::HashMap::new(),
        )?;
        Self::new_with_remote(remote, request_timeout)
    }

    pub fn new_with_remote(
        remote: ResolvedRemoteConfig,
        request_timeout: Duration,
    ) -> Result<Arc<Self>> {
        let client = Client::builder()
            .timeout(request_timeout)
            .build()
            .context("failed to build HTTP client for legacy SSE transport")?;

        Ok(Arc::new(Self {
            client,
            sse_url: Secret::new(remote.request_url().to_string()),
            display_url: remote.display_url().to_string(),
            default_headers: remote.headers().clone(),
            next_id: AtomicU64::new(1),
            auth: None,
            message_url: RwLock::new(None),
            request_timeout,
        }))
    }

    pub fn with_auth_remote(
        remote: ResolvedRemoteConfig,
        auth: SharedAuthProvider,
        request_timeout: Duration,
    ) -> Result<Arc<Self>> {
        let client = Client::builder()
            .timeout(request_timeout)
            .build()
            .context("failed to build HTTP client for legacy SSE transport")?;

        Ok(Arc::new(Self {
            client,
            sse_url: Secret::new(remote.request_url().to_string()),
            display_url: remote.display_url().to_string(),
            default_headers: remote.headers().clone(),
            next_id: AtomicU64::new(1),
            auth: Some(auth),
            message_url: RwLock::new(None),
            request_timeout,
        }))
    }

    /// Discover the message endpoint by connecting to the SSE stream.
    ///
    /// Sends a GET request to the SSE URL and streams the response,
    /// returning as soon as the `event: endpoint` event is found. This avoids
    /// blocking on servers that keep the SSE connection open after the initial
    /// endpoint event.
    async fn discover_endpoint(&self) -> Result<String> {
        use futures::StreamExt;

        debug!(url = %self.display_url, "discovering legacy SSE message endpoint");

        let mut req = self
            .client
            .get(self.sse_url.expose_secret())
            .timeout(SSE_ENDPOINT_DISCOVERY_TIMEOUT)
            .header("Accept", "text/event-stream");

        req = self.apply_default_headers(req);

        if let Some(token) = self.get_auth_token().await? {
            req = req.header("Authorization", format!("Bearer {}", token.expose_secret()));
        }

        let resp = req
            .send()
            .await
            .map_err(sanitize_reqwest_error)
            .with_context(|| {
                format!(
                    "legacy SSE endpoint discovery GET to '{}' failed",
                    self.display_url
                )
            })?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(McpTransportError::Unauthorized {
                www_authenticate: resp
                    .headers()
                    .get("www-authenticate")
                    .and_then(|v| v.to_str().ok())
                    .map(String::from),
            }
            .into());
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::message(format!(
                "legacy SSE endpoint discovery returned HTTP {status}: {body}"
            )));
        }

        // Stream the response incrementally. Real SSE servers keep the
        // connection open after sending `event: endpoint`, so we must not
        // wait for EOF. Return as soon as the endpoint event is parsed.
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk
                .map_err(sanitize_reqwest_error)
                .with_context(|| format!("error reading SSE stream from '{}'", self.display_url))?;
            buf.push_str(&String::from_utf8_lossy(&chunk));

            if let Ok(url) = parse_endpoint_event(&buf, self.sse_url.expose_secret()) {
                return Ok(url);
            }
        }

        // Stream closed without finding an endpoint event.
        Err(Error::message(format!(
            "no 'endpoint' event found in SSE stream from '{}' \
             (server may not support legacy SSE transport)",
            self.display_url
        )))
    }

    /// Get or discover the message endpoint URL.
    ///
    /// Uses a double-checked pattern to avoid duplicate discovery when
    /// multiple requests race on a cold start.
    async fn get_message_url(&self) -> Result<String> {
        if let Some(url) = self.message_url.read().await.as_ref() {
            return Ok(url.clone());
        }

        let endpoint = self.discover_endpoint().await?;

        // Re-check under write lock: another caller may have completed
        // discovery between our read-unlock and write-lock.
        let mut slot = self.message_url.write().await;
        if let Some(existing) = slot.as_ref() {
            return Ok(existing.clone());
        }

        info!(
            url = %self.display_url,
            message_endpoint = %endpoint,
            "discovered legacy SSE message endpoint"
        );
        *slot = Some(endpoint.clone());
        Ok(endpoint)
    }

    async fn get_auth_token(&self) -> Result<Option<Secret<String>>> {
        match &self.auth {
            Some(auth) => auth.access_token().await,
            None => Ok(None),
        }
    }

    fn apply_default_headers(&self, mut req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        for (name, value) in &self.default_headers {
            if self.auth.is_some() && name == reqwest::header::AUTHORIZATION {
                continue;
            }
            req = req.header(name, value);
        }
        req
    }

    /// POST a JSON-RPC message to the discovered message endpoint.
    async fn post_message(
        &self,
        method: &str,
        body: &impl serde::Serialize,
    ) -> Result<reqwest::Response> {
        let message_url = self.get_message_url().await?;

        let mut req = self
            .client
            .post(&message_url)
            .timeout(self.request_timeout)
            .header("Content-Type", "application/json");

        req = self.apply_default_headers(req);

        if let Some(token) = self.get_auth_token().await? {
            req = req.header("Authorization", format!("Bearer {}", token.expose_secret()));
        }

        let resp = req
            .json(body)
            .send()
            .await
            .map_err(sanitize_reqwest_error)
            .with_context(|| {
                format!(
                    "legacy SSE POST to '{}' for '{method}' failed",
                    self.display_url
                )
            })?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(McpTransportError::Unauthorized {
                www_authenticate: resp
                    .headers()
                    .get("www-authenticate")
                    .and_then(|v| v.to_str().ok())
                    .map(String::from),
            }
            .into());
        }

        Ok(resp)
    }
}

/// Parse the `event: endpoint` SSE event from the response body and resolve it
/// against the base SSE URL.
///
/// Only matches data lines preceded by an explicit `event: endpoint` line.
fn parse_endpoint_event(body: &str, base_url: &str) -> Result<String> {
    let mut current_event: Option<&str> = None;

    for line in body.lines() {
        let trimmed = line.trim_end();

        if let Some(event_type) = trimmed.strip_prefix("event:") {
            current_event = Some(event_type.trim());
            continue;
        }

        if let Some(data) = trimmed.strip_prefix("data:") {
            let data = data.trim();

            if current_event == Some("endpoint") && !data.is_empty() {
                return resolve_endpoint_url(data, base_url);
            }
            continue;
        }

        // Empty line resets event state
        if trimmed.is_empty() {
            current_event = None;
        }
    }

    Err(Error::message(
        "no 'endpoint' event found in SSE stream (server may not support legacy SSE transport)"
            .to_string(),
    ))
}

/// Resolve endpoint data (which may be relative) against the base SSE URL.
fn resolve_endpoint_url(endpoint_data: &str, base_url: &str) -> Result<String> {
    // If it's already an absolute URL, use it directly
    if endpoint_data.starts_with("http://") || endpoint_data.starts_with("https://") {
        return Ok(endpoint_data.to_string());
    }

    let base = Url::parse(base_url)
        .with_context(|| format!("invalid base URL for legacy SSE endpoint: {base_url}"))?;

    // Handle relative paths like "/message?sessionId=xxx" or "?sessionId=xxx"
    if let Some(query) = endpoint_data.strip_prefix('?') {
        // Query-only: append to the base URL path
        let mut resolved = base.clone();
        resolved.set_query(Some(query));
        return Ok(resolved.to_string());
    }

    // Relative path: resolve against base
    base.join(endpoint_data)
        .map(|u| u.to_string())
        .map_err(|_| {
            Error::message(format!(
                "failed to resolve legacy SSE endpoint '{}' against base '{}'",
                endpoint_data, base_url
            ))
        })
}

#[async_trait::async_trait]
impl McpTransport for LegacySseTransport {
    async fn request(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<JsonRpcResponse> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let req = JsonRpcRequest::new(id, method, params);

        debug!(method = %method, id = %id, url = %self.display_url, "legacy SSE client -> server");

        let http_resp = self.post_message(method, &req).await?;

        if !http_resp.status().is_success() {
            let status = http_resp.status();
            let body = http_resp.text().await.unwrap_or_default();
            return Err(Error::message(format!(
                "MCP legacy SSE server returned HTTP {status} for '{method}': {body}"
            )));
        }

        let resp: JsonRpcResponse = http_resp
            .json()
            .await
            .with_context(|| format!("failed to parse JSON-RPC response for '{method}'"))?;

        if let Some(ref err) = resp.error {
            return Err(Error::message(format!(
                "MCP legacy SSE error on '{method}': code={} message={}",
                err.code, err.message
            )));
        }

        Ok(resp)
    }

    async fn notify(&self, method: &str, params: Option<serde_json::Value>) -> Result<()> {
        let notif = JsonRpcNotification {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
        };

        debug!(
            method = %method,
            url = %self.display_url,
            "legacy SSE client -> server (notification)"
        );

        let http_resp = self.post_message(method, &notif).await?;

        if !http_resp.status().is_success() {
            let status = http_resp.status();
            warn!(method = %method, %status, "legacy SSE notification returned non-success");
        }

        Ok(())
    }

    async fn is_alive(&self) -> bool {
        // For legacy SSE, we just try to reach the SSE endpoint.
        let mut req = self
            .client
            .get(self.sse_url.expose_secret())
            .timeout(Duration::from_secs(5))
            .header("Accept", "text/event-stream");

        req = self.apply_default_headers(req);

        if let Some(token) = match &self.auth {
            Some(auth) => auth.access_token().await.ok().flatten(),
            None => None,
        } {
            req = req.header("Authorization", format!("Bearer {}", token.expose_secret()));
        }

        req.send().await.is_ok()
    }

    async fn kill(&self) {
        // Legacy SSE has no explicit session termination.
        // Clear the cached message URL so a reconnect would re-discover.
        let mut slot = self.message_url.write().await;
        *slot = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unused_local_url() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        format!("http://{addr}/sse")
    }

    #[test]
    fn test_legacy_sse_transport_creation() {
        let transport = LegacySseTransport::new("http://localhost:8080/sse");
        assert!(transport.is_ok());
    }

    #[test]
    fn test_legacy_sse_transport_invalid_url() {
        let transport = LegacySseTransport::new("not-a-url");
        assert!(transport.is_err());
    }

    #[tokio::test]
    async fn test_legacy_sse_transport_is_alive_unreachable() {
        let transport = LegacySseTransport::new(&unused_local_url()).unwrap();
        assert!(!transport.is_alive().await);
    }

    #[tokio::test]
    async fn test_legacy_sse_transport_request_unreachable() {
        let transport = LegacySseTransport::new(&unused_local_url()).unwrap();
        let result = transport.request("test", None).await;
        assert!(result.is_err());
    }

    // ── Endpoint parsing tests ────────────────────────────────────────

    #[test]
    fn test_parse_endpoint_event_relative_path() {
        let body = "event: endpoint\ndata: /message?sessionId=abc-123\n\n";
        let result = parse_endpoint_event(body, "http://localhost:3001/").unwrap();
        assert_eq!(result, "http://localhost:3001/message?sessionId=abc-123");
    }

    #[test]
    fn test_parse_endpoint_event_query_only() {
        let body = "event: endpoint\ndata: ?sessionId=f7e39497-f3c7-416c-9a3a-a48559a2bf5c\n\n";
        let result = parse_endpoint_event(body, "http://localhost:3001/").unwrap();
        assert_eq!(
            result,
            "http://localhost:3001/?sessionId=f7e39497-f3c7-416c-9a3a-a48559a2bf5c"
        );
    }

    #[test]
    fn test_parse_endpoint_event_absolute_url() {
        let body = "event: endpoint\ndata: http://other-host:4000/mcp/msg?sid=xyz\n\n";
        let result = parse_endpoint_event(body, "http://localhost:3001/sse").unwrap();
        assert_eq!(result, "http://other-host:4000/mcp/msg?sid=xyz");
    }

    #[test]
    fn test_parse_endpoint_event_relative_path_with_base_path() {
        let body = "event: endpoint\ndata: /mcp/message?sessionId=test-session\n\n";
        let result =
            parse_endpoint_event(body, "https://baserow.example.com/mcp/xxxx/sse").unwrap();
        assert_eq!(
            result,
            "https://baserow.example.com/mcp/message?sessionId=test-session"
        );
    }

    #[test]
    fn test_parse_endpoint_event_no_endpoint_found() {
        let body = "event: message\ndata: {\"some\": \"json\"}\n\n";
        let result = parse_endpoint_event(body, "http://localhost:3001/");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_endpoint_event_without_event_type_line_is_rejected() {
        // Data lines without an explicit `event: endpoint` are not treated as endpoints
        let body = "data: /message?sessionId=implicit-endpoint\n\n";
        let result = parse_endpoint_event(body, "http://localhost:3001/");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_endpoint_event_ignores_non_endpoint_events() {
        // Other event types with path-like data must not be mistaken for endpoints
        let body = "event: message\ndata: /error/occurred\n\nevent: endpoint\ndata: /message?sessionId=real\n\n";
        let result = parse_endpoint_event(body, "http://localhost:3001/").unwrap();
        assert_eq!(result, "http://localhost:3001/message?sessionId=real");
    }

    // ── Integration tests with mock server ────────────────────────────

    /// Reproduces issue #278: legacy SSE servers require endpoint discovery
    /// before the initialize request can be sent.
    #[tokio::test]
    async fn test_legacy_sse_full_handshake() {
        let mut server = mockito::Server::new_async().await;

        // Step 1: GET /sse returns the endpoint event
        let sse_mock = server
            .mock("GET", "/sse")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body("event: endpoint\ndata: /message?sessionId=test-session-id\n\n")
            .create_async()
            .await;

        // Step 2: POST /message?sessionId=test-session-id for initialize
        let msg_mock = server
            .mock("POST", "/message?sessionId=test-session-id")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"test"}}}"#)
            .create_async()
            .await;

        let url = format!("{}/sse", server.url());
        let transport = LegacySseTransport::new(&url).unwrap();

        let resp = transport
            .request(
                "initialize",
                Some(serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "moltis", "version": "test"}
                })),
            )
            .await
            .unwrap();

        assert!(resp.result.is_some());
        sse_mock.assert_async().await;
        msg_mock.assert_async().await;
    }

    /// Reproduces the lox-mcp case from issue #278: server returns 400
    /// "sessionId not provided" if POST is sent without endpoint discovery.
    #[tokio::test]
    async fn test_legacy_sse_query_only_endpoint() {
        let mut server = mockito::Server::new_async().await;

        // lox-mcp style: endpoint data is just a query string
        let sse_mock = server
            .mock("GET", "/")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body("event: endpoint\ndata: ?sessionId=f7e39497-f3c7-416c-9a3a-a48559a2bf5c\n\n")
            .create_async()
            .await;

        // The message should go to /?sessionId=...
        let msg_mock = server
            .mock(
                "POST",
                "/?sessionId=f7e39497-f3c7-416c-9a3a-a48559a2bf5c",
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"lox-mcp"}}}"#)
            .create_async()
            .await;

        let transport = LegacySseTransport::new(&server.url()).unwrap();

        let resp = transport
            .request(
                "initialize",
                Some(serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "moltis", "version": "test"}
                })),
            )
            .await
            .unwrap();

        assert!(resp.result.is_some());
        sse_mock.assert_async().await;
        msg_mock.assert_async().await;
    }

    /// Test that the message endpoint is cached after first discovery.
    #[tokio::test]
    async fn test_legacy_sse_caches_endpoint() {
        let mut server = mockito::Server::new_async().await;

        // The GET should only be called once
        let sse_mock = server
            .mock("GET", "/sse")
            .expect(1)
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body("event: endpoint\ndata: /message?sessionId=cached\n\n")
            .create_async()
            .await;

        let msg_mock = server
            .mock("POST", "/message?sessionId=cached")
            .expect(2)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#)
            .create_async()
            .await;

        let url = format!("{}/sse", server.url());
        let transport = LegacySseTransport::new(&url).unwrap();

        // First request triggers discovery
        transport.request("initialize", None).await.unwrap();
        // Second request uses cached endpoint
        transport.request("tools/list", None).await.unwrap();

        sse_mock.assert_async().await;
        msg_mock.assert_async().await;
    }

    /// Test that 401 from the SSE endpoint is properly reported.
    #[tokio::test]
    async fn test_legacy_sse_401_on_discovery() {
        let mut server = mockito::Server::new_async().await;

        let _mock = server
            .mock("GET", "/sse")
            .with_status(401)
            .with_header("www-authenticate", r#"Bearer realm="test""#)
            .create_async()
            .await;

        let url = format!("{}/sse", server.url());
        let transport = LegacySseTransport::new(&url).unwrap();

        let result = transport.request("initialize", None).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(
            err,
            crate::Error::Transport(McpTransportError::Unauthorized { .. })
        ));
    }
}
