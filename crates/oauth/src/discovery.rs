//! OAuth 2.1 metadata discovery for MCP servers.
//!
//! Implements:
//! - RFC 9728: OAuth 2.0 Protected Resource Metadata
//! - RFC 8414: OAuth 2.0 Authorization Server Metadata
//! - RFC 7591: OAuth 2.0 Dynamic Client Registration

use {
    reqwest::Client,
    serde::{Deserialize, Serialize},
    tracing::{debug, info},
    url::Url,
};

use crate::{Error, Result};

// ── Protected Resource Metadata (RFC 9728) ─────────────────────────────────

/// Metadata returned by `/.well-known/oauth-protected-resource`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtectedResourceMetadata {
    /// The resource server's identifier (its base URL).
    pub resource: String,
    /// Authorization server(s) that can issue tokens for this resource.
    #[serde(default)]
    pub authorization_servers: Vec<String>,
    /// Scopes the resource requires.
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    /// Bearer token methods supported.
    #[serde(default)]
    pub bearer_methods_supported: Vec<String>,
    /// RFC 8707 resource indicators supported.
    #[serde(default)]
    pub resource_signing_alg_values_supported: Vec<String>,
}

/// Fetch protected resource metadata from `{resource_url}/.well-known/oauth-protected-resource`.
pub async fn fetch_resource_metadata(
    client: &Client,
    resource_url: &Url,
) -> Result<ProtectedResourceMetadata> {
    let well_known = build_well_known_url(resource_url, "oauth-protected-resource")?;

    debug!(url = %well_known, "fetching protected resource metadata");

    let resp = client
        .get(well_known.as_str())
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|source| Error::external("failed to fetch protected resource metadata", source))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::message(format!(
            "protected resource metadata returned HTTP {status}: {body}"
        )));
    }

    let meta: ProtectedResourceMetadata = resp
        .json()
        .await
        .map_err(|source| Error::external("failed to parse protected resource metadata", source))?;

    info!(resource = %meta.resource, servers = meta.authorization_servers.len(), "fetched resource metadata");

    Ok(meta)
}

// ── Authorization Server Metadata (RFC 8414) ───────────────────────────────

/// Metadata returned by `/.well-known/oauth-authorization-server`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationServerMetadata {
    /// The AS issuer identifier (a URL).
    pub issuer: String,
    /// URL of the authorization endpoint.
    pub authorization_endpoint: String,
    /// URL of the token endpoint.
    pub token_endpoint: String,
    /// URL of the dynamic client registration endpoint (RFC 7591).
    #[serde(default)]
    pub registration_endpoint: Option<String>,
    /// Scopes the AS supports.
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    /// Response types supported (`code` expected).
    #[serde(default)]
    pub response_types_supported: Vec<String>,
    /// Grant types supported.
    #[serde(default)]
    pub grant_types_supported: Vec<String>,
    /// PKCE challenge methods supported (`S256` expected).
    #[serde(default)]
    pub code_challenge_methods_supported: Vec<String>,
}

/// Fetch authorization server metadata from `{as_url}/.well-known/oauth-authorization-server`.
pub async fn fetch_as_metadata(
    client: &Client,
    as_url: &Url,
) -> Result<AuthorizationServerMetadata> {
    let well_known = build_well_known_url(as_url, "oauth-authorization-server")?;

    debug!(url = %well_known, "fetching authorization server metadata");

    let resp = client
        .get(well_known.as_str())
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|source| {
            Error::external("failed to fetch authorization server metadata", source)
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::message(format!(
            "authorization server metadata returned HTTP {status}: {body}"
        )));
    }

    let meta: AuthorizationServerMetadata = resp.json().await.map_err(|source| {
        Error::external("failed to parse authorization server metadata", source)
    })?;

    info!(issuer = %meta.issuer, "fetched AS metadata");

    Ok(meta)
}

// ── Dynamic Client Registration (RFC 7591) ─────────────────────────────────

/// Request body for dynamic client registration.
#[derive(Debug, Clone, Serialize)]
pub struct ClientRegistrationRequest {
    pub redirect_uris: Vec<String>,
    pub client_name: String,
    pub grant_types: Vec<String>,
    pub response_types: Vec<String>,
    pub token_endpoint_auth_method: String,
}

/// Successful registration response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientRegistrationResponse {
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub client_id_issued_at: Option<u64>,
    #[serde(default)]
    pub client_secret_expires_at: Option<u64>,
    #[serde(default)]
    pub redirect_uris: Vec<String>,
}

/// Perform dynamic client registration at the given endpoint.
pub async fn register_client(
    client: &Client,
    registration_endpoint: &str,
    redirect_uris: Vec<String>,
    client_name: &str,
) -> Result<ClientRegistrationResponse> {
    debug!(endpoint = %registration_endpoint, client_name, "registering dynamic OAuth client");

    let req = ClientRegistrationRequest {
        redirect_uris,
        client_name: client_name.to_string(),
        grant_types: vec![
            "authorization_code".to_string(),
            "refresh_token".to_string(),
        ],
        response_types: vec!["code".to_string()],
        token_endpoint_auth_method: "none".to_string(),
    };

    let resp = client
        .post(registration_endpoint)
        .header("Content-Type", "application/json")
        .json(&req)
        .send()
        .await
        .map_err(|source| Error::external("failed to register OAuth client", source))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::message(format!(
            "dynamic client registration returned HTTP {status}: {body}"
        )));
    }

    let reg: ClientRegistrationResponse = resp.json().await.map_err(|source| {
        Error::external("failed to parse client registration response", source)
    })?;

    info!(client_id = %reg.client_id, "registered dynamic OAuth client");

    Ok(reg)
}

// ── WWW-Authenticate header parsing ────────────────────────────────────────

/// Parse the `resource_metadata` URL from a `WWW-Authenticate: Bearer ...` header.
///
/// Example header: `Bearer realm="example", resource_metadata="https://example.com/.well-known/oauth-protected-resource"`
#[must_use]
pub fn parse_www_authenticate(header: &str) -> Option<String> {
    let stripped = header
        .strip_prefix("Bearer")
        .or_else(|| header.strip_prefix("bearer"))?;
    let stripped = stripped.trim_start();

    for part in stripped.split(',') {
        let part = part.trim();
        if let Some(value) = part
            .strip_prefix("resource_metadata=")
            .or_else(|| part.strip_prefix("resource_metadata ="))
        {
            let value = value.trim().trim_matches('"');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Build a `/.well-known/<suffix>` URL following RFC 8615 path conventions.
fn build_well_known_url(base: &Url, suffix: &str) -> Result<Url> {
    let mut url = base.clone();
    // Ensure path ends with /
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    url = url
        .join(&format!(".well-known/{suffix}"))
        .map_err(|source| {
            Error::external(
                format!("failed to build .well-known/{suffix} URL from {base}"),
                source,
            )
        })?;
    Ok(url)
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    // ── WWW-Authenticate parsing ───────────────────────────────────────

    #[test]
    fn parse_www_authenticate_basic() {
        let header = r#"Bearer resource_metadata="https://example.com/.well-known/oauth-protected-resource""#;
        let result = parse_www_authenticate(header);
        assert_eq!(
            result.as_deref(),
            Some("https://example.com/.well-known/oauth-protected-resource")
        );
    }

    #[test]
    fn parse_www_authenticate_with_realm() {
        let header = r#"Bearer realm="example", resource_metadata="https://ex.com/meta""#;
        let result = parse_www_authenticate(header);
        assert_eq!(result.as_deref(), Some("https://ex.com/meta"));
    }

    #[test]
    fn parse_www_authenticate_lowercase() {
        let header = r#"bearer resource_metadata="https://ex.com/meta""#;
        let result = parse_www_authenticate(header);
        assert_eq!(result.as_deref(), Some("https://ex.com/meta"));
    }

    #[test]
    fn parse_www_authenticate_no_resource_metadata() {
        let header = "Bearer realm=\"example\"";
        assert!(parse_www_authenticate(header).is_none());
    }

    #[test]
    fn parse_www_authenticate_not_bearer() {
        let header = "Basic realm=\"example\"";
        assert!(parse_www_authenticate(header).is_none());
    }

    #[test]
    fn parse_www_authenticate_empty() {
        assert!(parse_www_authenticate("").is_none());
    }

    #[test]
    fn parse_www_authenticate_with_space_around_equals() {
        let header = r#"Bearer resource_metadata ="https://ex.com/meta""#;
        let result = parse_www_authenticate(header);
        assert_eq!(result.as_deref(), Some("https://ex.com/meta"));
    }

    // ── Well-known URL building ────────────────────────────────────────

    #[test]
    fn build_well_known_url_basic() {
        let base = Url::parse("https://example.com").unwrap();
        let url = build_well_known_url(&base, "oauth-protected-resource").unwrap();
        assert_eq!(
            url.as_str(),
            "https://example.com/.well-known/oauth-protected-resource"
        );
    }

    #[test]
    fn build_well_known_url_with_path() {
        let base = Url::parse("https://example.com/mcp/v1").unwrap();
        let url = build_well_known_url(&base, "oauth-protected-resource").unwrap();
        assert_eq!(
            url.as_str(),
            "https://example.com/mcp/v1/.well-known/oauth-protected-resource"
        );
    }

    #[test]
    fn build_well_known_url_with_trailing_slash() {
        let base = Url::parse("https://example.com/").unwrap();
        let url = build_well_known_url(&base, "oauth-authorization-server").unwrap();
        assert_eq!(
            url.as_str(),
            "https://example.com/.well-known/oauth-authorization-server"
        );
    }

    // ── HTTP integration tests (with mockito) ──────────────────────────

    #[tokio::test]
    async fn fetch_resource_metadata_success() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/.well-known/oauth-protected-resource")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "resource": server.url(),
                    "authorization_servers": ["https://auth.example.com"]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let client = Client::new();
        let url = Url::parse(&server.url()).unwrap();
        let meta = fetch_resource_metadata(&client, &url).await.unwrap();

        assert_eq!(meta.resource, server.url());
        assert_eq!(meta.authorization_servers, vec!["https://auth.example.com"]);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn fetch_resource_metadata_not_found() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/.well-known/oauth-protected-resource")
            .with_status(404)
            .with_body("not found")
            .create_async()
            .await;

        let client = Client::new();
        let url = Url::parse(&server.url()).unwrap();
        let result = fetch_resource_metadata(&client, &url).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("404"));
    }

    #[tokio::test]
    async fn fetch_as_metadata_success() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/.well-known/oauth-authorization-server")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "issuer": server.url(),
                    "authorization_endpoint": format!("{}/authorize", server.url()),
                    "token_endpoint": format!("{}/token", server.url()),
                    "registration_endpoint": format!("{}/register", server.url()),
                    "code_challenge_methods_supported": ["S256"]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let client = Client::new();
        let url = Url::parse(&server.url()).unwrap();
        let meta = fetch_as_metadata(&client, &url).await.unwrap();

        assert_eq!(meta.issuer, server.url());
        assert!(meta.authorization_endpoint.ends_with("/authorize"));
        assert!(meta.token_endpoint.ends_with("/token"));
        assert_eq!(
            meta.registration_endpoint.as_deref(),
            Some(format!("{}/register", server.url()).as_str())
        );
        assert_eq!(meta.code_challenge_methods_supported, vec!["S256"]);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn fetch_as_metadata_malformed_json() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/.well-known/oauth-authorization-server")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("{not valid json}")
            .create_async()
            .await;

        let client = Client::new();
        let url = Url::parse(&server.url()).unwrap();
        let result = fetch_as_metadata(&client, &url).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn register_client_success() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/register")
            .with_status(201)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "client_id": "abc123",
                    "client_secret": "secret456",
                    "client_id_issued_at": 1700000000u64,
                    "redirect_uris": ["http://127.0.0.1:9999/auth/callback"]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let client = Client::new();
        let reg = register_client(
            &client,
            &format!("{}/register", server.url()),
            vec!["http://127.0.0.1:9999/auth/callback".to_string()],
            "moltis-test",
        )
        .await
        .unwrap();

        assert_eq!(reg.client_id, "abc123");
        assert_eq!(reg.client_secret.as_deref(), Some("secret456"));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn register_client_no_secret() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/register")
            .with_status(201)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "client_id": "pub-only",
                    "redirect_uris": ["http://127.0.0.1:9999/auth/callback"]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let client = Client::new();
        let reg = register_client(
            &client,
            &format!("{}/register", server.url()),
            vec!["http://127.0.0.1:9999/auth/callback".to_string()],
            "moltis",
        )
        .await
        .unwrap();

        assert_eq!(reg.client_id, "pub-only");
        assert!(reg.client_secret.is_none());
    }

    #[tokio::test]
    async fn register_client_error_response() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/register")
            .with_status(400)
            .with_body(r#"{"error":"invalid_client_metadata"}"#)
            .create_async()
            .await;

        let client = Client::new();
        let result = register_client(
            &client,
            &format!("{}/register", server.url()),
            vec!["http://127.0.0.1:9999/auth/callback".to_string()],
            "moltis",
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("400"));
    }

    #[tokio::test]
    async fn fetch_resource_metadata_network_failure() {
        let client = Client::builder()
            .timeout(std::time::Duration::from_millis(100))
            .build()
            .unwrap();
        let url = Url::parse("http://127.0.0.1:1").unwrap();
        let result = fetch_resource_metadata(&client, &url).await;
        assert!(result.is_err());
    }
}
