//! HTTP authentication against `/api/auth/*`.
//!
//! Native clients log in once with the daemon password, use the short-lived
//! session cookie to mint a scoped `mk_` API key, and from then on present
//! only that key (stored in the OS keychain by the caller).

use {
    reqwest::{StatusCode, header},
    secrecy::{ExposeSecret, SecretString},
    serde::{Deserialize, Serialize},
};

use crate::{
    endpoint::Endpoint,
    error::{Error, Result},
    tls::client_config,
};

/// Name of the daemon's session cookie.
pub const SESSION_COOKIE: &str = "moltis_session";

/// Scopes requested for desktop API keys: chat and approvals, never admin.
pub const DESKTOP_SCOPES: &[&str] = &["operator.read", "operator.write", "operator.approvals"];

/// `GET /api/auth/status`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AuthStatus {
    pub setup_required: bool,
    pub setup_complete: bool,
    pub authenticated: bool,
    pub auth_disabled: bool,
    pub has_password: bool,
    pub has_passkeys: bool,
    pub localhost_only: bool,
    pub webauthn_available: bool,
}

impl AuthStatus {
    /// Whether a password login can produce an API key.
    #[must_use]
    pub fn can_login_with_password(&self) -> bool {
        self.setup_complete && self.has_password && !self.auth_disabled
    }
}

#[derive(Deserialize)]
struct CreatedKey {
    key: String,
}

/// HTTP client bound to one endpoint, sharing its TLS trust.
#[derive(Clone)]
pub struct AuthClient {
    http: reqwest::Client,
    endpoint: Endpoint,
}

impl AuthClient {
    pub fn new(endpoint: Endpoint) -> Result<Self> {
        let mut builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none());
        if endpoint.is_tls() {
            let config = client_config(&endpoint.tls)?;
            builder = builder.tls_backend_preconfigured((*config).clone());
        }
        Ok(Self {
            http: builder.build()?,
            endpoint,
        })
    }

    #[must_use]
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Unauthenticated liveness check.
    pub async fn health(&self) -> Result<()> {
        let response = self
            .http
            .get(self.endpoint.http_url("/health"))
            .send()
            .await?;
        check(response).await.map(drop)
    }

    pub async fn status(&self, api_key: Option<&SecretString>) -> Result<AuthStatus> {
        let mut request = self.http.get(self.endpoint.http_url("/api/auth/status"));
        if let Some(key) = api_key {
            request = request.bearer_auth(key.expose_secret());
        }
        Ok(check(request.send().await?).await?.json().await?)
    }

    /// Log in with the daemon password; returns the session token.
    pub async fn login(&self, password: &SecretString) -> Result<SecretString> {
        let response = self
            .http
            .post(self.endpoint.http_url("/api/auth/login"))
            .json(&serde_json::json!({ "password": password.expose_secret() }))
            .send()
            .await?;
        if response.status() == StatusCode::UNAUTHORIZED {
            return Err(Error::Unauthorized("invalid password".into()));
        }
        let response = check(response).await?;
        response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .find_map(session_token)
            .map(SecretString::new)
            .ok_or_else(|| Error::Unauthorized("login returned no session".into()))
    }

    /// Mint an API key with `scopes` using a session from [`Self::login`].
    pub async fn create_api_key(
        &self,
        session: &SecretString,
        label: &str,
        scopes: &[&str],
    ) -> Result<SecretString> {
        let response = self
            .http
            .post(self.endpoint.http_url("/api/auth/api-keys"))
            .header(
                header::COOKIE,
                format!("{SESSION_COOKIE}={}", session.expose_secret()),
            )
            .json(&serde_json::json!({ "label": label, "scopes": scopes }))
            .send()
            .await?;
        let created: CreatedKey = check(response).await?.json().await?;
        Ok(SecretString::new(created.key))
    }

    /// End a session from [`Self::login`]. Best effort.
    pub async fn logout(&self, session: &SecretString) -> Result<()> {
        let response = self
            .http
            .post(self.endpoint.http_url("/api/auth/logout"))
            .header(
                header::COOKIE,
                format!("{SESSION_COOKIE}={}", session.expose_secret()),
            )
            .send()
            .await?;
        check(response).await.map(drop)
    }

    /// Log in, mint a desktop API key, and log out again.
    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(skip(self, password), fields(endpoint = %self.endpoint))
    )]
    pub async fn exchange_password_for_key(
        &self,
        password: &SecretString,
        label: &str,
    ) -> Result<SecretString> {
        let session = self.login(password).await?;
        let key = self.create_api_key(&session, label, DESKTOP_SCOPES).await;
        let _ = self.logout(&session).await;
        key
    }
}

async fn check(response: reqwest::Response) -> Result<reqwest::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    Err(Error::Http {
        status: status.as_u16(),
        body: body.chars().take(500).collect(),
    })
}

/// The session token out of one `Set-Cookie` header value.
fn session_token(set_cookie: &str) -> Option<String> {
    let first = set_cookie.split(';').next()?.trim();
    let (name, value) = first.split_once('=')?;
    (name == SESSION_COOKIE && !value.is_empty()).then(|| value.to_string())
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use {
        axum::{
            Json, Router,
            http::{HeaderMap, StatusCode},
            response::IntoResponse,
            routing::{get, post},
        },
        serde_json::{Value, json},
    };

    use {super::*, crate::tls::TlsTrust};

    #[test]
    fn parses_session_cookie() {
        assert_eq!(
            session_token("moltis_session=abc; Path=/; HttpOnly").as_deref(),
            Some("abc")
        );
        assert_eq!(session_token("other=1; Path=/"), None);
        assert_eq!(session_token("moltis_session=; Max-Age=0"), None);
    }

    #[test]
    fn password_login_needs_setup() {
        let status = AuthStatus {
            setup_complete: true,
            has_password: true,
            ..AuthStatus::default()
        };
        assert!(status.can_login_with_password());
        assert!(
            !AuthStatus {
                auth_disabled: true,
                ..status.clone()
            }
            .can_login_with_password()
        );
        assert!(!AuthStatus::default().can_login_with_password());
    }

    #[derive(Default)]
    struct Seen {
        key_request: Option<(String, Value)>,
        logged_out: bool,
    }

    async fn serve(seen: Arc<Mutex<Seen>>) -> Endpoint {
        let login = post(|Json(body): Json<Value>| async move {
            if body["password"] == "secret" {
                (
                    [(
                        header::SET_COOKIE,
                        "moltis_session=tok123; Path=/; HttpOnly",
                    )],
                    "ok",
                )
                    .into_response()
            } else {
                (StatusCode::UNAUTHORIZED, "invalid password").into_response()
            }
        });
        let key_seen = Arc::clone(&seen);
        let keys = post(
            move |headers: HeaderMap, Json(body): Json<Value>| async move {
                let cookie = headers
                    .get(header::COOKIE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default()
                    .to_string();
                key_seen.lock().unwrap().key_request = Some((cookie, body));
                Json(json!({"id": 1, "key": "mk_new"}))
            },
        );
        let out_seen = Arc::clone(&seen);
        let logout = post(move || async move {
            out_seen.lock().unwrap().logged_out = true;
            "bye"
        });
        let app = Router::new()
            .route("/health", get(|| async { "ok" }))
            .route(
                "/api/auth/status",
                get(|| async {
                    Json(json!({"setup_complete": true, "has_password": true, "extra": 1}))
                }),
            )
            .route("/api/auth/login", login)
            .route("/api/auth/api-keys", keys)
            .route("/api/auth/logout", logout);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Endpoint::parse(&addr.to_string(), TlsTrust::System).unwrap()
    }

    #[tokio::test]
    async fn password_exchange_mints_scoped_key() {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let client = AuthClient::new(serve(Arc::clone(&seen)).await).unwrap();
        client.health().await.unwrap();
        assert!(client.status(None).await.unwrap().can_login_with_password());

        let key = client
            .exchange_password_for_key(&SecretString::new("secret".into()), "Moltis Desktop")
            .await
            .unwrap();
        assert_eq!(key.expose_secret(), "mk_new");
        let seen = seen.lock().unwrap();
        let (cookie, body) = seen.key_request.clone().unwrap();
        assert_eq!(cookie, "moltis_session=tok123");
        assert_eq!(body["label"], "Moltis Desktop");
        assert_eq!(body["scopes"], json!(DESKTOP_SCOPES));
        assert!(seen.logged_out);
    }

    #[tokio::test]
    async fn wrong_password_is_unauthorized() {
        let client = AuthClient::new(serve(Arc::new(Mutex::new(Seen::default()))).await).unwrap();
        let error = client
            .login(&SecretString::new("nope".into()))
            .await
            .unwrap_err();
        assert!(error.is_auth());
    }
}
