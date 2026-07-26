//! The Langfuse REST client and the request helpers its API modules share.
//!
//! Traces reach Langfuse over OTLP; everything OTLP cannot express — scores,
//! managed prompts, datasets, media, aggregated cost — goes through here.

use reqwest::header::HeaderMap;

use super::config::{HEALTH_PATH, INGESTION_PATH, LangfuseConfig};

/// Client for the Langfuse REST surfaces that OTLP cannot express.
pub struct LangfuseClient {
    config: LangfuseConfig,
    http: reqwest::Client,
}

impl LangfuseClient {
    /// Build a client over the workspace HTTP client, so the configured
    /// upstream proxy applies.
    #[must_use]
    pub fn new(config: LangfuseConfig) -> Self {
        Self {
            config,
            http: moltis_common::http_client::build_default_http_client(),
        }
    }

    /// The connection settings this client was built from.
    #[must_use]
    pub const fn config(&self) -> &LangfuseConfig {
        &self.config
    }

    /// Verify that the host is reachable and the credentials are accepted.
    ///
    /// Backs the settings UI's "Test connection" button. The health endpoint is
    /// unauthenticated, so credentials are checked with an empty ingestion
    /// batch: it is the cheapest authenticated call that creates no data.
    pub async fn test_connection(&self) -> anyhow::Result<()> {
        let health = self
            .http
            .get(self.config.url(HEALTH_PATH))
            .timeout(self.config.timeout)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("cannot reach Langfuse at {}: {e}", self.config.host))?;

        if !health.status().is_success() {
            return Err(anyhow::anyhow!(
                "Langfuse health check failed with HTTP {}",
                health.status()
            ));
        }

        let auth = self
            .post(INGESTION_PATH)
            .json(&serde_json::json!({ "batch": [] }))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Langfuse credential check failed: {e}"))?;

        let status = auth.status();
        match status {
            s if s.is_success() => Ok(()),
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => Err(
                anyhow::anyhow!("Langfuse rejected the credentials (HTTP {status})"),
            ),
            other => Err(anyhow::anyhow!(
                "unexpected response from Langfuse: HTTP {other}"
            )),
        }
    }

    /// Authenticated POST against `path`.
    pub(super) fn post(&self, path: &str) -> reqwest::RequestBuilder {
        self.http
            .post(self.config.url(path))
            .headers(self.header_map())
            .timeout(self.config.timeout)
    }

    /// Authenticated DELETE against `path`.
    pub(super) fn delete(&self, path: &str) -> reqwest::RequestBuilder {
        self.http
            .delete(self.config.url(path))
            .headers(self.header_map())
            .timeout(self.config.timeout)
    }

    /// Authenticated header map.
    ///
    /// Header values are marked sensitive so they stay out of any middleware
    /// that logs request headers.
    fn header_map(&self) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (key, value) in self.config.auth_headers() {
            let Ok(name) = key.parse::<reqwest::header::HeaderName>() else {
                continue;
            };
            let Ok(mut val) = reqwest::header::HeaderValue::from_str(&value) else {
                continue;
            };
            val.set_sensitive(true);
            map.insert(name, val);
        }
        map
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use {
        std::time::Duration,
        wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{header, method, path},
        },
    };

    use {super::*, secrecy::SecretString};

    fn config(host: String) -> LangfuseConfig {
        LangfuseConfig {
            host,
            public_key: "pk-lf-test".into(),
            secret_key: SecretString::new("sk-lf-secret".to_string()),
            environment: Some("production".into()),
            release: Some("20260726.01".into()),
            timeout: Duration::from_secs(5),
        }
    }

    #[tokio::test]
    async fn test_connection_accepts_a_healthy_authenticated_host() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(HEALTH_PATH))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(INGESTION_PATH))
            .and(header(
                "authorization",
                "Basic cGstbGYtdGVzdDpzay1sZi1zZWNyZXQ=",
            ))
            .respond_with(ResponseTemplate::new(207).set_body_json(serde_json::json!({
                "successes": [], "errors": []
            })))
            .mount(&server)
            .await;

        LangfuseClient::new(config(server.uri()))
            .test_connection()
            .await
            .expect("healthy host should pass");
    }

    #[tokio::test]
    async fn test_connection_reports_bad_credentials_distinctly() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(HEALTH_PATH))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(INGESTION_PATH))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let error = LangfuseClient::new(config(server.uri()))
            .test_connection()
            .await
            .expect_err("401 should fail");

        // The operator needs to know it is the keys, not the host.
        assert!(
            error.to_string().contains("rejected the credentials"),
            "unhelpful message: {error}"
        );
    }

    #[tokio::test]
    async fn test_connection_reports_an_unreachable_host_distinctly() {
        let error = LangfuseClient::new(config("http://127.0.0.1:1".into()))
            .test_connection()
            .await
            .expect_err("connection refused should fail");

        assert!(
            error.to_string().contains("cannot reach Langfuse"),
            "unhelpful message: {error}"
        );
    }
}
