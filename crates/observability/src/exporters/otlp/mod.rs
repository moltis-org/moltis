//! OTLP/HTTP JSON exporter.
//!
//! One transport serves every backend we target: Langfuse (which accepts OTLP
//! on `/api/public/otel/v1/traces`), Grafana Tempo/Alloy, Datadog's OTLP
//! intake, Honeycomb, and any other OTLP-compatible collector. The difference
//! between them is the endpoint and the auth header, not the payload.

pub mod attributes;
pub mod mapping;
pub mod wire;

use std::{collections::BTreeMap, time::Duration};

use {
    async_trait::async_trait,
    reqwest::{
        StatusCode,
        header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue},
    },
    tracing::{debug, warn},
};

use crate::{
    model::Event,
    profile::ExportProfile,
    runtime::{Transport, TransportError},
};

/// Configuration for an OTLP exporter.
#[derive(Clone)]
pub struct OtlpConfig {
    /// Display name for logs and status reporting.
    pub name: String,
    /// Full traces endpoint, e.g. `https://cloud.langfuse.com/api/public/otel/v1/traces`.
    pub endpoint: String,
    /// Additional headers, typically carrying authentication.
    pub headers: BTreeMap<String, String>,
    /// Per-request timeout.
    pub timeout: Duration,
    /// Value reported as `service.name`.
    pub service_name: String,
    /// Value reported as `service.version`.
    pub service_version: String,
    /// Deployment environment, reported on the resource and every span.
    pub environment: Option<String>,
    /// What this backend is allowed to receive. Defaults to the conservative
    /// profile so a misconfigured backend cannot silently receive prompts.
    pub profile: ExportProfile,
}

impl std::fmt::Debug for OtlpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OtlpConfig")
            .field("name", &self.name)
            .field("endpoint", &self.endpoint)
            .field("header_names", &self.headers.keys().collect::<Vec<_>>())
            .field("timeout", &self.timeout)
            .field("service_name", &self.service_name)
            .field("service_version", &self.service_version)
            .field("environment", &self.environment)
            .field("profile", &self.profile)
            .finish()
    }
}

impl Default for OtlpConfig {
    fn default() -> Self {
        Self {
            name: "otlp".to_string(),
            endpoint: String::new(),
            headers: BTreeMap::new(),
            timeout: Duration::from_secs(10),
            service_name: "moltis".to_string(),
            service_version: env!("CARGO_PKG_VERSION").to_string(),
            environment: None,
            profile: ExportProfile::default(),
        }
    }
}

/// OTLP/HTTP JSON transport.
pub struct OtlpTransport {
    config: OtlpConfig,
    client: reqwest::Client,
    headers: HeaderMap,
}

impl OtlpTransport {
    /// Build a transport, reusing the workspace HTTP client so the configured
    /// upstream proxy and default headers apply.
    ///
    /// Header values that cannot be encoded are dropped with a warning rather
    /// than failing construction: one malformed custom header should not
    /// disable instrumentation entirely.
    #[must_use]
    pub fn new(config: OtlpConfig) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        for (key, value) in &config.headers {
            let Ok(name) = key.parse::<HeaderName>() else {
                warn!(header = %key, "ignoring invalid OTLP header name");
                continue;
            };
            let Ok(mut val) = HeaderValue::from_str(value) else {
                warn!(header = %key, "ignoring OTLP header with unencodable value");
                continue;
            };
            // Credentials must never surface in a debug dump of the headers.
            val.set_sensitive(true);
            headers.insert(name, val);
        }

        Self {
            client: moltis_common::http_client::build_default_http_client(),
            headers,
            config,
        }
    }

    /// The configured endpoint.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.config.endpoint
    }

    /// Identity and policy applied to each exported batch.
    fn export_context(&self) -> mapping::ExportContext {
        mapping::ExportContext {
            service_name: self.config.service_name.clone(),
            service_version: self.config.service_version.clone(),
            environment: self.config.environment.clone(),
            profile: self.config.profile.clone(),
        }
    }

    /// Classify an HTTP status into a retry decision.
    ///
    /// 4xx other than 408/429 are fatal: retrying a rejected payload or bad
    /// credentials cannot succeed and only burns rate-limit budget.
    fn classify(status: StatusCode) -> TransportError {
        let message = format!("collector rejected OTLP batch with HTTP {status}");
        if status == StatusCode::TOO_MANY_REQUESTS
            || status == StatusCode::REQUEST_TIMEOUT
            || status.is_server_error()
        {
            TransportError::Retryable(message)
        } else {
            TransportError::Fatal(message)
        }
    }
}

#[async_trait]
impl Transport for OtlpTransport {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn send(&self, batch: &[Event]) -> Result<(), TransportError> {
        let request = mapping::batch_to_request(batch, &self.export_context());

        // A batch of only score events produces no spans; sending an empty
        // resourceSpans payload is a wasted round trip.
        if request
            .resource_spans
            .iter()
            .all(|rs| rs.scope_spans.iter().all(|ss| ss.spans.is_empty()))
        {
            return Ok(());
        }

        let body = serde_json::to_vec(&request)
            .map_err(|e| TransportError::Fatal(format!("failed to encode OTLP payload: {e}")))?;

        let response = self
            .client
            .post(&self.config.endpoint)
            .headers(self.headers.clone())
            .timeout(self.config.timeout)
            .body(body)
            .send()
            .await
            .map_err(|e| {
                // Connection-level failures are always worth retrying: the
                // collector may simply be restarting.
                TransportError::Retryable(format!("collector request failed: {e}"))
            })?;

        let status = response.status();
        if status.is_success() {
            debug!(
                sink = %self.config.name,
                events = batch.len(),
                "OTLP batch accepted"
            );
            return Ok(());
        }

        Err(Self::classify(status))
    }

    fn accepts(&self, event: &Event) -> bool {
        !matches!(event, Event::Score(_) | Event::ScoreDelete(_))
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path},
    };

    use {
        super::*,
        crate::model::{ObservationKind, ObservationRecord, TraceId},
    };

    fn config(endpoint: String) -> OtlpConfig {
        let mut headers = BTreeMap::new();
        headers.insert("Authorization".to_string(), "Basic dGVzdA==".to_string());
        OtlpConfig {
            name: "test".into(),
            endpoint,
            headers,
            timeout: Duration::from_secs(5),
            service_name: "moltis".into(),
            service_version: "test".into(),
            environment: Some("staging".into()),
            profile: ExportProfile::langfuse(),
        }
    }

    fn events() -> Vec<Event> {
        let mut obs =
            ObservationRecord::start(TraceId::generate(), ObservationKind::Generation, "llm-call");
        obs.finish();
        vec![Event::ObservationEnd(Box::new(obs))]
    }

    #[tokio::test]
    async fn posts_json_with_configured_headers() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/traces"))
            .and(header("content-type", "application/json"))
            .and(header("authorization", "Basic dGVzdA=="))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let transport = OtlpTransport::new(config(format!("{}/v1/traces", server.uri())));
        transport
            .send(&events())
            .await
            .expect("send should succeed");
    }

    #[tokio::test]
    async fn empty_span_batches_skip_the_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let transport = OtlpTransport::new(config(format!("{}/v1/traces", server.uri())));
        // Scores map to no spans, so this batch has nothing to send.
        let score = crate::model::ScoreRecord::new(
            TraceId::generate(),
            "feedback",
            crate::model::ScoreValue::Numeric(1.0),
        );
        transport
            .send(&[Event::Score(Box::new(score))])
            .await
            .expect("empty batch should be a no-op");
    }

    #[tokio::test]
    async fn server_errors_are_retryable() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let transport = OtlpTransport::new(config(format!("{}/v1/traces", server.uri())));
        let error = transport
            .send(&events())
            .await
            .expect_err("503 should fail");

        assert!(matches!(error, TransportError::Retryable(_)), "{error:?}");
    }

    #[tokio::test]
    async fn rate_limiting_is_retryable() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;

        let transport = OtlpTransport::new(config(format!("{}/v1/traces", server.uri())));
        let error = transport
            .send(&events())
            .await
            .expect_err("429 should fail");

        assert!(matches!(error, TransportError::Retryable(_)), "{error:?}");
    }

    #[tokio::test]
    async fn unauthorized_is_fatal_not_retried() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let transport = OtlpTransport::new(config(format!("{}/v1/traces", server.uri())));
        let error = transport
            .send(&events())
            .await
            .expect_err("401 should fail");

        // Retrying bad credentials cannot succeed and wastes rate-limit budget.
        assert!(matches!(error, TransportError::Fatal(_)), "{error:?}");
    }

    #[tokio::test]
    async fn bad_request_is_fatal() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_string("echoed prompt and Authorization: Basic super-secret"),
            )
            .mount(&server)
            .await;

        let transport = OtlpTransport::new(config(format!("{}/v1/traces", server.uri())));
        let error = transport
            .send(&events())
            .await
            .expect_err("400 should fail");

        let TransportError::Fatal(message) = error else {
            panic!("expected fatal error");
        };
        assert!(message.contains("HTTP 400"));
        assert!(!message.contains("echoed prompt"));
        assert!(!message.contains("super-secret"));
    }

    #[tokio::test]
    async fn connection_failure_is_retryable() {
        // Port 1 is reserved and refuses connections immediately.
        let transport = OtlpTransport::new(config("http://127.0.0.1:1/v1/traces".into()));
        let error = transport
            .send(&events())
            .await
            .expect_err("connection refused should fail");

        assert!(matches!(error, TransportError::Retryable(_)), "{error:?}");
    }

    #[test]
    fn invalid_headers_are_skipped_without_disabling_the_exporter() {
        let mut headers = BTreeMap::new();
        headers.insert("valid-header".to_string(), "ok".to_string());
        // Newlines cannot be encoded in a header value.
        headers.insert("bad".to_string(), "line\nbreak".to_string());
        // Spaces are not legal in a header name.
        headers.insert("in valid".to_string(), "x".to_string());

        let transport = OtlpTransport::new(OtlpConfig {
            headers,
            ..Default::default()
        });

        assert!(transport.headers.contains_key("valid-header"));
        assert!(!transport.headers.contains_key("bad"));
        assert_eq!(transport.headers.len(), 2, "content-type plus valid-header");
    }

    #[test]
    fn credential_headers_are_marked_sensitive() {
        let mut headers = BTreeMap::new();
        headers.insert("authorization".to_string(), "Basic secret".to_string());
        let transport = OtlpTransport::new(OtlpConfig {
            headers,
            ..Default::default()
        });

        let value = transport
            .headers
            .get("authorization")
            .expect("header present");
        // Sensitive headers are redacted by reqwest's own Debug output.
        assert!(value.is_sensitive());
        assert!(!format!("{:?}", transport.headers).contains("Basic secret"));
    }

    #[test]
    fn config_debug_omits_header_values() {
        let mut config = config("https://collector.example/v1/traces".into());
        config
            .headers
            .insert("Authorization".into(), "Bearer super-secret".into());

        let rendered = format!("{config:?}");

        assert!(rendered.contains("Authorization"));
        assert!(!rendered.contains("super-secret"));
    }
}
