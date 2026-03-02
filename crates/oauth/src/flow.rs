use {
    base64::{
        Engine,
        engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    },
    secrecy::Secret,
    url::Url,
};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, oauth as oauth_metrics};

use crate::{
    Error, Result,
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
            .map_err(|source| Error::external(format!("invalid auth_url: {source}"), source))?;
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.config.client_id)
            .append_pair("redirect_uri", &self.config.redirect_uri)
            .append_pair("code_challenge", &pkce.challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state);

        if let Some(resource) = &self.config.resource {
            url.query_pairs_mut().append_pair("resource", resource);
        }

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

        let mut form = vec![
            ("grant_type".to_string(), "authorization_code".to_string()),
            ("code".to_string(), code.to_string()),
            ("redirect_uri".to_string(), self.config.redirect_uri.clone()),
            ("client_id".to_string(), self.config.client_id.clone()),
            ("code_verifier".to_string(), verifier.to_string()),
        ];
        if let Some(resource) = &self.config.resource {
            form.push(("resource".to_string(), resource.clone()));
        }

        let result = self
            .client
            .post(&self.config.token_url)
            .form(&form)
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

        let mut form = vec![
            ("grant_type".to_string(), "refresh_token".to_string()),
            ("refresh_token".to_string(), refresh_token.to_string()),
            ("client_id".to_string(), self.config.client_id.clone()),
        ];
        if let Some(resource) = &self.config.resource {
            form.push(("resource".to_string(), resource.clone()));
        }

        let result = self
            .client
            .post(&self.config.token_url)
            .form(&form)
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
        .ok_or_else(|| Error::message("missing access_token in response"))?
        .to_string();

    let refresh_token = resp["refresh_token"].as_str().map(|s| s.to_string());
    let id_token = resp["id_token"].as_str().map(|s| s.to_string());
    let account_id = extract_account_id_from_tokens(&access_token, id_token.as_deref());

    let expires_at = resp["expires_in"].as_u64().and_then(|secs| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs() + secs)
    });

    Ok(OAuthTokens {
        access_token: Secret::new(access_token),
        refresh_token: refresh_token.map(Secret::new),
        id_token: id_token.map(Secret::new),
        account_id,
        expires_at,
    })
}

fn extract_account_id_from_tokens(access_token: &str, id_token: Option<&str>) -> Option<String> {
    id_token
        .and_then(extract_account_id_from_jwt)
        .or_else(|| extract_account_id_from_jwt(access_token))
}

fn extract_account_id_from_jwt(token: &str) -> Option<String> {
    let claims = parse_jwt_claims(token)?;
    extract_account_id_from_claims(&claims)
}

fn parse_jwt_claims(token: &str) -> Option<serde_json::Value> {
    let payload_b64 = token.split('.').nth(1)?;
    let payload = URL_SAFE_NO_PAD.decode(payload_b64).or_else(|_| {
        let padded = match payload_b64.len() % 4 {
            2 => format!("{payload_b64}=="),
            3 => format!("{payload_b64}="),
            _ => payload_b64.to_string(),
        };
        STANDARD.decode(padded)
    });
    let payload = payload.ok()?;
    serde_json::from_slice(&payload).ok()
}

fn extract_account_id_from_claims(claims: &serde_json::Value) -> Option<String> {
    claims
        .get("chatgpt_account_id")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            claims
                .get("https://api.openai.com/auth")
                .and_then(|v| v.get("chatgpt_account_id"))
                .and_then(serde_json::Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
        })
        .or_else(|| {
            claims
                .get("organizations")
                .and_then(serde_json::Value::as_array)
                .and_then(|arr| arr.first())
                .and_then(|v| v.get("id"))
                .and_then(serde_json::Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
        })
}
