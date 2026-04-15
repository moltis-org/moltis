use std::time::{Duration, Instant};

use {
    anyhow::Result,
    secrecy::{ExposeSecret, Secret},
    serde::Deserialize,
};

use crate::config::MsTeamsAccountConfig;

#[derive(Clone)]
pub struct CachedAccessToken {
    pub token: Secret<String>,
    pub expires_at: Instant,
}

impl CachedAccessToken {
    fn is_valid(&self) -> bool {
        let refresh_skew = Duration::from_secs(60);
        self.expires_at > Instant::now() + refresh_skew
    }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: Option<u64>,
}

/// Default OAuth scope for Microsoft Graph API.
pub const GRAPH_OAUTH_SCOPE: &str = "https://graph.microsoft.com/.default";

pub async fn get_access_token(
    client: &reqwest::Client,
    config: &MsTeamsAccountConfig,
    cache: &tokio::sync::Mutex<Option<CachedAccessToken>>,
) -> Result<Secret<String>> {
    acquire_token(
        client,
        &config.app_id,
        config.app_password.expose_secret(),
        &config.oauth_tenant,
        &config.oauth_scope,
        cache,
    )
    .await
}

/// Acquire a token scoped to Microsoft Graph API.
pub async fn get_graph_token(
    client: &reqwest::Client,
    config: &MsTeamsAccountConfig,
    cache: &tokio::sync::Mutex<Option<CachedAccessToken>>,
) -> Result<Secret<String>> {
    let tenant = config
        .graph_tenant_id
        .as_deref()
        .unwrap_or(&config.oauth_tenant);
    acquire_token(
        client,
        &config.app_id,
        config.app_password.expose_secret(),
        tenant,
        GRAPH_OAUTH_SCOPE,
        cache,
    )
    .await
}

async fn acquire_token(
    client: &reqwest::Client,
    client_id: &str,
    client_secret: &str,
    tenant: &str,
    scope: &str,
    cache: &tokio::sync::Mutex<Option<CachedAccessToken>>,
) -> Result<Secret<String>> {
    {
        let guard = cache.lock().await;
        if let Some(token) = guard.as_ref()
            && token.is_valid()
        {
            return Ok(token.token.clone());
        }
    }

    let token_url = format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token",);
    let form = [
        ("grant_type", "client_credentials"),
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("scope", scope),
    ];

    let resp = client.post(token_url).form(&form).send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("token request for scope '{scope}' failed ({status}): {body}");
    }

    let body: TokenResponse = resp.json().await?;
    let ttl = body.expires_in.unwrap_or(3600).max(120);
    let cached = CachedAccessToken {
        token: Secret::new(body.access_token),
        expires_at: Instant::now() + Duration::from_secs(ttl),
    };
    let token = cached.token.clone();

    let mut guard = cache.lock().await;
    *guard = Some(cached);
    Ok(token)
}
