//! HTTP/SSE transport for remote MCP servers (Streamable HTTP transport).
//!
//! Uses HTTP POST for JSON-RPC requests and GET for server-initiated SSE events.
//! Supports optional OAuth Bearer token injection and automatic 401 retry.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use {
    reqwest::Client,
    secrecy::ExposeSecret,
    tokio::sync::RwLock,
    tracing::{debug, info, warn},
};

use crate::{
    auth::SharedAuthProvider,
    error::{Context, Error, Result},
    traits::McpTransport,
    types::{
        JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, McpTransportError, PROTOCOL_VERSION,
    },
};

const MCP_PROTOCOL_VERSION_HEADER: &str = "MCP-Protocol-Version";
const MCP_SESSION_ID_HEADER: &str = "Mcp-Session-Id";
const STREAMABLE_ACCEPT_HEADER: &str = "application/json, text/event-stream";

/// HTTP/SSE-based transport for a remote MCP server.
pub struct SseTransport {
    client: Client,
    url: String,
    next_id: AtomicU64,
    /// Optional auth provider for Bearer token injection.
    auth: Option<SharedAuthProvider>,
    /// Session identifier used by streamable HTTP servers.
    session_id: RwLock<Option<String>>,
}

impl SseTransport {
    /// Create a new SSE transport pointing at the given MCP server URL.
    pub fn new(url: &str) -> Result<Arc<Self>> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .context("failed to build HTTP client for SSE transport")?;

        Ok(Arc::new(Self {
            client,
            url: url.to_string(),
            next_id: AtomicU64::new(1),
            auth: None,
            session_id: RwLock::new(None),
        }))
    }

    /// Create a new SSE transport with an OAuth auth provider.
    pub fn with_auth(url: &str, auth: SharedAuthProvider) -> Result<Arc<Self>> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .context("failed to build HTTP client for SSE transport")?;

        Ok(Arc::new(Self {
            client,
            url: url.to_string(),
            next_id: AtomicU64::new(1),
            auth: Some(auth),
            session_id: RwLock::new(None),
        }))
    }

    /// Build a request builder with optional Bearer token.
    async fn build_post(&self) -> Result<reqwest::RequestBuilder> {
        let mut req = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", STREAMABLE_ACCEPT_HEADER)
            .header(MCP_PROTOCOL_VERSION_HEADER, PROTOCOL_VERSION);

        if let Some(session_id) = self.session_id.read().await.clone() {
            req = req.header(MCP_SESSION_ID_HEADER, session_id);
        }

        if let Some(token) = match &self.auth {
            Some(auth) => auth.access_token().await?,
            None => None,
        } {
            req = req.header("Authorization", format!("Bearer {}", token.expose_secret()));
        }

        Ok(req)
    }

    async fn store_session_id_from_response(&self, response: &reqwest::Response) {
        let Some(raw) = response.headers().get(MCP_SESSION_ID_HEADER) else {
            return;
        };
        let Ok(session_id) = raw.to_str() else {
            return;
        };
        if session_id.trim().is_empty() {
            return;
        }

        let mut slot = self.session_id.write().await;
        let session_id = session_id.to_string();
        if slot.as_ref() != Some(&session_id) {
            debug!(
                url = %self.url,
                session_id = %session_id,
                "updated MCP streamable HTTP session id"
            );
            *slot = Some(session_id);
        }
    }

    fn response_is_event_stream(resp: &reqwest::Response) -> bool {
        resp.headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|ct| {
                ct.split(';')
                    .next()
                    .is_some_and(|base| base.trim() == "text/event-stream")
            })
            .unwrap_or(false)
    }

    fn parse_event_stream_response(body: &str, method: &str) -> Result<JsonRpcResponse> {
        let mut data = String::new();

        for line in body.lines() {
            let trimmed = line.trim_end();
            if let Some(rest) = trimmed.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(rest.trim_start());
                continue;
            }

            if trimmed.is_empty() && !data.is_empty() {
                if let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(&data) {
                    return Ok(resp);
                }
                data.clear();
            }
        }

        if !data.is_empty()
            && let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(&data)
        {
            return Ok(resp);
        }

        Err(Error::message(format!(
            "failed to parse JSON-RPC response from event stream for '{method}'"
        )))
    }

    /// Send a POST request and handle 401 with auth retry.
    /// Returns the HTTP response or a typed `McpTransportError`.
    async fn send_with_auth_retry(
        &self,
        method: &str,
        body: &impl serde::Serialize,
    ) -> Result<reqwest::Response> {
        // First attempt
        let req = self.build_post().await?;
        let http_resp = req
            .json(body)
            .send()
            .await
            .with_context(|| format!("SSE POST to '{}' for '{method}' failed", self.url))?;

        if http_resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            let has_session_header = http_resp.headers().contains_key(MCP_SESSION_ID_HEADER);
            self.store_session_id_from_response(&http_resp).await;

            if let Some(auth) = &self.auth {
                let first_www_auth = http_resp
                    .headers()
                    .get("www-authenticate")
                    .and_then(|v| v.to_str().ok())
                    .map(String::from);

                // Some streamable HTTP servers issue a session ID alongside 401
                // and expect subsequent requests to include it. Retry once with
                // current auth/session before forcing interactive re-auth.
                if has_session_header {
                    info!(
                        method = %method,
                        url = %self.url,
                        www_authenticate = ?first_www_auth,
                        "received 401 with session header, replaying request before OAuth re-auth"
                    );

                    let req = self.build_post().await?;
                    let replay_resp = req.json(body).send().await.with_context(|| {
                        format!(
                            "SSE POST session replay to '{}' for '{method}' failed",
                            self.url
                        )
                    })?;

                    if replay_resp.status() != reqwest::StatusCode::UNAUTHORIZED {
                        self.store_session_id_from_response(&replay_resp).await;
                        return Ok(replay_resp);
                    }

                    self.store_session_id_from_response(&replay_resp).await;
                    let replay_www_auth = replay_resp
                        .headers()
                        .get("www-authenticate")
                        .and_then(|v| v.to_str().ok())
                        .map(String::from);

                    info!(
                        method = %method,
                        url = %self.url,
                        "received 401 after session replay, attempting OAuth re-auth"
                    );

                    if auth.handle_unauthorized(replay_www_auth.as_deref()).await? {
                        // Retry with new token
                        let req = self.build_post().await?;
                        let retry_resp = req.json(body).send().await.with_context(|| {
                            format!("SSE POST retry to '{}' for '{method}' failed", self.url)
                        })?;

                        if retry_resp.status() == reqwest::StatusCode::UNAUTHORIZED {
                            return Err(McpTransportError::Unauthorized {
                                www_authenticate: retry_resp
                                    .headers()
                                    .get("www-authenticate")
                                    .and_then(|v| v.to_str().ok())
                                    .map(String::from),
                            }
                            .into());
                        }

                        self.store_session_id_from_response(&retry_resp).await;
                        return Ok(retry_resp);
                    }

                    return Err(McpTransportError::Unauthorized {
                        www_authenticate: replay_www_auth,
                    }
                    .into());
                }

                info!(
                    method = %method,
                    url = %self.url,
                    www_authenticate = ?first_www_auth,
                    "received 401, attempting OAuth re-auth"
                );

                if auth.handle_unauthorized(first_www_auth.as_deref()).await? {
                    // Retry with new token
                    let req = self.build_post().await?;
                    let retry_resp = req.json(body).send().await.with_context(|| {
                        format!("SSE POST retry to '{}' for '{method}' failed", self.url)
                    })?;

                    if retry_resp.status() == reqwest::StatusCode::UNAUTHORIZED {
                        return Err(McpTransportError::Unauthorized {
                            www_authenticate: retry_resp
                                .headers()
                                .get("www-authenticate")
                                .and_then(|v| v.to_str().ok())
                                .map(String::from),
                        }
                        .into());
                    }

                    self.store_session_id_from_response(&retry_resp).await;
                    return Ok(retry_resp);
                }
            }

            // No auth provider or auth failed
            return Err(McpTransportError::Unauthorized {
                www_authenticate: http_resp
                    .headers()
                    .get("www-authenticate")
                    .and_then(|v| v.to_str().ok())
                    .map(String::from),
            }
            .into());
        }

        self.store_session_id_from_response(&http_resp).await;
        Ok(http_resp)
    }
}

#[async_trait::async_trait]
impl McpTransport for SseTransport {
    async fn request(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<JsonRpcResponse> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let req = JsonRpcRequest::new(id, method, params);

        debug!(method = %method, id = %id, url = %self.url, "SSE client -> server");

        let http_resp = self.send_with_auth_retry(method, &req).await?;

        if !http_resp.status().is_success() {
            let status = http_resp.status();
            let body = http_resp.text().await.unwrap_or_default();
            return Err(Error::message(format!(
                "MCP SSE server returned HTTP {status} for '{method}': {body}"
            )));
        }

        let resp: JsonRpcResponse = if Self::response_is_event_stream(&http_resp) {
            let body = http_resp
                .text()
                .await
                .with_context(|| format!("failed to read event stream response for '{method}'"))?;
            Self::parse_event_stream_response(&body, method)?
        } else {
            http_resp
                .json()
                .await
                .with_context(|| format!("failed to parse JSON-RPC response for '{method}'"))?
        };

        if let Some(ref err) = resp.error {
            return Err(Error::message(format!(
                "MCP SSE error on '{method}': code={} message={}",
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

        debug!(method = %method, url = %self.url, "SSE client -> server (notification)");

        let http_resp = self.send_with_auth_retry(method, &notif).await?;

        if !http_resp.status().is_success() {
            let status = http_resp.status();
            warn!(method = %method, %status, "SSE notification returned non-success");
        }

        Ok(())
    }

    async fn is_alive(&self) -> bool {
        // Try a lightweight GET request to check connectivity and session continuity.
        let mut req = self
            .client
            .get(&self.url)
            .timeout(std::time::Duration::from_secs(5))
            .header("Accept", STREAMABLE_ACCEPT_HEADER)
            .header(MCP_PROTOCOL_VERSION_HEADER, PROTOCOL_VERSION);

        if let Some(session_id) = self.session_id.read().await.clone() {
            req = req.header(MCP_SESSION_ID_HEADER, session_id);
        }

        // Include auth header in health checks too
        if let Some(token) = match &self.auth {
            Some(auth) => auth.access_token().await.ok().flatten(),
            None => None,
        } {
            req = req.header("Authorization", format!("Bearer {}", token.expose_secret()));
        }

        match req.send().await {
            Ok(resp) => {
                self.store_session_id_from_response(&resp).await;
                true
            },
            Err(_) => false,
        }
    }

    async fn kill(&self) {
        let session_id = {
            let mut slot = self.session_id.write().await;
            slot.take()
        };

        let Some(session_id) = session_id else {
            return;
        };

        let mut req = self
            .client
            .delete(&self.url)
            .timeout(std::time::Duration::from_secs(5))
            .header(MCP_PROTOCOL_VERSION_HEADER, PROTOCOL_VERSION)
            .header(MCP_SESSION_ID_HEADER, session_id);

        if let Some(token) = match &self.auth {
            Some(auth) => auth.access_token().await.ok().flatten(),
            None => None,
        } {
            req = req.header("Authorization", format!("Bearer {}", token.expose_secret()));
        }

        if let Err(e) = req.send().await {
            warn!(url = %self.url, error = %e, "failed to close MCP streamable HTTP session");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sse_transport_creation() {
        let transport = SseTransport::new("http://localhost:8080/mcp");
        assert!(transport.is_ok());
    }

    #[test]
    fn test_sse_transport_invalid_url_still_creates() {
        // reqwest doesn't validate URLs at build time, only at request time
        let transport = SseTransport::new("not-a-url");
        assert!(transport.is_ok());
    }

    #[tokio::test]
    async fn test_sse_transport_is_alive_unreachable() {
        let transport = SseTransport::new("http://127.0.0.1:1/mcp").unwrap();
        assert!(!transport.is_alive().await);
    }

    #[tokio::test]
    async fn test_sse_transport_request_unreachable() {
        let transport = SseTransport::new("http://127.0.0.1:1/mcp").unwrap();
        let result = transport.request("test", None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_sse_transport_kill() {
        let transport = SseTransport::new("http://localhost:8080/mcp").unwrap();
        transport.kill().await;
        // Should not panic
    }

    #[test]
    fn test_sse_transport_with_auth_creation() {
        let auth: SharedAuthProvider = Arc::new(crate::auth::NoAuthProvider);
        let transport = SseTransport::with_auth("http://localhost:8080/mcp", auth);
        assert!(transport.is_ok());
    }

    #[tokio::test]
    async fn test_sse_transport_401_without_auth_returns_unauthorized() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/")
            .with_status(401)
            .with_header("www-authenticate", r#"Bearer realm="test""#)
            .create_async()
            .await;

        let transport = SseTransport::new(&server.url()).unwrap();
        let result = transport.request("test", None).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(
            err,
            crate::Error::Transport(McpTransportError::Unauthorized { .. })
        ));
    }

    #[tokio::test]
    async fn test_sse_transport_200_no_auth() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#)
            .create_async()
            .await;

        let transport = SseTransport::new(&server.url()).unwrap();
        let resp = transport.request("test", None).await.unwrap();
        assert!(resp.result.is_some());
    }

    #[tokio::test]
    async fn test_sse_transport_bearer_header_injected() {
        use secrecy::Secret;

        use crate::auth::{McpAuthProvider, McpAuthState};

        /// Test auth provider that always returns a fixed token.
        struct FixedTokenProvider;

        #[async_trait::async_trait]
        impl McpAuthProvider for FixedTokenProvider {
            async fn access_token(&self) -> Result<Option<Secret<String>>> {
                Ok(Some(Secret::new("test-token-123".to_string())))
            }

            async fn handle_unauthorized(&self, _: Option<&str>) -> Result<bool> {
                Ok(false)
            }

            async fn start_oauth(
                &self,
                _redirect_uri: &str,
                _www_authenticate: Option<&str>,
            ) -> Result<Option<String>> {
                Ok(None)
            }

            async fn complete_oauth(&self, _state: &str, _code: &str) -> Result<bool> {
                Ok(false)
            }

            fn pending_auth_url(&self) -> Option<String> {
                None
            }

            fn auth_state(&self) -> McpAuthState {
                McpAuthState::Authenticated
            }
        }

        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .match_header("authorization", "Bearer test-token-123")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#)
            .create_async()
            .await;

        let auth: SharedAuthProvider = Arc::new(FixedTokenProvider);
        let transport = SseTransport::with_auth(&server.url(), auth).unwrap();
        let resp = transport.request("test", None).await.unwrap();
        assert!(resp.result.is_some());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_sse_transport_propagates_session_id() {
        let mut server = mockito::Server::new_async().await;

        let first = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_header("mcp-session-id", "session-123")
            .with_body(r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#)
            .create_async()
            .await;

        let second = server
            .mock("POST", "/")
            .match_header("mcp-session-id", "session-123")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":2,"result":{"ok":true}}"#)
            .create_async()
            .await;

        let transport = SseTransport::new(&server.url()).unwrap();
        transport.request("initialize", None).await.unwrap();
        transport.request("tools/list", None).await.unwrap();

        first.assert_async().await;
        second.assert_async().await;
    }

    #[tokio::test]
    async fn test_sse_transport_401_with_session_replays_before_reauth() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        use secrecy::Secret;

        use crate::auth::{McpAuthProvider, McpAuthState};

        struct CountingAuthProvider {
            reauth_calls: AtomicUsize,
        }

        #[async_trait::async_trait]
        impl McpAuthProvider for CountingAuthProvider {
            async fn access_token(&self) -> Result<Option<Secret<String>>> {
                Ok(Some(Secret::new("token-123".to_string())))
            }

            async fn handle_unauthorized(&self, _www_authenticate: Option<&str>) -> Result<bool> {
                self.reauth_calls.fetch_add(1, Ordering::SeqCst);
                Ok(false)
            }

            async fn start_oauth(
                &self,
                _redirect_uri: &str,
                _www_authenticate: Option<&str>,
            ) -> Result<Option<String>> {
                Ok(None)
            }

            async fn complete_oauth(&self, _state: &str, _code: &str) -> Result<bool> {
                Ok(false)
            }

            fn pending_auth_url(&self) -> Option<String> {
                None
            }

            fn auth_state(&self) -> McpAuthState {
                McpAuthState::Authenticated
            }
        }

        let mut server = mockito::Server::new_async().await;

        let first = server
            .mock("POST", "/")
            .match_header("authorization", "Bearer token-123")
            .with_status(401)
            .with_header("mcp-session-id", "session-reauth-1")
            .with_header("www-authenticate", r#"Bearer realm="test""#)
            .create_async()
            .await;

        let second = server
            .mock("POST", "/")
            .match_header("authorization", "Bearer token-123")
            .match_header("mcp-session-id", "session-reauth-1")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#)
            .create_async()
            .await;

        let auth = Arc::new(CountingAuthProvider {
            reauth_calls: AtomicUsize::new(0),
        });
        let auth_shared: SharedAuthProvider = auth.clone();

        let transport = SseTransport::with_auth(&server.url(), auth_shared).unwrap();
        let resp = transport.request("initialize", None).await.unwrap();
        assert!(resp.result.is_some());
        assert_eq!(auth.reauth_calls.load(Ordering::SeqCst), 0);

        first.assert_async().await;
        second.assert_async().await;
    }

    #[tokio::test]
    async fn test_sse_transport_parses_event_stream_response() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(
                "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}\n\n",
            )
            .create_async()
            .await;

        let transport = SseTransport::new(&server.url()).unwrap();
        let resp = transport.request("initialize", None).await.unwrap();
        assert!(resp.result.is_some());
    }

    #[tokio::test]
    async fn test_sse_transport_kill_sends_delete_with_session_id() {
        let mut server = mockito::Server::new_async().await;
        let init = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_header("mcp-session-id", "session-to-close")
            .with_body(r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#)
            .create_async()
            .await;
        let delete = server
            .mock("DELETE", "/")
            .match_header("mcp-session-id", "session-to-close")
            .with_status(204)
            .create_async()
            .await;

        let transport = SseTransport::new(&server.url()).unwrap();
        transport.request("initialize", None).await.unwrap();
        transport.kill().await;

        init.assert_async().await;
        delete.assert_async().await;
    }
}
