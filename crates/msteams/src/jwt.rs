//! Bot Framework JWT token validation.
//!
//! Validates Bearer tokens sent by the Teams Bot Connector using JWKS
//! (JSON Web Key Sets) from Microsoft's well-known endpoints. Supports both
//! the Bot Framework issuer and the Entra (Azure AD) issuer.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use {
    jsonwebtoken::{
        Algorithm, DecodingKey, Validation,
        jwk::{AlgorithmParameters, JwkSet},
    },
    tracing::{debug, warn},
};

/// Bot Framework JWKS endpoint.
const BOT_FRAMEWORK_JWKS_URL: &str = "https://login.botframework.com/v1/.well-known/keys";

/// Entra (Azure AD) common JWKS endpoint.
const ENTRA_JWKS_URL: &str = "https://login.microsoftonline.com/common/discovery/v2.0/keys";

/// Bot Framework token issuer.
const BOT_FRAMEWORK_ISSUER: &str = "https://api.botframework.com";

/// How long to cache JWKS before re-fetching.
const JWKS_CACHE_TTL: Duration = Duration::from_secs(300);

/// Cached JWKS key set with TTL.
struct CachedJwks {
    keys: JwkSet,
    fetched_at: Instant,
}

impl CachedJwks {
    fn is_fresh(&self) -> bool {
        self.fetched_at.elapsed() < JWKS_CACHE_TTL
    }
}

/// Validates Bot Framework JWT tokens from the `Authorization` header.
pub struct BotFrameworkJwtValidator {
    app_id: String,
    tenant_id: Option<String>,
    http: reqwest::Client,
    bot_framework_jwks: Arc<tokio::sync::RwLock<Option<CachedJwks>>>,
    entra_jwks: Arc<tokio::sync::RwLock<Option<CachedJwks>>>,
}

impl BotFrameworkJwtValidator {
    pub fn new(app_id: String, tenant_id: Option<String>, http: reqwest::Client) -> Self {
        Self {
            app_id,
            tenant_id,
            http,
            bot_framework_jwks: Arc::new(tokio::sync::RwLock::new(None)),
            entra_jwks: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    /// Validate a JWT from the `Authorization: Bearer <token>` header.
    ///
    /// Returns `true` if the token is valid (signature, audience, issuer,
    /// expiry all check out).
    pub async fn validate(&self, auth_header: &str) -> bool {
        let token = match auth_header.strip_prefix("Bearer ") {
            Some(t) => t.trim(),
            None => {
                debug!("Teams JWT: missing Bearer prefix");
                return false;
            },
        };

        if token.is_empty() {
            debug!("Teams JWT: empty token");
            return false;
        }

        // Decode header to find the key ID.
        let header = match jsonwebtoken::decode_header(token) {
            Ok(h) => h,
            Err(e) => {
                debug!("Teams JWT: invalid header: {e}");
                return false;
            },
        };

        let kid = match header.kid.as_deref() {
            Some(k) => k,
            None => {
                debug!("Teams JWT: no kid in header");
                return false;
            },
        };

        // Try Bot Framework issuer first.
        if let Some(result) = self
            .try_validate_with_jwks(
                token,
                kid,
                BOT_FRAMEWORK_JWKS_URL,
                &self.bot_framework_jwks,
                BOT_FRAMEWORK_ISSUER,
            )
            .await
        {
            return result;
        }

        // Try Entra issuer.
        let entra_issuer = self.entra_issuer();
        if let Some(result) = self
            .try_validate_with_jwks(token, kid, ENTRA_JWKS_URL, &self.entra_jwks, &entra_issuer)
            .await
        {
            return result;
        }

        debug!("Teams JWT: kid '{kid}' not found in any JWKS");
        false
    }

    async fn try_validate_with_jwks(
        &self,
        token: &str,
        kid: &str,
        jwks_url: &str,
        cache: &Arc<tokio::sync::RwLock<Option<CachedJwks>>>,
        expected_issuer: &str,
    ) -> Option<bool> {
        let jwks = self.get_or_fetch_jwks(jwks_url, cache).await?;

        let jwk = jwks
            .keys
            .iter()
            .find(|k| k.common.key_id.as_deref() == Some(kid))?;

        let decoding_key = match &jwk.algorithm {
            AlgorithmParameters::RSA(rsa) => {
                DecodingKey::from_rsa_components(&rsa.n, &rsa.e).ok()?
            },
            _ => {
                debug!("Teams JWT: unsupported algorithm for kid '{kid}'");
                return Some(false);
            },
        };

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[&self.app_id]);
        validation.set_issuer(&[expected_issuer]);
        // Also accept common Entra issuers.
        validation.validate_exp = true;

        match jsonwebtoken::decode::<serde_json::Value>(token, &decoding_key, &validation) {
            Ok(_) => Some(true),
            Err(e) => {
                debug!("Teams JWT: validation failed with issuer {expected_issuer}: {e}");
                Some(false)
            },
        }
    }

    async fn get_or_fetch_jwks(
        &self,
        url: &str,
        cache: &Arc<tokio::sync::RwLock<Option<CachedJwks>>>,
    ) -> Option<Arc<JwkSet>> {
        // Check cache.
        {
            let guard = cache.read().await;
            if let Some(cached) = guard.as_ref()
                && cached.is_fresh()
            {
                return Some(Arc::new(cached.keys.clone()));
            }
        }

        // Fetch new JWKS. On failure, fall back to stale cache if available
        // so that a transient outage doesn't reject all legitimate requests.
        let jwks = match self.fetch_jwks(url).await {
            Ok(keys) => keys,
            Err(e) => {
                let guard = cache.read().await;
                if let Some(stale) = guard.as_ref() {
                    warn!("Teams JWT: JWKS refresh failed, using stale cache: {e}");
                    return Some(Arc::new(stale.keys.clone()));
                }
                warn!("Teams JWT: failed to fetch JWKS from {url} (no cache): {e}");
                return None;
            },
        };

        let result = Arc::new(jwks.clone());

        // Update cache.
        let mut guard = cache.write().await;
        *guard = Some(CachedJwks {
            keys: jwks,
            fetched_at: Instant::now(),
        });

        Some(result)
    }

    async fn fetch_jwks(&self, url: &str) -> anyhow::Result<JwkSet> {
        let resp = self
            .http
            .get(url)
            .timeout(Duration::from_secs(10))
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("JWKS fetch returned {}", resp.status());
        }

        let jwks: JwkSet = resp.json().await?;
        Ok(jwks)
    }

    fn entra_issuer(&self) -> String {
        match self.tenant_id.as_deref() {
            Some(tid) if !tid.is_empty() && tid != "botframework.com" => {
                format!("https://sts.windows.net/{tid}/")
            },
            _ => "https://sts.windows.net/botframework.com/".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entra_issuer_with_tenant() {
        let v = BotFrameworkJwtValidator::new(
            "app".into(),
            Some("my-tenant-id".into()),
            reqwest::Client::new(),
        );
        assert_eq!(v.entra_issuer(), "https://sts.windows.net/my-tenant-id/");
    }

    #[test]
    fn entra_issuer_default() {
        let v = BotFrameworkJwtValidator::new("app".into(), None, reqwest::Client::new());
        assert_eq!(
            v.entra_issuer(),
            "https://sts.windows.net/botframework.com/"
        );
    }

    #[test]
    fn entra_issuer_botframework_tenant() {
        let v = BotFrameworkJwtValidator::new(
            "app".into(),
            Some("botframework.com".into()),
            reqwest::Client::new(),
        );
        assert_eq!(
            v.entra_issuer(),
            "https://sts.windows.net/botframework.com/"
        );
    }

    #[tokio::test]
    async fn validate_rejects_empty_bearer() {
        let v = BotFrameworkJwtValidator::new("app".into(), None, reqwest::Client::new());
        assert!(!v.validate("Bearer ").await);
    }

    #[tokio::test]
    async fn validate_rejects_no_bearer_prefix() {
        let v = BotFrameworkJwtValidator::new("app".into(), None, reqwest::Client::new());
        assert!(!v.validate("Basic abc123").await);
    }

    #[tokio::test]
    async fn validate_rejects_garbage_token() {
        let v = BotFrameworkJwtValidator::new("app".into(), None, reqwest::Client::new());
        assert!(!v.validate("Bearer not.a.jwt").await);
    }
}
