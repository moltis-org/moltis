use {
    async_trait::async_trait,
    futures::TryStreamExt,
    moltis_oauth::{OAuthConfig, OAuthFlow},
    secrecy::{ExposeSecret, Secret},
    serde::de::DeserializeOwned,
    std::{sync::Arc, time::Duration},
    url::Url,
};

use crate::{
    Result, TeslaAccountConfig, TeslaApiVehicle, TeslaConnectorError, TeslaResponse,
    TeslaVehicleData,
};

const MAX_VEHICLE_LIST_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_VEHICLE_DATA_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// Read-only scopes. Location is requested separately because Tesla gates
/// precise coordinates behind its own scope.
const READ_SCOPES: [&str; 3] = ["openid", "offline_access", "vehicle_device_data"];

#[async_trait]
pub trait TeslaClient: Send + Sync {
    async fn list_vehicles(&self) -> Result<Vec<TeslaApiVehicle>>;

    /// `endpoints` is the pre-joined Fleet API `endpoints` parameter.
    async fn vehicle_data(&self, vin: &str, endpoints: &str) -> Result<TeslaVehicleData>;
}

#[async_trait]
trait AccessTokenProvider: Send + Sync {
    async fn access_token(&self) -> Result<Secret<String>>;
}

struct RefreshTokenProvider {
    client_id: String,
    refresh_token: Secret<String>,
    token_url: String,
    authorize_url: String,
}

#[async_trait]
impl AccessTokenProvider for RefreshTokenProvider {
    async fn access_token(&self) -> Result<Secret<String>> {
        // Tesla's refresh grant takes grant_type, client_id and refresh_token
        // only; sending a client secret makes it reject the exchange.
        let config = OAuthConfig {
            client_id: self.client_id.clone(),
            client_secret: None,
            auth_url: self.authorize_url.clone(),
            token_url: self.token_url.clone(),
            redirect_uri: String::new(),
            resource: None,
            scopes: READ_SCOPES.map(ToOwned::to_owned).to_vec(),
            extra_auth_params: Vec::new(),
            device_flow: false,
        };
        tokio::time::timeout(
            REQUEST_TIMEOUT,
            OAuthFlow::new(config).refresh(self.refresh_token.expose_secret()),
        )
        .await
        .map_err(|_| TeslaConnectorError::Timeout)?
        .map(|tokens| tokens.access_token)
        .map_err(TeslaConnectorError::OAuth)
    }
}

pub struct NativeTeslaClient {
    client: reqwest::Client,
    base_url: Url,
    tokens: Arc<dyn AccessTokenProvider>,
    access_token: tokio::sync::OnceCell<Secret<String>>,
    request_timeout: Duration,
}

impl NativeTeslaClient {
    pub fn new(account: &TeslaAccountConfig) -> Result<Self> {
        account.validate()?;
        let base_url = Url::parse(account.region.base_url()).map_err(|error| {
            TeslaConnectorError::ServerResponse(format!("invalid fixed Fleet API URL: {error}"))
        })?;
        Ok(Self {
            client: moltis_common::http_client::build_default_http_client(),
            base_url,
            tokens: Arc::new(RefreshTokenProvider {
                client_id: account.client_id.clone(),
                refresh_token: account.refresh_token.clone(),
                token_url: account.region.token_url().to_owned(),
                authorize_url: account.region.authorize_url().to_owned(),
            }),
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
                TeslaConnectorError::ServerResponse(
                    "Fleet API base URL cannot contain path segments".to_owned(),
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
            .map_err(TeslaConnectorError::Http)?;
        let status = response.status();
        if !status.is_success() {
            return Err(TeslaConnectorError::from_status(status.as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > max_bytes as u64)
        {
            return Err(TeslaConnectorError::ServerResponse(
                "Fleet API response exceeds connector limit".to_owned(),
            ));
        }

        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.try_next().await.map_err(TeslaConnectorError::Http)? {
            let next_len = bytes.len().checked_add(chunk.len()).ok_or_else(|| {
                TeslaConnectorError::ServerResponse(
                    "Fleet API response exceeds connector limit".to_owned(),
                )
            })?;
            if next_len > max_bytes {
                return Err(TeslaConnectorError::ServerResponse(
                    "Fleet API response exceeds connector limit".to_owned(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|error| {
            TeslaConnectorError::ServerResponse(format!("invalid Fleet API JSON: {error}"))
        })
    }
}

#[async_trait]
impl TeslaClient for NativeTeslaClient {
    async fn list_vehicles(&self) -> Result<Vec<TeslaApiVehicle>> {
        let url = self.endpoint(&["api", "1", "vehicles"])?;
        let response: TeslaResponse<Vec<TeslaApiVehicle>> = self
            .send_json(self.client.get(url), MAX_VEHICLE_LIST_RESPONSE_BYTES)
            .await?;
        Ok(response.response)
    }

    async fn vehicle_data(&self, vin: &str, endpoints: &str) -> Result<TeslaVehicleData> {
        crate::validate_vin(vin)?;
        let url = self.endpoint(&["api", "1", "vehicles", vin, "vehicle_data"])?;
        let request = self.client.get(url).query(&[("endpoints", endpoints)]);
        let response: TeslaResponse<TeslaVehicleData> = self
            .send_json(request, MAX_VEHICLE_DATA_RESPONSE_BYTES)
            .await?;
        Ok(response.response)
    }
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

    fn test_url(port_path: &str) -> Result<Url> {
        Url::parse(port_path).map_err(|error| {
            TeslaConnectorError::ServerResponse(format!("test URL failed: {error}"))
        })
    }

    fn test_client(base_url: Url, timeout: Duration) -> NativeTeslaClient {
        NativeTeslaClient::with_test_dependencies(
            reqwest::Client::new(),
            base_url,
            Arc::new(FixedToken),
            timeout,
        )
    }

    async fn one_response_server(response: String, delay: Duration) -> Result<Url> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| {
                TeslaConnectorError::ServerResponse(format!("test listener failed: {error}"))
            })?;
        let address = listener.local_addr().map_err(|error| {
            TeslaConnectorError::ServerResponse(format!("test listener failed: {error}"))
        })?;
        tokio::spawn(async move {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::time::sleep(delay).await;
            let _ = stream.write_all(response.as_bytes()).await;
        });
        test_url(&format!("http://{address}/"))
    }

    #[test]
    fn endpoint_paths_are_percent_encoded() -> Result<()> {
        let client = test_client(test_url("http://127.0.0.1:9/")?, REQUEST_TIMEOUT);
        assert_eq!(
            client.endpoint(&["api", "1", "vehicles"])?.as_str(),
            "http://127.0.0.1:9/api/1/vehicles"
        );
        assert_eq!(
            client
                .endpoint(&["api", "1", "vehicles", "a/b", "vehicle_data"])?
                .as_str(),
            "http://127.0.0.1:9/api/1/vehicles/a%2Fb/vehicle_data"
        );
        Ok(())
    }

    #[tokio::test]
    async fn vehicle_data_rejects_malformed_vin_before_any_request() -> Result<()> {
        let client = test_client(test_url("http://127.0.0.1:9/")?, REQUEST_TIMEOUT);
        assert!(matches!(
            client.vehicle_data("../../admin", "charge_state").await,
            Err(TeslaConnectorError::DatasetConfig(_))
        ));
        Ok(())
    }

    #[tokio::test]
    async fn access_token_is_fetched_once_per_client() -> Result<()> {
        let provider = Arc::new(CountingToken(AtomicUsize::new(0)));
        let client = NativeTeslaClient::with_test_dependencies(
            reqwest::Client::new(),
            test_url("http://127.0.0.1:9/")?,
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

    #[tokio::test]
    async fn setup_and_auth_failures_map_to_actionable_errors() -> Result<()> {
        for (status, reason) in [
            (401, "Unauthorized"),
            (403, "Forbidden"),
            (412, "Precondition Failed"),
            (429, "Too Many Requests"),
        ] {
            let url = one_response_server(
                format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                ),
                Duration::ZERO,
            )
            .await?;
            let error = test_client(url, REQUEST_TIMEOUT)
                .list_vehicles()
                .await
                .err()
                .ok_or_else(|| {
                    TeslaConnectorError::ServerResponse("expected an error".to_owned())
                })?;
            match (status, error) {
                (401, TeslaConnectorError::Unauthorized)
                | (403 | 412, TeslaConnectorError::PartnerRegistrationMissing)
                | (429, TeslaConnectorError::RateLimited) => {},
                (_, other) => {
                    return Err(TeslaConnectorError::ServerResponse(format!(
                        "status {status} produced unexpected error: {other}"
                    )));
                },
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn oversized_declared_response_is_rejected() -> Result<()> {
        let url = one_response_server(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                MAX_VEHICLE_LIST_RESPONSE_BYTES + 1
            ),
            Duration::ZERO,
        )
        .await?;
        assert!(matches!(
            test_client(url, REQUEST_TIMEOUT).list_vehicles().await,
            Err(TeslaConnectorError::ServerResponse(_))
        ));
        Ok(())
    }

    #[tokio::test]
    async fn request_timeout_is_enforced() -> Result<()> {
        let url = one_response_server(
            "HTTP/1.1 200 OK\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"response\":[]}"
                .to_owned(),
            Duration::from_millis(100),
        )
        .await?;
        assert!(matches!(
            test_client(url, Duration::from_millis(10)).list_vehicles().await,
            Err(TeslaConnectorError::Http(error)) if error.is_timeout()
        ));
        Ok(())
    }

    #[tokio::test]
    async fn vehicle_list_envelope_is_unwrapped() -> Result<()> {
        let body = r#"{"response":[{"vin":"5YJ3E1EAXKF123456","state":"asleep"}],"count":1}"#;
        let url = one_response_server(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ),
            Duration::ZERO,
        )
        .await?;
        let vehicles = test_client(url, REQUEST_TIMEOUT).list_vehicles().await?;
        assert_eq!(vehicles.len(), 1);
        assert_eq!(vehicles[0].vin, "5YJ3E1EAXKF123456");
        Ok(())
    }
}
