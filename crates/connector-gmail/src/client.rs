use {
    async_trait::async_trait,
    futures::TryStreamExt,
    moltis_oauth::{OAuthConfig, OAuthFlow},
    secrecy::{ExposeSecret, Secret},
    serde::de::DeserializeOwned,
    std::{path::Path, sync::Arc, time::Duration},
    tokio::io::AsyncReadExt,
    url::Url,
};

use crate::{
    AuthorizedUserToken, GmailApiMessage, GmailConnectorError, GmailMessagePage, GmailProfile,
    Result,
};

const GMAIL_API_BASE: &str = "https://gmail.googleapis.com/gmail/v1/";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const MAX_CREDENTIAL_BYTES: u64 = 1024 * 1024;
const MAX_PROFILE_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_LIST_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_MESSAGE_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

#[async_trait]
pub trait GmailClient: Send + Sync {
    async fn profile(&self) -> Result<GmailProfile>;

    async fn list_messages(
        &self,
        query: &str,
        page_token: Option<&str>,
        max_results: u32,
    ) -> Result<GmailMessagePage>;

    async fn get_message(&self, id: &str) -> Result<GmailApiMessage>;
}

#[async_trait]
trait AccessTokenProvider: Send + Sync {
    async fn access_token(&self) -> Result<Secret<String>>;
}

struct OAuthAccessTokenProvider {
    credential: AuthorizedUserToken,
}

#[async_trait]
impl AccessTokenProvider for OAuthAccessTokenProvider {
    async fn access_token(&self) -> Result<Secret<String>> {
        let config = OAuthConfig {
            client_id: self.credential.client_id.clone(),
            client_secret: Some(self.credential.client_secret.clone()),
            auth_url: GOOGLE_AUTH_URL.to_owned(),
            token_url: GOOGLE_TOKEN_URL.to_owned(),
            redirect_uri: String::new(),
            resource: None,
            scopes: self.credential.scopes.clone(),
            extra_auth_params: Vec::new(),
            device_flow: false,
        };
        tokio::time::timeout(
            REQUEST_TIMEOUT,
            OAuthFlow::new(config).refresh(self.credential.refresh_token.expose_secret()),
        )
        .await
        .map_err(|_| GmailConnectorError::Timeout)?
        .map(|tokens| tokens.access_token)
        .map_err(GmailConnectorError::OAuth)
    }
}

pub struct NativeGmailClient {
    client: reqwest::Client,
    base_url: Url,
    tokens: Arc<dyn AccessTokenProvider>,
    access_token: tokio::sync::OnceCell<Secret<String>>,
    request_timeout: Duration,
}

impl NativeGmailClient {
    pub async fn from_authorized_user_file(path: &Path) -> Result<Self> {
        let credential = read_authorized_user_file(path).await?;
        let base_url = Url::parse(GMAIL_API_BASE).map_err(|error| {
            GmailConnectorError::ServerResponse(format!("invalid fixed Gmail API URL: {error}"))
        })?;
        Ok(Self {
            client: moltis_common::http_client::build_default_http_client(),
            base_url,
            tokens: Arc::new(OAuthAccessTokenProvider { credential }),
            access_token: tokio::sync::OnceCell::new(),
            request_timeout: REQUEST_TIMEOUT,
        })
    }

    #[cfg(test)]
    fn with_test_dependencies(
        client: reqwest::Client,
        base_url: Url,
        tokens: Arc<dyn AccessTokenProvider>,
        request_timeout: Duration,
    ) -> Self {
        Self {
            client,
            base_url,
            tokens,
            access_token: tokio::sync::OnceCell::new(),
            request_timeout,
        }
    }

    fn endpoint(&self, segments: &[&str]) -> Result<Url> {
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .map_err(|()| {
                GmailConnectorError::ServerResponse(
                    "Gmail API base URL cannot contain path segments".to_owned(),
                )
            })?
            .pop_if_empty()
            .extend(segments);
        Ok(url)
    }

    async fn send_json<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
        max_bytes: usize,
    ) -> Result<T> {
        let token = self
            .access_token
            .get_or_try_init(|| self.tokens.access_token())
            .await?;
        let response = request
            .timeout(self.request_timeout)
            .bearer_auth(token.expose_secret())
            .send()
            .await
            .map_err(GmailConnectorError::Http)?;
        let status = response.status();
        if !status.is_success() {
            return Err(GmailConnectorError::ApiStatus(status.as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > max_bytes as u64)
        {
            return Err(GmailConnectorError::ServerResponse(
                "Gmail API response exceeds connector limit".to_owned(),
            ));
        }

        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.try_next().await.map_err(GmailConnectorError::Http)? {
            let next_len = bytes.len().checked_add(chunk.len()).ok_or_else(|| {
                GmailConnectorError::ServerResponse(
                    "Gmail API response exceeds connector limit".to_owned(),
                )
            })?;
            if next_len > max_bytes {
                return Err(GmailConnectorError::ServerResponse(
                    "Gmail API response exceeds connector limit".to_owned(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|error| {
            GmailConnectorError::ServerResponse(format!("invalid Gmail API JSON: {error}"))
        })
    }
}

#[async_trait]
impl GmailClient for NativeGmailClient {
    async fn profile(&self) -> Result<GmailProfile> {
        let url = self.endpoint(&["users", "me", "profile"])?;
        self.send_json(self.client.get(url), MAX_PROFILE_RESPONSE_BYTES)
            .await
    }

    async fn list_messages(
        &self,
        query: &str,
        page_token: Option<&str>,
        max_results: u32,
    ) -> Result<GmailMessagePage> {
        if !(1..=500).contains(&max_results) {
            return Err(GmailConnectorError::ServerResponse(
                "Gmail list maxResults must be between 1 and 500".to_owned(),
            ));
        }
        let url = self.endpoint(&["users", "me", "messages"])?;
        let mut request = self
            .client
            .get(url)
            .query(&[("maxResults", max_results.to_string())]);
        if !query.is_empty() {
            request = request.query(&[("q", query)]);
        }
        if let Some(page_token) = page_token {
            request = request.query(&[("pageToken", page_token)]);
        }
        self.send_json(request, MAX_LIST_RESPONSE_BYTES).await
    }

    async fn get_message(&self, id: &str) -> Result<GmailApiMessage> {
        let url = self.endpoint(&["users", "me", "messages", id])?;
        let request = self.client.get(url).query(&[("format", "full")]);
        self.send_json(request, MAX_MESSAGE_RESPONSE_BYTES).await
    }
}

async fn read_authorized_user_file(path: &Path) -> Result<AuthorizedUserToken> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(GmailConnectorError::CredentialIo)?;
    let mut bytes = Vec::new();
    file.take(MAX_CREDENTIAL_BYTES + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(GmailConnectorError::CredentialIo)?;
    if bytes.len() as u64 > MAX_CREDENTIAL_BYTES {
        return Err(GmailConnectorError::Credential(
            "credential file exceeds 1 MiB",
        ));
    }
    AuthorizedUserToken::parse(&bytes)
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        std::sync::atomic::{AtomicUsize, Ordering},
        tokio::io::AsyncWriteExt,
    };

    struct FixedToken;

    #[async_trait]
    impl AccessTokenProvider for FixedToken {
        async fn access_token(&self) -> Result<Secret<String>> {
            Ok(Secret::new("token".to_owned()))
        }
    }

    struct CountingToken(AtomicUsize);

    #[async_trait]
    impl AccessTokenProvider for CountingToken {
        async fn access_token(&self) -> Result<Secret<String>> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(Secret::new("token".to_owned()))
        }
    }

    #[test]
    fn test_constructor_can_inject_base_and_client() -> Result<()> {
        let base_url = Url::parse("http://127.0.0.1:9/gmail/v1/").map_err(|error| {
            GmailConnectorError::ServerResponse(format!("test URL failed: {error}"))
        })?;
        let client = NativeGmailClient::with_test_dependencies(
            reqwest::Client::new(),
            base_url,
            Arc::new(FixedToken),
            REQUEST_TIMEOUT,
        );
        assert_eq!(
            client
                .endpoint(&["users", "me", "messages", "a/b"])?
                .as_str(),
            "http://127.0.0.1:9/gmail/v1/users/me/messages/a%2Fb"
        );
        Ok(())
    }

    #[tokio::test]
    async fn access_token_is_cached_for_client_lifetime() -> Result<()> {
        let provider = Arc::new(CountingToken(AtomicUsize::new(0)));
        let client = NativeGmailClient::with_test_dependencies(
            reqwest::Client::new(),
            Url::parse("http://127.0.0.1:9/gmail/v1/").map_err(|error| {
                GmailConnectorError::ServerResponse(format!("test URL failed: {error}"))
            })?,
            provider.clone(),
            REQUEST_TIMEOUT,
        );
        let _ = client
            .access_token
            .get_or_try_init(|| client.tokens.access_token())
            .await?;
        let _ = client
            .access_token
            .get_or_try_init(|| client.tokens.access_token())
            .await?;
        assert_eq!(provider.0.load(Ordering::Relaxed), 1);
        Ok(())
    }

    async fn one_response_server(response: String, delay: Duration) -> Result<Url> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(GmailConnectorError::CredentialIo)?;
        let address = listener
            .local_addr()
            .map_err(GmailConnectorError::CredentialIo)?;
        tokio::spawn(async move {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::time::sleep(delay).await;
            let _ = stream.write_all(response.as_bytes()).await;
        });
        Url::parse(&format!("http://{address}/gmail/v1/")).map_err(|error| {
            GmailConnectorError::ServerResponse(format!("test URL failed: {error}"))
        })
    }

    #[tokio::test]
    async fn http_status_and_declared_size_limits_are_enforced() -> Result<()> {
        let status_url = one_response_server(
            "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                .to_owned(),
            Duration::ZERO,
        )
        .await?;
        let status_client = NativeGmailClient::with_test_dependencies(
            reqwest::Client::new(),
            status_url,
            Arc::new(FixedToken),
            REQUEST_TIMEOUT,
        );
        assert!(matches!(
            status_client.profile().await,
            Err(GmailConnectorError::ApiStatus(503))
        ));

        let oversized_url = one_response_server(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                MAX_PROFILE_RESPONSE_BYTES + 1
            ),
            Duration::ZERO,
        )
        .await?;
        let oversized_client = NativeGmailClient::with_test_dependencies(
            reqwest::Client::new(),
            oversized_url,
            Arc::new(FixedToken),
            REQUEST_TIMEOUT,
        );
        assert!(matches!(
            oversized_client.profile().await,
            Err(GmailConnectorError::ServerResponse(_))
        ));
        Ok(())
    }

    #[tokio::test]
    async fn http_request_timeout_is_enforced() -> Result<()> {
        let url = one_response_server(
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_owned(),
            Duration::from_millis(100),
        )
        .await?;
        let client = NativeGmailClient::with_test_dependencies(
            reqwest::Client::new(),
            url,
            Arc::new(FixedToken),
            Duration::from_millis(10),
        );
        assert!(matches!(
            client.profile().await,
            Err(GmailConnectorError::Http(error)) if error.is_timeout()
        ));
        Ok(())
    }
}
