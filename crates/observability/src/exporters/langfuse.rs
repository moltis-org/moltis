//! Langfuse backend.
//!
//! Traces reach Langfuse over OTLP — that is Langfuse's own modern ingest path,
//! the one its v3+ Python and v4+ JS SDKs use, and the only one that carries the
//! full observation taxonomy (`AGENT`, `TOOL`, `RETRIEVER`, ...). The native
//! `/api/public/ingestion` event `observation-create` is marked deprecated
//! upstream and its non-deprecated `span-create`/`generation-create` pair
//! cannot express the newer types at all.
//!
//! What OTLP *cannot* express is scores, so those go over the ingestion API.
//! This module owns both, plus the credential handling and the connection test
//! behind the settings UI's "Test connection" button.

use std::{collections::BTreeMap, time::Duration};

use {
    base64::{Engine as _, engine::general_purpose::STANDARD as BASE64},
    secrecy::{ExposeSecret, SecretString},
    serde::Serialize,
    tracing::debug,
};

use {
    super::otlp::{OtlpConfig, OtlpTransport},
    crate::{
        model::{ScoreRecord, ScoreValue},
        profile::ExportProfile,
    },
};

/// Path Langfuse serves OTLP traces on, appended to the configured host.
pub const OTEL_TRACES_PATH: &str = "/api/public/otel/v1/traces";
/// Batch ingestion path, used for scores.
pub const INGESTION_PATH: &str = "/api/public/ingestion";
/// Unauthenticated liveness probe.
pub const HEALTH_PATH: &str = "/api/public/health";

/// Langfuse connection settings.
#[derive(Clone)]
pub struct LangfuseConfig {
    /// Base host, e.g. `https://cloud.langfuse.com` or a self-hosted URL.
    pub host: String,
    /// Project public key (Basic auth username).
    pub public_key: String,
    /// Project secret key (Basic auth password).
    pub secret_key: SecretString,
    /// Deployment environment.
    pub environment: Option<String>,
    /// Build release identifier.
    pub release: Option<String>,
    /// Per-request timeout.
    pub timeout: Duration,
}

// Manual impl: the derived one would print the secret key.
impl std::fmt::Debug for LangfuseConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LangfuseConfig")
            .field("host", &self.host)
            .field("public_key", &self.public_key)
            .field("secret_key", &"[REDACTED]")
            .field("environment", &self.environment)
            .field("release", &self.release)
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl LangfuseConfig {
    /// Join `path` onto the configured host, tolerating a trailing slash.
    #[must_use]
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.host.trim_end_matches('/'))
    }

    /// `Authorization` header value for Basic auth.
    ///
    /// Langfuse authenticates with the public key as username and the secret
    /// key as password.
    #[must_use]
    pub fn basic_auth_header(&self) -> String {
        let raw = format!("{}:{}", self.public_key, self.secret_key.expose_secret());
        format!("Basic {}", BASE64.encode(raw))
    }

    /// Headers every authenticated Langfuse request carries.
    #[must_use]
    pub fn auth_headers(&self) -> BTreeMap<String, String> {
        let mut headers = BTreeMap::new();
        headers.insert("Authorization".to_string(), self.basic_auth_header());
        // Langfuse surfaces these in its own diagnostics.
        headers.insert("X-Langfuse-Sdk-Name".to_string(), "moltis".to_string());
        headers.insert(
            "X-Langfuse-Sdk-Version".to_string(),
            env!("CARGO_PKG_VERSION").to_string(),
        );
        headers.insert("X-Langfuse-Public-Key".to_string(), self.public_key.clone());
        headers
    }

    /// Build the OTLP transport that carries traces to Langfuse.
    ///
    /// The profile is fixed to [`ExportProfile::langfuse`]: content capture is
    /// the entire point of sending to Langfuse, and letting it be configured
    /// down to `MetadataOnly` would produce traces with no conversation in
    /// them, which is worse than not exporting at all.
    #[must_use]
    pub fn build_transport(&self, service_version: String) -> OtlpTransport {
        OtlpTransport::new(OtlpConfig {
            name: "langfuse".to_string(),
            endpoint: self.url(OTEL_TRACES_PATH),
            headers: self.auth_headers(),
            timeout: self.timeout,
            service_name: "moltis".to_string(),
            service_version,
            environment: self.environment.clone(),
            profile: ExportProfile::langfuse(),
        })
    }
}

// ── Scores ──────────────────────────────────────────────────────────────────

/// A `score-create` event in the ingestion batch envelope.
#[derive(Debug, Serialize)]
struct IngestionEvent<'a> {
    id: String,
    timestamp: String,
    #[serde(rename = "type")]
    kind: &'static str,
    body: ScoreBody<'a>,
}

/// Langfuse `ScoreBody`.
#[derive(Debug, Serialize)]
struct ScoreBody<'a> {
    id: &'a str,
    #[serde(rename = "traceId")]
    trace_id: &'a str,
    #[serde(rename = "observationId", skip_serializing_if = "Option::is_none")]
    observation_id: Option<&'a str>,
    name: &'a str,
    value: &'a ScoreValue,
    #[serde(rename = "dataType")]
    data_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    comment: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    environment: Option<&'a str>,
}

/// The ingestion request envelope.
#[derive(Debug, Serialize)]
struct IngestionRequest<'a> {
    batch: Vec<IngestionEvent<'a>>,
}

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
            .http
            .post(self.config.url(INGESTION_PATH))
            .headers(self.header_map())
            .timeout(self.config.timeout)
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

    /// Submit scores. Langfuse upserts on score id.
    pub async fn submit_scores(&self, scores: &[ScoreRecord]) -> anyhow::Result<()> {
        if scores.is_empty() {
            return Ok(());
        }

        let now = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();

        let batch: Vec<IngestionEvent<'_>> = scores
            .iter()
            .map(|score| IngestionEvent {
                id: uuid::Uuid::new_v4().to_string(),
                timestamp: now.clone(),
                kind: "score-create",
                body: ScoreBody {
                    id: &score.id,
                    trace_id: &score.trace_id.0,
                    observation_id: score.observation_id.as_ref().map(|o| o.0.as_str()),
                    name: &score.name,
                    value: &score.value,
                    data_type: match score.value {
                        ScoreValue::Numeric(_) => "NUMERIC",
                        ScoreValue::Categorical(_) => "CATEGORICAL",
                    },
                    comment: score.comment.as_deref(),
                    environment: score
                        .environment
                        .as_deref()
                        .or(self.config.environment.as_deref()),
                },
            })
            .collect();

        let response = self
            .http
            .post(self.config.url(INGESTION_PATH))
            .headers(self.header_map())
            .timeout(self.config.timeout)
            .json(&IngestionRequest { batch })
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Langfuse rejected scores: HTTP {status}: {}",
                body.chars().take(512).collect::<String>()
            ));
        }

        // The ingestion endpoint reports per-event outcomes in a 207, so a 2xx
        // alone does not mean every score landed.
        let body: serde_json::Value = response.json().await.unwrap_or_default();
        if let Some(errors) = body.get("errors").and_then(|e| e.as_array())
            && !errors.is_empty()
        {
            return Err(anyhow::anyhow!(
                "Langfuse rejected {} of {} scores: {}",
                errors.len(),
                scores.len(),
                serde_json::to_string(errors).unwrap_or_default()
            ));
        }

        debug!(count = scores.len(), "submitted scores to Langfuse");
        Ok(())
    }

    /// Authenticated header map.
    fn header_map(&self) -> reqwest::header::HeaderMap {
        let mut map = reqwest::header::HeaderMap::new();
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
mod tests {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json_schema, header, method, path},
    };

    use {
        super::*,
        crate::model::{ObservationId, TraceId},
    };

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

    #[test]
    fn basic_auth_encodes_public_and_secret_key() {
        let header = config("https://example.com".into()).basic_auth_header();
        let encoded = header.strip_prefix("Basic ").expect("basic scheme");
        let decoded =
            String::from_utf8(BASE64.decode(encoded).expect("valid base64")).expect("valid utf-8");

        assert_eq!(decoded, "pk-lf-test:sk-lf-secret");
    }

    #[test]
    fn debug_never_prints_the_secret_key() {
        let rendered = format!("{:?}", config("https://example.com".into()));

        assert!(
            !rendered.contains("sk-lf-secret"),
            "secret leaked: {rendered}"
        );
        assert!(rendered.contains("[REDACTED]"));
        // The public key is not a secret and is useful in diagnostics.
        assert!(rendered.contains("pk-lf-test"));
    }

    #[test]
    fn url_join_tolerates_a_trailing_slash_on_the_host() {
        let with = config("https://example.com/".into());
        let without = config("https://example.com".into());

        assert_eq!(with.url(OTEL_TRACES_PATH), without.url(OTEL_TRACES_PATH));
        assert_eq!(
            without.url(OTEL_TRACES_PATH),
            "https://example.com/api/public/otel/v1/traces"
        );
    }

    #[test]
    fn transport_targets_the_otel_endpoint_with_the_langfuse_profile() {
        let transport = config("https://cloud.langfuse.com".into()).build_transport("1.0".into());

        assert_eq!(
            transport.endpoint(),
            "https://cloud.langfuse.com/api/public/otel/v1/traces"
        );
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

    #[tokio::test]
    async fn scores_post_as_score_create_events() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(INGESTION_PATH))
            .and(body_json_schema::<serde_json::Value>)
            .respond_with(ResponseTemplate::new(207).set_body_json(serde_json::json!({
                "successes": [{ "id": "1", "status": 201 }], "errors": []
            })))
            .expect(1)
            .mount(&server)
            .await;

        let mut score = ScoreRecord::new(
            TraceId("trace-1".into()),
            "user-feedback",
            ScoreValue::Numeric(1.0),
        );
        score.observation_id = Some(ObservationId("obs-1".into()));
        score.comment = Some("helpful".into());

        LangfuseClient::new(config(server.uri()))
            .submit_scores(&[score])
            .await
            .expect("score submission should succeed");
    }

    #[tokio::test]
    async fn partial_rejection_in_a_207_is_surfaced_as_an_error() {
        let server = MockServer::start().await;
        // A 2xx alone does not mean the data landed: the ingestion endpoint
        // reports per-event outcomes in the body.
        Mock::given(method("POST"))
            .and(path(INGESTION_PATH))
            .respond_with(ResponseTemplate::new(207).set_body_json(serde_json::json!({
                "successes": [],
                "errors": [{ "id": "1", "status": 400, "message": "invalid value" }]
            })))
            .mount(&server)
            .await;

        let score = ScoreRecord::new(
            TraceId("trace-1".into()),
            "user-feedback",
            ScoreValue::Numeric(1.0),
        );
        let error = LangfuseClient::new(config(server.uri()))
            .submit_scores(&[score])
            .await
            .expect_err("partial failure should surface");

        assert!(error.to_string().contains("rejected 1 of 1"), "{error}");
    }

    #[tokio::test]
    async fn empty_score_batches_skip_the_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        LangfuseClient::new(config(server.uri()))
            .submit_scores(&[])
            .await
            .expect("empty batch is a no-op");
    }

    #[test]
    fn categorical_and_numeric_scores_declare_their_data_type() {
        let numeric = ScoreValue::Numeric(0.5);
        let categorical = ScoreValue::Categorical("helpful".into());

        assert_eq!(
            serde_json::to_value(&numeric).expect("serializable"),
            serde_json::json!(0.5)
        );
        assert_eq!(
            serde_json::to_value(&categorical).expect("serializable"),
            serde_json::json!("helpful")
        );
    }
}
