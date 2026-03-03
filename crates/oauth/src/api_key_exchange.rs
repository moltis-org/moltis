use {
    crate::error::Error,
    secrecy::{ExposeSecret, Secret},
};

/// Response from the API key creation endpoint.
#[derive(serde::Deserialize)]
struct CreateApiKeyResponse {
    raw_key: String,
}

/// Exchange a temporary OAuth access token for a permanent API key.
///
/// POSTs to `endpoint` with `Authorization: Bearer <token>` and parses the
/// `raw_key` field from the JSON response.
pub async fn create_api_key_from_oauth(
    endpoint: &str,
    access_token: &Secret<String>,
) -> crate::Result<Secret<String>> {
    let client = reqwest::Client::new();
    let resp = client
        .post(endpoint)
        .bearer_auth(access_token.expose_secret())
        .header("Content-Type", "application/json")
        .body("{}")
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::message(format!(
            "API key creation failed (HTTP {}): {body}",
            status.as_u16(),
        )));
    }

    let parsed: CreateApiKeyResponse = resp.json().await?;

    if parsed.raw_key.is_empty() {
        return Err(Error::message("API key creation returned an empty key"));
    }

    Ok(Secret::new(parsed.raw_key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::unwrap_used)]
    #[tokio::test]
    async fn exchange_success() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/create_api_key")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"raw_key":"sk-ant-api03-test-key"}"#)
            .create_async()
            .await;

        let token = Secret::new("oauth-access-token".to_string());
        let result =
            create_api_key_from_oauth(&format!("{}/create_api_key", server.url()), &token).await;

        mock.assert_async().await;
        let key = result.unwrap();
        assert_eq!(key.expose_secret(), "sk-ant-api03-test-key");
    }

    #[allow(clippy::unwrap_used)]
    #[tokio::test]
    async fn exchange_empty_key() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/create_api_key")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"raw_key":""}"#)
            .create_async()
            .await;

        let token = Secret::new("token".to_string());
        let result =
            create_api_key_from_oauth(&format!("{}/create_api_key", server.url()), &token).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty key"),);
    }

    #[allow(clippy::unwrap_used)]
    #[tokio::test]
    async fn exchange_server_error() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/create_api_key")
            .with_status(500)
            .with_body("internal server error")
            .create_async()
            .await;

        let token = Secret::new("token".to_string());
        let result =
            create_api_key_from_oauth(&format!("{}/create_api_key", server.url()), &token).await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("500"),
            "error should mention status code: {err}"
        );
    }

    #[allow(clippy::unwrap_used)]
    #[tokio::test]
    async fn exchange_missing_field() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/create_api_key")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"other_field":"value"}"#)
            .create_async()
            .await;

        let token = Secret::new("token".to_string());
        let result =
            create_api_key_from_oauth(&format!("{}/create_api_key", server.url()), &token).await;

        assert!(result.is_err());
    }
}
