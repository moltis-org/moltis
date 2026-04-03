use std::{collections::HashMap, time::Duration};

use {
    anyhow::{Context, Result, anyhow},
    base64::Engine,
    moltis_config::{MoltisConfig, VERSION},
    opentelemetry::{KeyValue, global},
    opentelemetry_otlp::{Protocol, SpanExporter, WithExportConfig, WithHttpConfig},
    opentelemetry_sdk::{
        Resource,
        trace::{Sampler, SdkTracerProvider},
    },
    secrecy::ExposeSecret,
};

pub struct TelemetryHandles {
    tracer_provider: Option<SdkTracerProvider>,
}

impl TelemetryHandles {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            tracer_provider: None,
        }
    }

    #[must_use]
    pub const fn new(tracer_provider: SdkTracerProvider) -> Self {
        Self {
            tracer_provider: Some(tracer_provider),
        }
    }

    pub fn shutdown(self) {
        if let Some(provider) = self.tracer_provider
            && let Err(error) = provider.shutdown()
        {
            eprintln!("failed to flush OpenTelemetry spans during shutdown: {error}");
        }
    }
}

#[derive(Debug)]
pub struct LangfuseTracing {
    pub provider: SdkTracerProvider,
}

pub fn build_langfuse_tracing(config: &MoltisConfig) -> Result<Option<LangfuseTracing>> {
    let langfuse = &config.metrics.langfuse;
    if !langfuse.enabled {
        return Ok(None);
    }

    let host = langfuse
        .host
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow!("metrics.langfuse.enabled = true but metrics.langfuse.host is missing")
        })?;
    let public_key = langfuse.public_key.as_ref().ok_or_else(|| {
        anyhow!("metrics.langfuse.enabled = true but metrics.langfuse.public_key is missing")
    })?;
    let secret_key = langfuse.secret_key.as_ref().ok_or_else(|| {
        anyhow!("metrics.langfuse.enabled = true but metrics.langfuse.secret_key is missing")
    })?;

    let endpoint = build_langfuse_traces_endpoint(host);
    let exporter = SpanExporter::builder()
        .with_http()
        .with_protocol(Protocol::HttpBinary)
        .with_endpoint(endpoint)
        .with_timeout(Duration::from_secs(10))
        .with_headers(langfuse_headers(
            public_key.expose_secret(),
            secret_key.expose_secret(),
        ))
        .build()
        .context("failed to build Langfuse OTLP exporter")?;

    let mut resource = vec![
        KeyValue::new("service.name", "moltis"),
        KeyValue::new("service.version", VERSION.to_string()),
    ];
    if let Some(environment) = langfuse
        .environment
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        resource.push(KeyValue::new(
            "deployment.environment",
            environment.to_string(),
        ));
    }

    let sample_rate = langfuse.sample_rate.clamp(0.0, 1.0);
    let provider = SdkTracerProvider::builder()
        .with_sampler(Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(
            sample_rate,
        ))))
        .with_resource(Resource::builder_empty().with_attributes(resource).build())
        .with_batch_exporter(exporter)
        .build();

    global::set_tracer_provider(provider.clone());

    Ok(Some(LangfuseTracing { provider }))
}

fn build_langfuse_traces_endpoint(host: &str) -> String {
    let trimmed = host.trim().trim_end_matches('/');
    format!("{trimmed}/api/public/otel/v1/traces")
}

fn langfuse_headers(public_key: &str, secret_key: &str) -> HashMap<String, String> {
    let auth =
        base64::engine::general_purpose::STANDARD.encode(format!("{public_key}:{secret_key}"));
    HashMap::from([
        ("Authorization".to_string(), format!("Basic {auth}")),
        ("x-langfuse-ingestion-version".to_string(), "4".to_string()),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_langfuse_traces_endpoint() {
        assert_eq!(
            build_langfuse_traces_endpoint("https://cloud.langfuse.com/"),
            "https://cloud.langfuse.com/api/public/otel/v1/traces"
        );
    }

    #[test]
    fn langfuse_headers_include_basic_auth_and_ingestion_version() {
        let headers = langfuse_headers("pk", "sk");
        assert_eq!(
            headers.get("Authorization").map(String::as_str),
            Some("Basic cGs6c2s=")
        );
        assert_eq!(
            headers
                .get("x-langfuse-ingestion-version")
                .map(String::as_str),
            Some("4")
        );
    }

    #[test]
    fn disabled_langfuse_returns_none() {
        let config = MoltisConfig::default();
        assert!(build_langfuse_tracing(&config).expect("ok").is_none());
    }

    #[test]
    fn enabled_langfuse_requires_credentials() {
        let mut config = MoltisConfig::default();
        config.metrics.langfuse = moltis_config::schema::LangfuseConfig {
            enabled: true,
            host: Some("https://cloud.langfuse.com".to_string()),
            ..moltis_config::schema::LangfuseConfig::default()
        };

        let error = build_langfuse_tracing(&config).expect_err("should fail");
        assert!(error.to_string().contains("public_key"));
    }
}
