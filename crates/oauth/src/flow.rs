use {anyhow::Result, secrecy::Secret, url::Url};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, oauth as oauth_metrics};

use crate::{
    pkce::{generate_pkce, generate_state},
    types::{OAuthConfig, OAuthTokens, PkceChallenge},
};

/// Manages the OAuth 2.0 authorization code flow with PKCE.
pub struct OAuthFlow {
    config: OAuthConfig,
    client: reqwest::Client,
}

/// Result of starting the OAuth flow.
pub struct AuthorizationRequest {
    pub url: String,
    pub pkce: PkceChallenge,
    pub state: String,
}

impl OAuthFlow {
    pub fn new(config: OAuthConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    /// Build the authorization URL and generate PKCE + state.
    pub fn start(&self) -> Result<AuthorizationRequest> {
        #[cfg(feature = "metrics")]
        counter!(oauth_metrics::FLOW_STARTS_TOTAL).increment(1);

        let pkce = generate_pkce();
        let state = generate_state();

        let mut url = Url::parse(&self.config.auth_url)
            .map_err(|e| anyhow::anyhow!("invalid auth_url: {e}"))?;
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.config.client_id)
            .append_pair("redirect_uri", &self.config.redirect_uri)
            .append_pair("code_challenge", &pkce.challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state);

        if !self.config.scopes.is_empty() {
            url.query_pairs_mut()
                .append_pair("scope", &self.config.scopes.join(" "));
        }

        for (key, value) in &self.config.extra_auth_params {
            url.query_pairs_mut().append_pair(key, value);
        }

        // Always include originator
        url.query_pairs_mut().append_pair("originator", "pi");

        Ok(AuthorizationRequest {
            url: url.to_string(),
            pkce,
            state,
        })
    }

    /// Exchange an authorization code for tokens.
    pub async fn exchange(&self, code: &str, verifier: &str) -> Result<OAuthTokens> {
        #[cfg(feature = "metrics")]
        counter!(oauth_metrics::CODE_EXCHANGE_TOTAL).increment(1);

        let result = self
            .client
            .post(&self.config.token_url)
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", &self.config.redirect_uri),
                ("client_id", &self.config.client_id),
                ("code_verifier", verifier),
            ])
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await;

        match result {
            Ok(resp) => {
                #[cfg(feature = "metrics")]
                counter!(oauth_metrics::FLOW_COMPLETIONS_TOTAL).increment(1);
                parse_token_response(&resp)
            },
            Err(e) => {
                #[cfg(feature = "metrics")]
                counter!(oauth_metrics::CODE_EXCHANGE_ERRORS_TOTAL).increment(1);
                Err(e.into())
            },
        }
    }

    /// Refresh an access token using a refresh token.
    pub async fn refresh(&self, refresh_token: &str) -> Result<OAuthTokens> {
        #[cfg(feature = "metrics")]
        counter!(oauth_metrics::TOKEN_REFRESH_TOTAL).increment(1);

        let result = self
            .client
            .post(&self.config.token_url)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", &self.config.client_id),
            ])
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await;

        match result {
            Ok(resp) => parse_token_response(&resp),
            Err(e) => {
                #[cfg(feature = "metrics")]
                counter!(oauth_metrics::TOKEN_REFRESH_FAILURES_TOTAL).increment(1);
                Err(e.into())
            },
        }
    }
}

fn parse_token_response(resp: &serde_json::Value) -> Result<OAuthTokens> {
    let access_token = resp["access_token"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing access_token in response"))?
        .to_string();

    let refresh_token = resp["refresh_token"].as_str().map(|s| s.to_string());

    let expires_at = resp["expires_in"].as_u64().and_then(|secs| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs() + secs)
    });

    Ok(OAuthTokens {
        access_token: Secret::new(access_token),
        refresh_token: refresh_token.map(Secret::new),
        expires_at,
    })
}
