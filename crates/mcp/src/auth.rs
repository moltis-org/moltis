//! OAuth 2.1 authentication provider for MCP servers.
//!
//! Implements the MCP Authorization Spec (2025-06-18):
//! - Protected resource metadata discovery (RFC 9728)
//! - Authorization server metadata discovery (RFC 8414)
//! - Dynamic client registration (RFC 7591)
//! - PKCE authorization code flow with browser callback

use std::sync::Arc;

use {
    anyhow::{Context, Result, bail},
    async_trait::async_trait,
    secrecy::{ExposeSecret, Secret},
    tokio::sync::RwLock,
    tracing::{debug, info, warn},
    url::Url,
};

use moltis_oauth::{
    CallbackServer, OAuthConfig, OAuthFlow, OAuthTokens, RegistrationStore, StoredRegistration,
    TokenStore, fetch_as_metadata, fetch_resource_metadata, parse_www_authenticate,
    register_client,
};

// ── Auth state ─────────────────────────────────────────────────────────────

/// Observable state of MCP OAuth authentication.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpAuthState {
    /// No authentication required or not yet attempted.
    NotRequired,
    /// Browser opened, waiting for user to complete OAuth flow.
    AwaitingBrowser,
    /// Successfully authenticated (have valid tokens).
    Authenticated,
    /// Authentication failed.
    Failed,
}

// ── Auth provider trait ────────────────────────────────────────────────────

/// Provides OAuth tokens for authenticating MCP HTTP requests.
#[async_trait]
pub trait McpAuthProvider: Send + Sync {
    /// Return a valid access token, refreshing if necessary.
    /// Returns `None` if no token is available and auth hasn't been initiated.
    async fn access_token(&self) -> Result<Option<Secret<String>>>;

    /// Handle a 401 Unauthorized response by performing the OAuth flow.
    /// Returns `true` if authentication succeeded and the request should be retried.
    async fn handle_unauthorized(&self, www_authenticate: Option<&str>) -> Result<bool>;

    /// Current authentication state.
    fn auth_state(&self) -> McpAuthState;
}

// ── Concrete OAuth provider ────────────────────────────────────────────────

/// Manual OAuth override configuration (from `moltis.toml`).
#[derive(Debug, Clone)]
pub struct McpOAuthOverride {
    pub client_id: String,
    pub auth_url: String,
    pub token_url: String,
    pub scopes: Vec<String>,
}

/// OAuth 2.1 provider for a single MCP server.
pub struct McpOAuthProvider {
    server_name: String,
    server_url: String,
    http_client: reqwest::Client,
    token_store: TokenStore,
    registration_store: RegistrationStore,
    state: RwLock<McpAuthState>,
    cached_token: RwLock<Option<OAuthTokens>>,
    /// Optional manual override (skip discovery).
    oauth_override: Option<McpOAuthOverride>,
}

impl McpOAuthProvider {
    pub fn new(server_name: &str, server_url: &str) -> Self {
        Self {
            server_name: server_name.to_string(),
            server_url: server_url.to_string(),
            http_client: reqwest::Client::new(),
            token_store: TokenStore::new(),
            registration_store: RegistrationStore::new(),
            state: RwLock::new(McpAuthState::NotRequired),
            cached_token: RwLock::new(None),
            oauth_override: None,
        }
    }

    /// Create with custom stores (for testing).
    pub fn with_stores(
        server_name: &str,
        server_url: &str,
        token_store: TokenStore,
        registration_store: RegistrationStore,
    ) -> Self {
        Self {
            server_name: server_name.to_string(),
            server_url: server_url.to_string(),
            http_client: reqwest::Client::new(),
            token_store,
            registration_store,
            state: RwLock::new(McpAuthState::NotRequired),
            cached_token: RwLock::new(None),
            oauth_override: None,
        }
    }

    /// Set a manual OAuth override (skip discovery + dynamic registration).
    pub fn with_oauth_override(mut self, ov: McpOAuthOverride) -> Self {
        self.oauth_override = Some(ov);
        self
    }

    /// Token store key for this server.
    fn store_key(&self) -> String {
        format!("mcp:{}", self.server_name)
    }

    /// Check whether the cached token is expired or near-expiry (60s buffer).
    fn is_token_expired(tokens: &OAuthTokens) -> bool {
        let Some(expires_at) = tokens.expires_at else {
            return false; // No expiry info → assume valid
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        now + 60 >= expires_at
    }

    /// Try to refresh tokens using the refresh token.
    async fn try_refresh(&self, tokens: &OAuthTokens) -> Result<Option<OAuthTokens>> {
        let refresh_token = match &tokens.refresh_token {
            Some(rt) => rt,
            None => return Ok(None),
        };

        // Need the token endpoint. Try loading from stored registration or override.
        let (client_id, token_url, resource) = if let Some(ov) = &self.oauth_override {
            (
                ov.client_id.clone(),
                ov.token_url.clone(),
                Some(self.server_url.clone()),
            )
        } else if let Some(reg) = self.registration_store.load(&self.server_url) {
            (reg.client_id, reg.token_endpoint, Some(reg.resource))
        } else {
            return Ok(None); // Can't refresh without knowing where to send the request
        };

        debug!(server = %self.server_name, "refreshing MCP OAuth token");

        let config = OAuthConfig {
            client_id,
            auth_url: String::new(), // Not needed for refresh
            token_url,
            redirect_uri: String::new(),
            resource,
            scopes: Vec::new(),
            extra_auth_params: Vec::new(),
            device_flow: false,
        };

        let flow = OAuthFlow::new(config);
        match flow.refresh(refresh_token.expose_secret()).await {
            Ok(new_tokens) => {
                self.token_store.save(&self.store_key(), &new_tokens)?;
                info!(server = %self.server_name, "MCP OAuth token refreshed");
                Ok(Some(new_tokens))
            },
            Err(e) => {
                warn!(server = %self.server_name, error = %e, "MCP OAuth token refresh failed");
                Ok(None)
            },
        }
    }

    /// Perform the full OAuth flow: discovery → registration → PKCE → browser → callback.
    async fn perform_oauth_flow(&self, www_authenticate: Option<&str>) -> Result<()> {
        *self.state.write().await = McpAuthState::AwaitingBrowser;

        let result = self.perform_oauth_flow_inner(www_authenticate).await;

        match &result {
            Ok(()) => {
                *self.state.write().await = McpAuthState::Authenticated;
            },
            Err(e) => {
                warn!(server = %self.server_name, error = %e, "MCP OAuth flow failed");
                *self.state.write().await = McpAuthState::Failed;
            },
        }

        result
    }

    async fn perform_oauth_flow_inner(&self, www_authenticate: Option<&str>) -> Result<()> {
        // Bind ephemeral port for callback FIRST — this port must be used for
        // both dynamic client registration and the authorization request.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .context("failed to bind callback listener")?;
        let callback_port = listener.local_addr()?.port();
        drop(listener); // Release so CallbackServer can bind

        let redirect_uri = format!("http://127.0.0.1:{callback_port}/auth/callback");

        let (client_id, auth_url, token_url, scopes, resource) =
            if let Some(ov) = &self.oauth_override {
                // Manual override: skip discovery
                (
                    ov.client_id.clone(),
                    ov.auth_url.clone(),
                    ov.token_url.clone(),
                    ov.scopes.clone(),
                    self.server_url.clone(),
                )
            } else {
                // Full discovery flow
                self.discover_and_register(www_authenticate, &redirect_uri)
                    .await?
            };

        let config = OAuthConfig {
            client_id,
            auth_url,
            token_url,
            redirect_uri,
            resource: Some(resource),
            scopes,
            extra_auth_params: Vec::new(),
            device_flow: false,
        };

        info!(
            server = %self.server_name,
            resource = %config.resource.as_deref().unwrap_or(""),
            "starting MCP OAuth authorization flow"
        );

        let flow = OAuthFlow::new(config);
        let auth_req = flow.start()?;

        info!(
            server = %self.server_name,
            port = callback_port,
            "opening browser for MCP OAuth flow"
        );

        // Open browser
        if let Err(e) = open::that(&auth_req.url) {
            warn!(error = %e, "failed to open browser for OAuth — please open manually");
            info!(url = %auth_req.url, "OAuth authorization URL");
        }

        // Wait for callback (120s timeout, handled by CallbackServer's own 60s + our select)
        let code = CallbackServer::wait_for_code(callback_port, auth_req.state)
            .await
            .context("OAuth callback failed")?;

        // Exchange code for tokens
        let tokens = flow
            .exchange(&code, &auth_req.pkce.verifier)
            .await
            .context("OAuth token exchange failed")?;

        // Persist tokens
        self.token_store.save(&self.store_key(), &tokens)?;
        *self.cached_token.write().await = Some(tokens);

        info!(server = %self.server_name, "MCP OAuth authentication complete");

        Ok(())
    }

    /// Extract the origin (scheme + host + port) from a URL, stripping the path.
    fn origin_url(url: &Url) -> Url {
        let mut origin = url.clone();
        origin.set_path("/");
        origin.set_query(None);
        origin.set_fragment(None);
        origin
    }

    /// Build an RFC 8707 resource indicator from a server URL origin.
    ///
    /// Uses scheme + host (+ explicit port) without path/query/fragment.
    fn origin_resource(url: &Url) -> String {
        match (url.host_str(), url.port()) {
            (Some(host), Some(port)) => format!("{}://{host}:{port}", url.scheme()),
            (Some(host), None) => format!("{}://{host}", url.scheme()),
            _ => url.to_string(),
        }
    }

    /// Discover resource + AS metadata and perform dynamic client registration.
    ///
    /// Returns `(client_id, auth_url, token_url, scopes, resource)`.
    ///
    /// Per the MCP Authorization spec, well-known metadata URLs are tried at the
    /// server's full URL first (path-aware), then at the origin (scheme + host)
    /// as a fallback.
    async fn discover_and_register(
        &self,
        www_authenticate: Option<&str>,
        redirect_uri: &str,
    ) -> Result<(String, String, String, Vec<String>, String)> {
        let server_url = Url::parse(&self.server_url)
            .with_context(|| format!("invalid MCP server URL: {}", self.server_url))?;
        let origin = Self::origin_url(&server_url);
        let has_path = server_url.path() != "/" && !server_url.path().is_empty();

        debug!(
            server = %self.server_name,
            server_url = %server_url,
            origin = %origin,
            has_path,
            www_authenticate = ?www_authenticate,
            "starting OAuth discovery"
        );

        // Step 1: Try to get resource metadata (RFC 9728) from WWW-Authenticate
        // header or well-known endpoint. If the path-aware URL fails and the
        // server URL has a non-trivial path, retry at the origin.
        let resource_meta_result =
            if let Some(meta_url) = www_authenticate.and_then(parse_www_authenticate) {
                debug!(url = %meta_url, "using resource_metadata URL from WWW-Authenticate");
                let meta_url = Url::parse(&meta_url)
                    .context("invalid resource_metadata URL in WWW-Authenticate header")?;
                fetch_resource_metadata(&self.http_client, &meta_url).await
            } else {
                let result = fetch_resource_metadata(&self.http_client, &server_url).await;
                if result.is_err() && has_path {
                    debug!(
                        server = %self.server_name,
                        origin = %origin,
                        "resource metadata unavailable at path-aware URL, trying origin"
                    );
                    // Try origin; if that also fails, keep the original error
                    fetch_resource_metadata(&self.http_client, &origin)
                        .await
                        .or(result)
                } else {
                    result
                }
            };

        // Step 2: Get AS metadata — either from resource metadata's
        // authorization_servers list, or directly from the server's origin.
        let (as_meta, resource) = match resource_meta_result {
            Ok(resource_meta) => {
                let resource = resource_meta.resource.clone();
                let as_url_str = resource_meta
                    .authorization_servers
                    .first()
                    .context("no authorization_servers in protected resource metadata")?;
                let as_url = Url::parse(as_url_str)
                    .with_context(|| format!("invalid authorization server URL: {as_url_str}"))?;
                let as_meta = fetch_as_metadata(&self.http_client, &as_url).await?;
                (as_meta, resource)
            },
            Err(e) => {
                debug!(
                    server = %self.server_name,
                    error = %e,
                    "RFC 9728 resource metadata unavailable, trying RFC 8414"
                );
                // Fall back: fetch AS metadata. Try the server URL first, then
                // the origin if the server has a non-trivial path.
                let as_meta = match fetch_as_metadata(&self.http_client, &server_url).await {
                    Ok(meta) => meta,
                    Err(path_err) if has_path => {
                        debug!(
                            server = %self.server_name,
                            origin = %origin,
                            "AS metadata unavailable at path-aware URL, trying origin"
                        );
                        fetch_as_metadata(&self.http_client, &origin).await.with_context(|| {
                            format!(
                                "AS metadata unavailable at both {server_url} and {origin}: {path_err}"
                            )
                        })?
                    },
                    Err(e) => return Err(e),
                };
                // When resource metadata is unavailable we fall back to origin
                // as the resource indicator to avoid path-scoped audience mismatches.
                let resource = Self::origin_resource(&server_url);
                (as_meta, resource)
            },
        };

        debug!(
            server = %self.server_name,
            issuer = %as_meta.issuer,
            auth_endpoint = %as_meta.authorization_endpoint,
            token_endpoint = %as_meta.token_endpoint,
            registration = ?as_meta.registration_endpoint,
            resource = %resource,
            "resolved OAuth endpoints"
        );

        // Step 3: Dynamic client registration (or use cached)
        let client_id = if let Some(cached) = self.registration_store.load(&self.server_url) {
            debug!(
                server = %self.server_name,
                client_id = %cached.client_id,
                "reusing cached dynamic registration"
            );
            cached.client_id
        } else if let Some(reg_endpoint) = &as_meta.registration_endpoint {
            // Register the exact callback URI that we'll use for this auth flow.
            // Some providers require an exact redirect URI match and reject
            // port-agnostic loopback registrations.
            let reg = register_client(
                &self.http_client,
                reg_endpoint,
                vec![redirect_uri.to_string()],
                &format!("moltis ({})", self.server_name),
            )
            .await?;

            // Persist registration
            let stored = StoredRegistration {
                client_id: reg.client_id.clone(),
                client_secret: reg.client_secret.map(Secret::new),
                authorization_endpoint: as_meta.authorization_endpoint.clone(),
                token_endpoint: as_meta.token_endpoint.clone(),
                resource: resource.clone(),
                registered_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            };
            self.registration_store.save(&self.server_url, &stored)?;

            reg.client_id
        } else {
            bail!("AS does not support dynamic client registration and no client_id configured");
        };

        Ok((
            client_id,
            as_meta.authorization_endpoint,
            as_meta.token_endpoint,
            as_meta.scopes_supported,
            resource,
        ))
    }
}

#[async_trait]
impl McpAuthProvider for McpOAuthProvider {
    async fn access_token(&self) -> Result<Option<Secret<String>>> {
        // Check cache first
        {
            let cached = self.cached_token.read().await;
            #[allow(clippy::collapsible_if)]
            if let Some(tokens) = cached.as_ref() {
                if !Self::is_token_expired(tokens) {
                    return Ok(Some(tokens.access_token.clone()));
                }
                // Token expired — try refresh below
            }
        }

        // Try loading from store
        if let Some(tokens) = self.token_store.load(&self.store_key()) {
            if Self::is_token_expired(&tokens) {
                // Try refresh
                if let Some(new_tokens) = self.try_refresh(&tokens).await? {
                    let token = new_tokens.access_token.clone();
                    *self.cached_token.write().await = Some(new_tokens);
                    return Ok(Some(token));
                }
                // Refresh failed or no refresh token — return None to trigger re-auth
                return Ok(None);
            }
            let token = tokens.access_token.clone();
            *self.cached_token.write().await = Some(tokens);
            return Ok(Some(token));
        }

        Ok(None)
    }

    async fn handle_unauthorized(&self, www_authenticate: Option<&str>) -> Result<bool> {
        // Clear only in-memory cache before re-auth. We intentionally keep
        // persisted credentials until a new flow succeeds so an interrupted
        // browser flow does not permanently wipe usable tokens.
        *self.cached_token.write().await = None;

        match self.perform_oauth_flow(www_authenticate).await {
            Ok(()) => Ok(true),
            Err(e) => {
                warn!(server = %self.server_name, error = %e, "OAuth re-auth failed");
                Ok(false)
            },
        }
    }

    fn auth_state(&self) -> McpAuthState {
        // Use try_read to avoid blocking; fall back to NotRequired
        self.state
            .try_read()
            .map(|s| *s)
            .unwrap_or(McpAuthState::NotRequired)
    }
}

/// A no-op auth provider for servers that don't need authentication.
pub struct NoAuthProvider;

#[async_trait]
impl McpAuthProvider for NoAuthProvider {
    async fn access_token(&self) -> Result<Option<Secret<String>>> {
        Ok(None)
    }

    async fn handle_unauthorized(&self, _www_authenticate: Option<&str>) -> Result<bool> {
        Ok(false)
    }

    fn auth_state(&self) -> McpAuthState {
        McpAuthState::NotRequired
    }
}

// ── Thread-safe wrapper ────────────────────────────────────────────────────

/// Type alias for a shared auth provider.
pub type SharedAuthProvider = Arc<dyn McpAuthProvider>;

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use mockito::Matcher;

    use super::*;

    #[test]
    fn origin_url_strips_path() {
        let url = Url::parse("https://mcp.linear.app/sse").unwrap();
        let origin = McpOAuthProvider::origin_url(&url);
        assert_eq!(origin.as_str(), "https://mcp.linear.app/");
    }

    #[test]
    fn origin_url_preserves_port() {
        let url = Url::parse("https://mcp.example.com:8443/v1/mcp").unwrap();
        let origin = McpOAuthProvider::origin_url(&url);
        assert_eq!(origin.as_str(), "https://mcp.example.com:8443/");
    }

    #[test]
    fn origin_url_root_unchanged() {
        let url = Url::parse("https://mcp.example.com/").unwrap();
        let origin = McpOAuthProvider::origin_url(&url);
        assert_eq!(origin.as_str(), "https://mcp.example.com/");
    }

    #[test]
    fn origin_url_strips_query_and_fragment() {
        let url = Url::parse("https://mcp.example.com/sse?token=abc#frag").unwrap();
        let origin = McpOAuthProvider::origin_url(&url);
        assert_eq!(origin.as_str(), "https://mcp.example.com/");
    }

    #[test]
    fn origin_resource_strips_path_and_trailing_slash() {
        let url = Url::parse("https://mcp.linear.app/mcp").unwrap();
        let resource = McpOAuthProvider::origin_resource(&url);
        assert_eq!(resource, "https://mcp.linear.app");
    }

    #[test]
    fn origin_resource_preserves_explicit_port() {
        let url = Url::parse("https://mcp.example.com:8443/v1/mcp").unwrap();
        let resource = McpOAuthProvider::origin_resource(&url);
        assert_eq!(resource, "https://mcp.example.com:8443");
    }

    #[test]
    fn auth_state_serialization() {
        assert_eq!(
            serde_json::to_string(&McpAuthState::NotRequired).unwrap(),
            r#""not_required""#
        );
        assert_eq!(
            serde_json::to_string(&McpAuthState::AwaitingBrowser).unwrap(),
            r#""awaiting_browser""#
        );
        assert_eq!(
            serde_json::to_string(&McpAuthState::Authenticated).unwrap(),
            r#""authenticated""#
        );
        assert_eq!(
            serde_json::to_string(&McpAuthState::Failed).unwrap(),
            r#""failed""#
        );
    }

    #[tokio::test]
    async fn no_auth_provider_returns_none() {
        let provider = NoAuthProvider;
        assert!(provider.access_token().await.unwrap().is_none());
        assert!(!provider.handle_unauthorized(None).await.unwrap());
        assert_eq!(provider.auth_state(), McpAuthState::NotRequired);
    }

    #[test]
    fn token_expiry_check() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Expired token
        let expired = OAuthTokens {
            access_token: Secret::new("test".to_string()),
            refresh_token: None,
            expires_at: Some(now - 100),
        };
        assert!(McpOAuthProvider::is_token_expired(&expired));

        // Near-expiry token (within 60s buffer)
        let near_expiry = OAuthTokens {
            access_token: Secret::new("test".to_string()),
            refresh_token: None,
            expires_at: Some(now + 30),
        };
        assert!(McpOAuthProvider::is_token_expired(&near_expiry));

        // Valid token (far from expiry)
        let valid = OAuthTokens {
            access_token: Secret::new("test".to_string()),
            refresh_token: None,
            expires_at: Some(now + 3600),
        };
        assert!(!McpOAuthProvider::is_token_expired(&valid));

        // No expiry info
        let no_expiry = OAuthTokens {
            access_token: Secret::new("test".to_string()),
            refresh_token: None,
            expires_at: None,
        };
        assert!(!McpOAuthProvider::is_token_expired(&no_expiry));
    }

    #[tokio::test]
    async fn provider_loads_from_store() {
        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join("tokens.json");
        let reg_path = dir.path().join("registrations.json");

        let token_store = TokenStore::with_path(token_path);
        let reg_store = RegistrationStore::with_path(reg_path);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Pre-populate tokens
        let tokens = OAuthTokens {
            access_token: Secret::new("cached-token".to_string()),
            refresh_token: None,
            expires_at: Some(now + 3600),
        };
        token_store.save("mcp:test-server", &tokens).unwrap();

        let provider = McpOAuthProvider::with_stores(
            "test-server",
            "https://mcp.example.com",
            token_store,
            reg_store,
        );

        let token = provider.access_token().await.unwrap().unwrap();
        assert_eq!(token.expose_secret(), "cached-token");
        assert_eq!(provider.auth_state(), McpAuthState::NotRequired);
    }

    #[tokio::test]
    async fn provider_returns_none_for_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join("tokens.json");
        let reg_path = dir.path().join("registrations.json");

        let provider = McpOAuthProvider::with_stores(
            "test-server",
            "https://mcp.example.com",
            TokenStore::with_path(token_path),
            RegistrationStore::with_path(reg_path),
        );

        assert!(provider.access_token().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn provider_returns_none_for_expired_token_no_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join("tokens.json");
        let reg_path = dir.path().join("registrations.json");

        let token_store = TokenStore::with_path(token_path);
        let reg_store = RegistrationStore::with_path(reg_path);

        // Save an expired token with no refresh_token
        let tokens = OAuthTokens {
            access_token: Secret::new("expired-token".to_string()),
            refresh_token: None,
            expires_at: Some(0), // long expired
        };
        token_store.save("mcp:test-server", &tokens).unwrap();

        let provider = McpOAuthProvider::with_stores(
            "test-server",
            "https://mcp.example.com",
            token_store,
            reg_store,
        );

        assert!(provider.access_token().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn discovery_falls_back_to_origin_as_metadata_for_path_url() {
        let mut server = mockito::Server::new_async().await;
        let base = server.url();

        let resource_meta_path = server
            .mock("GET", "/sse/.well-known/oauth-protected-resource")
            .with_status(404)
            .create_async()
            .await;
        let resource_meta_origin = server
            .mock("GET", "/.well-known/oauth-protected-resource")
            .with_status(404)
            .create_async()
            .await;

        let as_meta_path = server
            .mock("GET", "/sse/.well-known/oauth-authorization-server")
            .with_status(404)
            .create_async()
            .await;
        let as_meta_origin = server
            .mock("GET", "/.well-known/oauth-authorization-server")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "issuer": base.clone(),
                    "authorization_endpoint": format!("{base}/authorize"),
                    "token_endpoint": format!("{base}/token"),
                    "registration_endpoint": format!("{base}/register"),
                    "scopes_supported": ["read"],
                })
                .to_string(),
            )
            .create_async()
            .await;

        let redirect_uri = "http://127.0.0.1:5555/auth/callback";
        let register = server
            .mock("POST", "/register")
            .match_body(Matcher::PartialJson(serde_json::json!({
                "redirect_uris": [redirect_uri],
            })))
            .with_status(201)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "client_id": "client-fallback",
                    "redirect_uris": [redirect_uri],
                })
                .to_string(),
            )
            .create_async()
            .await;

        let dir = tempfile::tempdir().unwrap();
        let provider = McpOAuthProvider::with_stores(
            "linear",
            &format!("{base}/sse"),
            TokenStore::with_path(dir.path().join("tokens.json")),
            RegistrationStore::with_path(dir.path().join("registrations.json")),
        );

        let (client_id, auth_url, token_url, scopes, resource) = provider
            .discover_and_register(None, redirect_uri)
            .await
            .unwrap();

        assert_eq!(client_id, "client-fallback");
        assert_eq!(auth_url, format!("{base}/authorize"));
        assert_eq!(token_url, format!("{base}/token"));
        assert_eq!(scopes, vec!["read".to_string()]);
        assert_eq!(resource, base);

        resource_meta_path.assert_async().await;
        resource_meta_origin.assert_async().await;
        as_meta_path.assert_async().await;
        as_meta_origin.assert_async().await;
        register.assert_async().await;
    }

    #[tokio::test]
    async fn dynamic_registration_uses_exact_redirect_uri() {
        let mut server = mockito::Server::new_async().await;
        let base = server.url();
        let redirect_uri = "http://127.0.0.1:43123/auth/callback";

        let resource_meta = server
            .mock("GET", "/mcp/.well-known/oauth-protected-resource")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "resource": format!("{base}/mcp"),
                    "authorization_servers": [base.clone()],
                    "scopes_supported": ["read", "write"],
                })
                .to_string(),
            )
            .create_async()
            .await;

        let as_meta = server
            .mock("GET", "/.well-known/oauth-authorization-server")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "issuer": base.clone(),
                    "authorization_endpoint": format!("{base}/authorize"),
                    "token_endpoint": format!("{base}/token"),
                    "registration_endpoint": format!("{base}/register"),
                })
                .to_string(),
            )
            .create_async()
            .await;

        let register = server
            .mock("POST", "/register")
            .match_body(Matcher::PartialJson(serde_json::json!({
                "redirect_uris": [redirect_uri],
            })))
            .with_status(201)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "client_id": "client-redirect",
                    "redirect_uris": [redirect_uri],
                })
                .to_string(),
            )
            .create_async()
            .await;

        let dir = tempfile::tempdir().unwrap();
        let provider = McpOAuthProvider::with_stores(
            "remote",
            &format!("{base}/mcp"),
            TokenStore::with_path(dir.path().join("tokens.json")),
            RegistrationStore::with_path(dir.path().join("registrations.json")),
        );

        let (client_id, _, _, _, _) = provider
            .discover_and_register(None, redirect_uri)
            .await
            .unwrap();

        assert_eq!(client_id, "client-redirect");
        resource_meta.assert_async().await;
        as_meta.assert_async().await;
        register.assert_async().await;
    }

    #[test]
    fn store_key_format() {
        let provider = McpOAuthProvider::new("my-server", "https://mcp.example.com");
        assert_eq!(provider.store_key(), "mcp:my-server");
    }
}
