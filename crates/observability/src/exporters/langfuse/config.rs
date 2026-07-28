//! Langfuse connection settings, endpoint paths and credential handling.

use std::{collections::BTreeMap, time::Duration};

use {
    base64::{Engine as _, engine::general_purpose::STANDARD as BASE64},
    secrecy::{ExposeSecret, SecretString},
};

use crate::{
    exporters::otlp::{OtlpConfig, OtlpTransport},
    profile::ExportProfile,
};

/// Path Langfuse serves OTLP traces on, appended to the configured host.
pub const OTEL_TRACES_PATH: &str = "/api/public/otel/v1/traces";
/// Batch ingestion path, used for scores and dataset run items.
pub const INGESTION_PATH: &str = "/api/public/ingestion";
/// Unauthenticated liveness probe.
pub const HEALTH_PATH: &str = "/api/public/health";
/// Individual score resource, `{id}` appended, used for retraction.
pub const SCORES_PATH: &str = "/api/public/scores";
/// Managed-prompt retrieval, `{name}` appended.
pub const PROMPTS_PATH: &str = "/api/public/v2/prompts";
/// Dataset collection.
pub const DATASETS_PATH: &str = "/api/public/v2/datasets";
/// Dataset item collection.
pub const DATASET_ITEMS_PATH: &str = "/api/public/dataset-items";
/// Dataset run item collection, linking a trace to a dataset item.
pub const DATASET_RUN_ITEMS_PATH: &str = "/api/public/dataset-run-items";
/// Media upload negotiation.
pub const MEDIA_PATH: &str = "/api/public/media";
/// Aggregated daily usage and cost.
pub const DAILY_METRICS_PATH: &str = "/api/public/metrics/daily";

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

    /// Auth headers required by Langfuse's v4 OTLP ingestion contract.
    fn otlp_auth_headers(&self) -> BTreeMap<String, String> {
        let mut headers = self.auth_headers();
        headers.insert("X-Langfuse-Ingestion-Version".to_string(), "4".to_string());
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
            headers: self.otlp_auth_headers(),
            timeout: self.timeout,
            service_name: "moltis".to_string(),
            service_version,
            environment: self.environment.clone(),
            profile: ExportProfile::langfuse(),
        })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use {
        super::*,
        crate::{model::Event, runtime::Transport},
        wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{header, method, path},
        },
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

    #[test]
    fn otlp_headers_request_ingestion_version_four() {
        let headers = config("https://cloud.langfuse.com".into()).otlp_auth_headers();
        assert_eq!(
            headers
                .get("X-Langfuse-Ingestion-Version")
                .map(String::as_str),
            Some("4")
        );
    }

    #[tokio::test]
    async fn ingestion_version_four_is_sent_on_the_wire() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(OTEL_TRACES_PATH))
            .and(header("x-langfuse-ingestion-version", "4"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let transport = config(server.uri()).build_transport("test".into());
        let mut trace = crate::model::TraceRecord::new("turn");
        trace.finish();

        transport
            .send(&[Event::Trace(Box::new(trace))])
            .await
            .expect("OTLP request accepted");
    }
}
