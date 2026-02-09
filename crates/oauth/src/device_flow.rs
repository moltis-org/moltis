use {anyhow::Result, reqwest::header::HeaderMap, secrecy::Secret};

use crate::types::{OAuthConfig, OAuthTokens};

/// Response from the device code request.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    /// Some providers (e.g. Kimi) return a complete URI with the code embedded.
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    #[serde(default = "default_interval")]
    pub interval: u64,
}

fn default_interval() -> u64 {
    5
}

/// Request a device code from the provider.
pub async fn request_device_code(
    client: &reqwest::Client,
    config: &OAuthConfig,
) -> Result<DeviceCodeResponse> {
    request_device_code_with_headers(client, config, None).await
}

/// Request a device code, optionally sending extra headers.
pub async fn request_device_code_with_headers(
    client: &reqwest::Client,
    config: &OAuthConfig,
    extra_headers: Option<&HeaderMap>,
) -> Result<DeviceCodeResponse> {
    let mut req = client
        .post(&config.auth_url)
        .header("Accept", "application/json")
        .form(&[("client_id", config.client_id.as_str()), ("scope", "")]);

    if let Some(headers) = extra_headers {
        req = req.headers(headers.clone());
    }

    let resp = req.send().await?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("device code request failed: {body}");
    }

    Ok(resp.json().await?)
}

#[derive(Debug, serde::Deserialize)]
struct TokenPollResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    error: Option<String>,
}

/// Poll the token endpoint until the user completes the device flow.
pub async fn poll_for_token(
    client: &reqwest::Client,
    config: &OAuthConfig,
    device_code: &str,
    interval: u64,
) -> Result<OAuthTokens> {
    poll_for_token_with_headers(client, config, device_code, interval, None).await
}

/// Poll the token endpoint, optionally sending extra headers.
pub async fn poll_for_token_with_headers(
    client: &reqwest::Client,
    config: &OAuthConfig,
    device_code: &str,
    interval: u64,
    extra_headers: Option<&HeaderMap>,
) -> Result<OAuthTokens> {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;

        let mut req = client
            .post(&config.token_url)
            .header("Accept", "application/json")
            .form(&[
                ("client_id", config.client_id.as_str()),
                ("device_code", device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ]);

        if let Some(headers) = extra_headers {
            req = req.headers(headers.clone());
        }

        let resp = req.send().await?;

        let body: TokenPollResponse = resp.json().await?;

        if let Some(token) = body.access_token {
            let expires_at = body.expires_in.and_then(|secs| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .ok()
                    .map(|d| d.as_secs() + secs)
            });
            return Ok(OAuthTokens {
                access_token: Secret::new(token),
                refresh_token: body.refresh_token.map(Secret::new),
                expires_at,
            });
        }

        match body.error.as_deref() {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            },
            Some(err) => anyhow::bail!("device flow error: {err}"),
            None => anyhow::bail!("unexpected response from token endpoint"),
        }
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};

    use {
        axum::{Router, extract::Form, routing::post},
        secrecy::ExposeSecret,
    };

    fn test_config(auth_url: String, token_url: String) -> OAuthConfig {
        OAuthConfig {
            client_id: "test-client".into(),
            auth_url,
            token_url,
            redirect_uri: String::new(),
            scopes: vec![],
            extra_auth_params: vec![],
            device_flow: true,
        }
    }

    /// Start a mock HTTP server and return its base URL.
    async fn start_mock(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
    }

    #[test]
    fn device_code_response_deserialize() {
        let json = r#"{
            "device_code": "dc_123",
            "user_code": "ABCD-1234",
            "verification_uri": "https://github.com/login/device"
        }"#;
        let resp: DeviceCodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.device_code, "dc_123");
        assert_eq!(resp.user_code, "ABCD-1234");
        assert_eq!(resp.verification_uri, "https://github.com/login/device");
        assert_eq!(resp.interval, 5); // default
        assert!(resp.verification_uri_complete.is_none());
    }

    #[test]
    fn device_code_response_with_interval() {
        let json = r#"{
            "device_code": "dc",
            "user_code": "CODE",
            "verification_uri": "https://example.com",
            "interval": 10
        }"#;
        let resp: DeviceCodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.interval, 10);
    }

    #[test]
    fn device_code_response_with_verification_uri_complete() {
        let json = r#"{
            "device_code": "dc",
            "user_code": "CODE",
            "verification_uri": "https://example.com",
            "verification_uri_complete": "https://example.com?code=CODE"
        }"#;
        let resp: DeviceCodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            resp.verification_uri_complete.as_deref(),
            Some("https://example.com?code=CODE")
        );
    }

    #[test]
    fn device_code_response_serialize_roundtrip() {
        let resp = DeviceCodeResponse {
            device_code: "dc_abc".into(),
            user_code: "WXYZ-1234".into(),
            verification_uri: "https://example.com/device".into(),
            verification_uri_complete: Some("https://example.com/device?code=WXYZ-1234".into()),
            interval: 8,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: DeviceCodeResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.device_code, "dc_abc");
        assert_eq!(back.user_code, "WXYZ-1234");
        assert_eq!(back.interval, 8);
        assert!(back.verification_uri_complete.is_some());
    }

    #[tokio::test]
    async fn request_device_code_success() {
        let app = Router::new().route(
            "/device/code",
            post(|| async {
                axum::Json(serde_json::json!({
                    "device_code": "mock_dc",
                    "user_code": "TEST-CODE",
                    "verification_uri": "https://example.com/device",
                    "interval": 1
                }))
            }),
        );
        let base = start_mock(app).await;
        let config = test_config(format!("{base}/device/code"), String::new());

        let client = reqwest::Client::new();
        let resp = request_device_code(&client, &config).await.unwrap();
        assert_eq!(resp.device_code, "mock_dc");
        assert_eq!(resp.user_code, "TEST-CODE");
        assert_eq!(resp.verification_uri, "https://example.com/device");
        assert_eq!(resp.interval, 1);
    }

    #[tokio::test]
    async fn request_device_code_with_extra_headers() {
        use axum::extract::Request;

        let app = Router::new().route(
            "/device/code",
            post(|req: Request| async move {
                // Verify the extra header was sent
                let has_header = req.headers().get("X-Custom").is_some();
                assert!(has_header, "expected X-Custom header");
                axum::Json(serde_json::json!({
                    "device_code": "dc",
                    "user_code": "CODE",
                    "verification_uri": "https://example.com",
                }))
            }),
        );
        let base = start_mock(app).await;
        let config = test_config(format!("{base}/device/code"), String::new());

        let client = reqwest::Client::new();
        let mut headers = HeaderMap::new();
        headers.insert("X-Custom", "test-value".parse().unwrap());
        let resp = request_device_code_with_headers(&client, &config, Some(&headers))
            .await
            .unwrap();
        assert_eq!(resp.device_code, "dc");
    }

    #[tokio::test]
    async fn request_device_code_server_error() {
        let app = Router::new().route(
            "/device/code",
            post(|| async { (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
        );
        let base = start_mock(app).await;
        let config = test_config(format!("{base}/device/code"), String::new());

        let client = reqwest::Client::new();
        let err = request_device_code(&client, &config).await.unwrap_err();
        assert!(err.to_string().contains("device code request failed"));
    }

    #[tokio::test]
    async fn poll_for_token_immediate_success() {
        let app = Router::new().route(
            "/token",
            post(|| async {
                axum::Json(serde_json::json!({
                    "access_token": "ghp_mock_token"
                }))
            }),
        );
        let base = start_mock(app).await;
        let config = test_config(String::new(), format!("{base}/token"));

        let client = reqwest::Client::new();
        let tokens = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            poll_for_token(&client, &config, "dc_123", 0),
        )
        .await
        .expect("timed out")
        .unwrap();
        assert_eq!(tokens.access_token.expose_secret(), "ghp_mock_token");
        assert!(tokens.refresh_token.is_none());
    }

    #[tokio::test]
    async fn poll_for_token_with_refresh_and_expiry() {
        let app = Router::new().route(
            "/token",
            post(|| async {
                axum::Json(serde_json::json!({
                    "access_token": "at_123",
                    "refresh_token": "rt_456",
                    "expires_in": 3600
                }))
            }),
        );
        let base = start_mock(app).await;
        let config = test_config(String::new(), format!("{base}/token"));

        let client = reqwest::Client::new();
        let tokens = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            poll_for_token(&client, &config, "dc_123", 0),
        )
        .await
        .expect("timed out")
        .unwrap();
        assert_eq!(tokens.access_token.expose_secret(), "at_123");
        assert_eq!(
            tokens
                .refresh_token
                .as_ref()
                .map(|s| s.expose_secret().as_str()),
            Some("rt_456")
        );
        assert!(tokens.expires_at.is_some());
        // expires_at should be roughly now + 3600
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let expires_at = tokens.expires_at.unwrap();
        assert!(expires_at >= now + 3590 && expires_at <= now + 3610);
    }

    #[tokio::test]
    async fn poll_for_token_pending_then_success() {
        // Return "authorization_pending" once, then success
        let call_count = std::sync::Arc::new(AtomicUsize::new(0));
        let counter = call_count.clone();

        let app = Router::new().route(
            "/token",
            post(move |_body: Form<Vec<(String, String)>>| {
                let counter = counter.clone();
                async move {
                    let n = counter.fetch_add(1, Ordering::SeqCst);
                    if n == 0 {
                        axum::Json(serde_json::json!({"error": "authorization_pending"}))
                    } else {
                        axum::Json(serde_json::json!({"access_token": "ghp_success"}))
                    }
                }
            }),
        );
        let base = start_mock(app).await;
        let config = test_config(String::new(), format!("{base}/token"));

        let client = reqwest::Client::new();
        let tokens = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            poll_for_token(&client, &config, "dc_123", 0),
        )
        .await
        .expect("timed out")
        .unwrap();
        assert_eq!(tokens.access_token.expose_secret(), "ghp_success");
        assert!(call_count.load(Ordering::SeqCst) >= 2);
    }

    #[tokio::test]
    async fn poll_for_token_access_denied_error() {
        let app = Router::new().route(
            "/token",
            post(|| async { axum::Json(serde_json::json!({"error": "access_denied"})) }),
        );
        let base = start_mock(app).await;
        let config = test_config(String::new(), format!("{base}/token"));

        let client = reqwest::Client::new();
        let err = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            poll_for_token(&client, &config, "dc_123", 0),
        )
        .await
        .expect("timed out")
        .unwrap_err();
        assert!(err.to_string().contains("access_denied"));
    }

    #[tokio::test]
    async fn poll_for_token_unexpected_response() {
        let app = Router::new().route(
            "/token",
            post(|| async { axum::Json(serde_json::json!({})) }),
        );
        let base = start_mock(app).await;
        let config = test_config(String::new(), format!("{base}/token"));

        let client = reqwest::Client::new();
        let err = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            poll_for_token(&client, &config, "dc_123", 0),
        )
        .await
        .expect("timed out")
        .unwrap_err();
        assert!(err.to_string().contains("unexpected response"));
    }
}
