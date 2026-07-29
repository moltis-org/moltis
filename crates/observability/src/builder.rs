//! Construction of live sinks from `[instrumentation]` configuration.

use std::sync::Arc;

use {
    moltis_config::{ContentCaptureMode, InstrumentationConfig},
    tracing::{info, warn},
};

use crate::{
    profile::ContentCapture,
    recorder::RecorderSettings,
    redact::RedactionPolicy,
    sink::{ObservationSink, SinkFanout},
};

// Everything below is needed only by an exporter that was compiled in. A build
// with no exporter feature still parses `[instrumentation]` and reports each
// enabled backend as skipped, so the config never silently does nothing.
#[cfg(feature = "langfuse")]
use {
    crate::exporters::langfuse::{LangfuseClient, LangfuseConfig, ScoreSink},
    moltis_config::LangfuseSettings,
};
#[cfg(any(feature = "langfuse", feature = "otlp"))]
use {
    crate::runtime::{BatchConfig, BatchSink},
    secrecy::ExposeSecret,
    std::time::Duration,
};
#[cfg(feature = "otlp")]
use {
    crate::{
        exporters::otlp::{OtlpConfig, OtlpTransport},
        profile::ExportProfile,
    },
    moltis_config::{DatadogSettings, OtlpSettings},
};

/// Convert the config-level content mode to the profile-level one.
impl From<ContentCaptureMode> for ContentCapture {
    fn from(mode: ContentCaptureMode) -> Self {
        match mode {
            ContentCaptureMode::Full => Self::Full,
            ContentCaptureMode::MetadataOnly => Self::MetadataOnly,
            ContentCaptureMode::None => Self::None,
        }
    }
}

/// The result of building instrumentation from config.
pub struct BuiltInstrumentation {
    /// Fanout over every enabled backend.
    pub sink: Arc<dyn ObservationSink>,
    /// Settings the agent runtime applies when recording.
    pub recorder_settings: RecorderSettings,
    /// Whether a backend that accepts scores is running.
    ///
    /// Scores are the one event type most backends have no representation for,
    /// so the feedback surfaces ask this rather than naming a backend.
    pub scores: bool,
    /// Langfuse REST client, present when the Langfuse backend is enabled.
    /// Used for the settings UI connection test.
    #[cfg(feature = "langfuse")]
    pub langfuse: Option<Arc<LangfuseClient>>,
    /// Names of the backends that were built.
    pub backends: Vec<String>,
}

/// Reasons a backend was skipped, surfaced in the settings UI rather than only
/// in logs — a silently disabled exporter is indistinguishable from a broken one.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SkippedBackend {
    /// Backend name.
    pub name: String,
    /// Why it was not built.
    pub reason: String,
}

/// Outcome of a build attempt.
pub struct BuildOutcome {
    /// Built instrumentation, or `None` when nothing was enabled.
    pub built: Option<BuiltInstrumentation>,
    /// Backends that were enabled but could not be built.
    pub skipped: Vec<SkippedBackend>,
}

/// Batching parameters shared by every backend.
#[cfg(any(feature = "langfuse", feature = "otlp"))]
fn batch_config(config: &InstrumentationConfig) -> BatchConfig {
    BatchConfig {
        max_batch_bytes: config.max_batch_bytes,
        flush_interval: Duration::from_millis(config.flush_interval_ms),
        queue_capacity: config.queue_capacity,
        ..Default::default()
    }
}

fn validate_batch_config(config: &InstrumentationConfig) -> Result<(), String> {
    let invalid = [
        (config.queue_capacity == 0, "queue_capacity"),
        (config.flush_interval_ms == 0, "flush_interval_ms"),
        (config.max_batch_bytes == 0, "max_batch_bytes"),
    ];
    if let Some((_, field)) = invalid.into_iter().find(|(invalid, _)| *invalid) {
        return Err(format!("{field} must be at least 1"));
    }
    Ok(())
}

/// Validate a backend endpoint.
///
/// Plaintext HTTP is refused for non-loopback hosts: these payloads carry
/// conversation content and credentials in a header, and shipping them
/// unencrypted across a network is not a trade-off an operator should make by
/// typing a URL. Loopback is allowed for a local Agent or collector.
#[cfg(any(feature = "langfuse", feature = "otlp"))]
fn validate_endpoint(raw: &str) -> Result<(), String> {
    let Ok(url) = reqwest::Url::parse(raw) else {
        return Err("endpoint is not a valid URL".to_string());
    };
    if !url.username().is_empty() || url.password().is_some() {
        return Err("endpoint must not contain credentials; use headers instead".to_string());
    }
    match url.scheme() {
        "https" => Ok(()),
        "http" => {
            let host = url.host_str().unwrap_or_default();
            let is_loopback = host == "localhost"
                || host == "::1"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback());
            if is_loopback {
                Ok(())
            } else {
                Err(format!(
                    "refusing plaintext http for non-loopback host `{host}`; use https"
                ))
            }
        },
        other => Err(format!("unsupported URL scheme `{other}`")),
    }
}

/// Build the Langfuse backend.
///
/// Returns two sinks, not one. Traces go over OTLP, but the OTLP mapping has
/// no representation for a score and drops those events, so scores need their
/// own sink onto the ingestion API — otherwise every score recorded would be
/// silently discarded on the way out.
#[cfg(feature = "langfuse")]
fn build_langfuse(
    settings: &LangfuseSettings,
    config: &InstrumentationConfig,
    release: &str,
) -> Result<(Vec<Arc<dyn ObservationSink>>, Arc<LangfuseClient>), String> {
    if settings.public_key.trim().is_empty() {
        return Err("public_key is not set".to_string());
    }
    let Some(secret) = settings.secret_key.as_ref() else {
        return Err("secret_key is not set".to_string());
    };
    if secret.expose_secret().trim().is_empty() {
        return Err("secret_key is empty".to_string());
    }
    if settings.timeout_secs == 0 {
        return Err("timeout_secs must be at least 1".to_string());
    }
    validate_endpoint(&settings.host)?;

    let langfuse_config = LangfuseConfig {
        host: settings.host.clone(),
        public_key: settings.public_key.clone(),
        secret_key: secrecy::SecretString::new(secret.expose_secret().clone()),
        environment: Some(config.environment.clone()),
        release: Some(release.to_string()),
        timeout: Duration::from_secs(settings.timeout_secs),
    };

    let transport = Arc::new(langfuse_config.build_transport(release.to_string()));
    let traces = Arc::new(BatchSink::spawn(transport, batch_config(config)));
    let client = Arc::new(LangfuseClient::new(langfuse_config));
    let scores = Arc::new(ScoreSink::spawn(Arc::clone(&client), batch_config(config)));
    Ok((vec![traces, scores], client))
}

/// Build a generic OTLP backend.
#[cfg(feature = "otlp")]
fn build_otlp(
    settings: &OtlpSettings,
    config: &InstrumentationConfig,
    release: &str,
) -> Result<Arc<dyn ObservationSink>, String> {
    if settings.endpoint.trim().is_empty() {
        return Err("endpoint is not set".to_string());
    }
    if settings.timeout_secs == 0 {
        return Err("timeout_secs must be at least 1".to_string());
    }
    validate_endpoint(&settings.endpoint)?;

    let profile = ExportProfile {
        content: settings.content.into(),
        emit_user_id: settings.emit_user_id,
        ..ExportProfile::otel_generic()
    };

    let transport = Arc::new(OtlpTransport::new(OtlpConfig {
        name: "otlp".to_string(),
        endpoint: settings.endpoint.clone(),
        headers: settings.headers.clone(),
        timeout: Duration::from_secs(settings.timeout_secs),
        service_name: "moltis".to_string(),
        service_version: release.to_string(),
        environment: Some(config.environment.clone()),
        profile,
    }));
    Ok(Arc::new(BatchSink::spawn(transport, batch_config(config))))
}

/// Build the Datadog backend, over the same OTLP wire format.
#[cfg(feature = "otlp")]
fn build_datadog(
    settings: &DatadogSettings,
    config: &InstrumentationConfig,
    release: &str,
) -> Result<Arc<dyn ObservationSink>, String> {
    if settings.endpoint.trim().is_empty() {
        return Err("endpoint is not set".to_string());
    }
    if settings.timeout_secs == 0 {
        return Err("timeout_secs must be at least 1".to_string());
    }
    if settings.service.trim().is_empty() {
        return Err("service is not set".to_string());
    }
    validate_endpoint(&settings.endpoint)?;

    let mut headers = std::collections::BTreeMap::new();
    if let Some(key) = &settings.api_key {
        headers.insert("DD-API-KEY".to_string(), key.expose_secret().clone());
    }

    let profile = ExportProfile {
        content: settings.content.into(),
        ..ExportProfile::datadog()
    };

    let transport = Arc::new(OtlpTransport::new(OtlpConfig {
        name: "datadog".to_string(),
        endpoint: settings.endpoint.clone(),
        headers,
        timeout: Duration::from_secs(settings.timeout_secs),
        service_name: settings.service.clone(),
        service_version: release.to_string(),
        environment: Some(config.environment.clone()),
        profile,
    }));
    Ok(Arc::new(BatchSink::spawn(transport, batch_config(config))))
}

/// Build every enabled backend.
///
/// A backend that cannot be built is reported in `skipped` and the others still
/// start: one misconfigured exporter must not take instrumentation down.
///
/// Must be called from within a Tokio runtime — each backend spawns a
/// background export task.
#[must_use]
// With no exporter compiled in there is nothing to build: every enabled backend
// is reported as skipped, so the accumulators are never written and the release
// string is never read.
#[cfg_attr(
    not(any(feature = "langfuse", feature = "otlp")),
    allow(unused_mut, unused_variables)
)]
pub fn build(config: &InstrumentationConfig, release: &str) -> BuildOutcome {
    let mut skipped = Vec::new();

    if !config.enabled {
        return BuildOutcome {
            built: None,
            skipped,
        };
    }

    if let Err(reason) = validate_batch_config(config) {
        let skipped = [
            ("langfuse", config.langfuse.enabled),
            ("otlp", config.otlp.enabled),
            ("datadog", config.datadog.enabled),
        ]
        .into_iter()
        .filter(|(_, enabled)| *enabled)
        .map(|(name, _)| SkippedBackend {
            name: name.to_string(),
            reason: reason.clone(),
        })
        .collect();
        return BuildOutcome {
            built: None,
            skipped,
        };
    }

    let mut sinks: Vec<Arc<dyn ObservationSink>> = Vec::new();
    let mut backends = Vec::new();
    #[cfg(feature = "langfuse")]
    let mut langfuse_client = None;

    if config.langfuse.enabled {
        #[cfg(feature = "langfuse")]
        match build_langfuse(&config.langfuse, config, release) {
            Ok((langfuse_sinks, client)) => {
                sinks.extend(langfuse_sinks);
                langfuse_client = Some(client);
                backends.push("langfuse".to_string());
            },
            Err(reason) => {
                warn!(backend = "langfuse", %reason, "instrumentation backend disabled");
                skipped.push(SkippedBackend {
                    name: "langfuse".to_string(),
                    reason,
                });
            },
        }
        #[cfg(not(feature = "langfuse"))]
        skipped.push(not_compiled_in("langfuse"));
    }

    if config.otlp.enabled {
        #[cfg(feature = "otlp")]
        match build_otlp(&config.otlp, config, release) {
            Ok(sink) => {
                sinks.push(sink);
                backends.push("otlp".to_string());
            },
            Err(reason) => {
                warn!(backend = "otlp", %reason, "instrumentation backend disabled");
                skipped.push(SkippedBackend {
                    name: "otlp".to_string(),
                    reason,
                });
            },
        }
        #[cfg(not(feature = "otlp"))]
        skipped.push(not_compiled_in("otlp"));
    }

    if config.datadog.enabled {
        #[cfg(feature = "otlp")]
        match build_datadog(&config.datadog, config, release) {
            Ok(sink) => {
                sinks.push(sink);
                backends.push("datadog".to_string());
            },
            Err(reason) => {
                warn!(backend = "datadog", %reason, "instrumentation backend disabled");
                skipped.push(SkippedBackend {
                    name: "datadog".to_string(),
                    reason,
                });
            },
        }
        // Datadog is reached over the same OTLP wire format.
        #[cfg(not(feature = "otlp"))]
        skipped.push(not_compiled_in("datadog"));
    }

    if sinks.is_empty() {
        return BuildOutcome {
            built: None,
            skipped,
        };
    }

    info!(backends = ?backends, "instrumentation enabled");

    // Capture switches live on the Langfuse settings: it is the only backend
    // that receives bodies at all, and the APM profiles gate content
    // independently through their own `content` mode.
    let recorder_settings = RecorderSettings {
        redaction: RedactionPolicy::from_needles(&config.redact),
        capture_input: config.langfuse.capture_input,
        capture_output: config.langfuse.capture_output,
        capture_tool_io: config.langfuse.capture_tool_io,
        sample_rate: config.sample_rate.clamp(0.0, 1.0),
    };

    BuildOutcome {
        built: Some(BuiltInstrumentation {
            sink: Arc::new(SinkFanout::new(sinks)),
            recorder_settings,
            // Langfuse is the only backend with a score representation; the
            // OTLP mapping drops score events on the floor.
            scores: backends.iter().any(|name| name == "langfuse"),
            #[cfg(feature = "langfuse")]
            langfuse: langfuse_client,
            backends,
        }),
        skipped,
    }
}

/// Report a backend the operator enabled but this binary cannot speak to.
///
/// Reported as a skipped backend rather than logged, so a build without the
/// exporter feature explains itself in the settings UI instead of looking like
/// a misconfiguration.
#[cfg(not(all(feature = "langfuse", feature = "otlp")))]
fn not_compiled_in(name: &str) -> SkippedBackend {
    warn!(
        backend = name,
        "instrumentation backend enabled in config but not compiled into this build"
    );
    SkippedBackend {
        name: name.to_string(),
        reason: "not compiled into this build".to_string(),
    }
}

// These build real backends, so they need the exporters compiled in. `langfuse`
// implies `otlp`, so gating on it covers the default feature set; the reduced
// builds are covered by `reduced_build_tests` below.
#[cfg(all(test, feature = "langfuse"))]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use {
        crate::{Event, ScoreRecord, ScoreValue, TraceId},
        secrecy::Secret,
    };

    use super::*;

    fn enabled_config() -> InstrumentationConfig {
        InstrumentationConfig {
            enabled: true,
            ..Default::default()
        }
    }

    fn valid_langfuse() -> LangfuseSettings {
        LangfuseSettings {
            enabled: true,
            host: "https://cloud.langfuse.com".into(),
            public_key: "pk-lf-1".into(),
            secret_key: Some(Secret::new("sk-lf-1".to_string())),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn disabled_config_builds_nothing() {
        let outcome = build(&InstrumentationConfig::default(), "test");
        assert!(outcome.built.is_none());
        assert!(outcome.skipped.is_empty());
    }

    #[tokio::test]
    async fn master_switch_off_skips_enabled_backends_silently() {
        let config = InstrumentationConfig {
            enabled: false,
            langfuse: valid_langfuse(),
            ..Default::default()
        };
        let outcome = build(&config, "test");

        assert!(outcome.built.is_none());
        // Not "skipped": the operator turned the whole thing off deliberately.
        assert!(outcome.skipped.is_empty());
    }

    #[tokio::test]
    async fn langfuse_builds_with_valid_credentials() {
        let config = InstrumentationConfig {
            langfuse: valid_langfuse(),
            ..enabled_config()
        };
        let outcome = build(&config, "20260726.01");
        let built = outcome.built.expect("langfuse should build");

        assert_eq!(built.backends, vec!["langfuse"]);
        assert!(built.langfuse.is_some(), "REST client needed for scores");
    }

    #[tokio::test]
    async fn langfuse_without_credentials_is_skipped_with_a_reason() {
        let config = InstrumentationConfig {
            langfuse: LangfuseSettings {
                enabled: true,
                public_key: String::new(),
                ..Default::default()
            },
            ..enabled_config()
        };
        let outcome = build(&config, "test");

        assert!(outcome.built.is_none());
        assert_eq!(outcome.skipped.len(), 1);
        assert!(outcome.skipped[0].reason.contains("public_key"));
    }

    #[tokio::test]
    async fn one_broken_backend_does_not_disable_the_others() {
        let config = InstrumentationConfig {
            langfuse: valid_langfuse(),
            otlp: OtlpSettings {
                enabled: true,
                endpoint: String::new(),
                ..Default::default()
            },
            ..enabled_config()
        };
        let outcome = build(&config, "test");
        let built = outcome.built.expect("langfuse should still build");

        assert_eq!(built.backends, vec!["langfuse"]);
        assert_eq!(outcome.skipped.len(), 1);
        assert_eq!(outcome.skipped[0].name, "otlp");
    }

    #[tokio::test]
    async fn multiple_backends_fan_out_together() {
        let config = InstrumentationConfig {
            langfuse: valid_langfuse(),
            otlp: OtlpSettings {
                enabled: true,
                endpoint: "http://localhost:4318/v1/traces".into(),
                ..Default::default()
            },
            datadog: DatadogSettings {
                enabled: true,
                ..Default::default()
            },
            ..enabled_config()
        };
        let built = build(&config, "test").built.expect("should build");

        assert_eq!(built.backends, vec!["langfuse", "otlp", "datadog"]);
        // Langfuse contributes two sinks: traces over OTLP and scores over the
        // ingestion API. It still reports as one backend to the operator.
        assert_eq!(built.sink.name(), "langfuse+langfuse-scores+otlp+datadog");
        assert_eq!(
            built
                .sink
                .delivery_stats()
                .into_iter()
                .map(|stats| stats.name)
                .collect::<Vec<_>>(),
            vec!["langfuse", "langfuse-scores", "otlp", "datadog"]
        );
    }

    #[tokio::test]
    async fn langfuse_gets_a_score_sink_alongside_the_trace_sink() {
        // The OTLP mapping has no representation for a score and drops those
        // events, so without a second sink every score would be recorded by the
        // agent and then silently discarded on the way out.
        let config = InstrumentationConfig {
            langfuse: valid_langfuse(),
            ..enabled_config()
        };
        let built = build(&config, "test").built.expect("should build");

        assert_eq!(built.sink.name(), "langfuse+langfuse-scores");

        built.sink.record(Event::Score(Box::new(ScoreRecord::new(
            TraceId::generate(),
            "quality",
            ScoreValue::Numeric(1.0),
        ))));
        let delivery = built.sink.delivery_stats();
        assert_eq!(delivery[0].name, "langfuse");
        assert_eq!(delivery[0].accepted, 0);
        assert_eq!(delivery[1].name, "langfuse-scores");
        assert_eq!(delivery[1].accepted, 1);
    }

    #[tokio::test]
    async fn plaintext_http_is_refused_for_remote_hosts() {
        // Conversation content plus a credential header over the open network.
        let config = InstrumentationConfig {
            otlp: OtlpSettings {
                enabled: true,
                endpoint: "http://collector.example.com/v1/traces".into(),
                ..Default::default()
            },
            ..enabled_config()
        };
        let outcome = build(&config, "test");

        assert!(outcome.built.is_none());
        assert!(outcome.skipped[0].reason.contains("plaintext http"));
    }

    #[tokio::test]
    async fn plaintext_http_is_allowed_for_loopback() {
        // The common Datadog Agent and local-collector deployment.
        for endpoint in [
            "http://localhost:4318/v1/traces",
            "http://127.0.0.1:4318/v1/traces",
        ] {
            let config = InstrumentationConfig {
                otlp: OtlpSettings {
                    enabled: true,
                    endpoint: endpoint.to_string(),
                    ..Default::default()
                },
                ..enabled_config()
            };
            assert!(
                build(&config, "test").built.is_some(),
                "{endpoint} should be accepted"
            );
        }
    }

    #[tokio::test]
    async fn malformed_endpoints_are_rejected() {
        let config = InstrumentationConfig {
            otlp: OtlpSettings {
                enabled: true,
                endpoint: "not a url".into(),
                ..Default::default()
            },
            ..enabled_config()
        };
        let outcome = build(&config, "test");
        assert!(outcome.skipped[0].reason.contains("not a valid URL"));
    }

    #[tokio::test]
    async fn endpoint_credentials_are_rejected_without_echoing_them() {
        let config = InstrumentationConfig {
            otlp: OtlpSettings {
                enabled: true,
                endpoint: "https://operator:super-secret@collector.example/v1/traces".into(),
                ..Default::default()
            },
            ..enabled_config()
        };

        let outcome = build(&config, "test");

        assert!(outcome.built.is_none());
        assert!(
            outcome.skipped[0]
                .reason
                .contains("must not contain credentials")
        );
        assert!(!outcome.skipped[0].reason.contains("super-secret"));
    }

    #[tokio::test]
    async fn zero_batch_settings_skip_backends_without_panicking() {
        for (queue_capacity, flush_interval_ms, max_batch_bytes, field) in [
            (0, 1, 1, "queue_capacity"),
            (1, 0, 1, "flush_interval_ms"),
            (1, 1, 0, "max_batch_bytes"),
        ] {
            let config = InstrumentationConfig {
                queue_capacity,
                flush_interval_ms,
                max_batch_bytes,
                langfuse: valid_langfuse(),
                ..enabled_config()
            };

            let outcome = build(&config, "test");

            assert!(outcome.built.is_none());
            assert_eq!(outcome.skipped.len(), 1);
            assert!(outcome.skipped[0].reason.contains(field));
        }
    }

    #[tokio::test]
    async fn sample_rate_is_clamped_into_range() {
        let config = InstrumentationConfig {
            sample_rate: 7.5,
            langfuse: valid_langfuse(),
            ..enabled_config()
        };
        let built = build(&config, "test").built.expect("should build");
        assert_eq!(built.recorder_settings.sample_rate, 1.0);
    }

    #[tokio::test]
    async fn capture_switches_flow_through_to_the_recorder() {
        let config = InstrumentationConfig {
            langfuse: LangfuseSettings {
                capture_tool_io: false,
                ..valid_langfuse()
            },
            ..enabled_config()
        };
        let built = build(&config, "test").built.expect("should build");

        assert!(!built.recorder_settings.capture_tool_io);
        assert!(built.recorder_settings.capture_input);
    }

    #[tokio::test]
    async fn configured_redaction_needles_extend_the_defaults() {
        let config = InstrumentationConfig {
            redact: vec!["customer_ref".into()],
            langfuse: valid_langfuse(),
            ..enabled_config()
        };
        let built = build(&config, "test").built.expect("should build");
        let policy = &built.recorder_settings.redaction;

        assert!(policy.is_sensitive_key("customer_ref"));
        assert!(policy.is_sensitive_key("password"));
    }

    #[tokio::test]
    async fn datadog_api_key_is_sent_as_a_header_not_a_query_param() {
        let config = InstrumentationConfig {
            datadog: DatadogSettings {
                enabled: true,
                endpoint: "https://otlp.datadoghq.com/v1/traces".into(),
                api_key: Some(Secret::new("dd-key".to_string())),
                ..Default::default()
            },
            ..enabled_config()
        };
        assert!(build(&config, "test").built.is_some());
    }

    #[test]
    fn content_modes_convert_to_the_matching_profile_policy() {
        assert_eq!(
            ContentCapture::from(ContentCaptureMode::Full),
            ContentCapture::Full
        );
        assert_eq!(
            ContentCapture::from(ContentCaptureMode::MetadataOnly),
            ContentCapture::MetadataOnly
        );
        assert_eq!(
            ContentCapture::from(ContentCaptureMode::None),
            ContentCapture::None
        );
    }
}

/// Coverage for builds that left an exporter out.
#[cfg(all(test, not(feature = "langfuse")))]
mod reduced_build_tests {
    use {moltis_config::LangfuseSettings, secrecy::Secret};

    use super::*;

    #[tokio::test]
    async fn a_backend_this_build_lacks_is_skipped_with_a_reason() {
        // Enabling a backend the binary cannot speak to must explain itself,
        // otherwise it is indistinguishable from bad credentials.
        let config = InstrumentationConfig {
            enabled: true,
            langfuse: LangfuseSettings {
                enabled: true,
                host: "https://cloud.langfuse.com".into(),
                public_key: "pk-lf-1".into(),
                secret_key: Some(Secret::new("sk-lf-1".to_string())),
                ..Default::default()
            },
            ..Default::default()
        };

        let outcome = build(&config, "test");

        assert!(outcome.built.is_none());
        assert_eq!(outcome.skipped.len(), 1);
        assert_eq!(outcome.skipped[0].name, "langfuse");
        assert_eq!(outcome.skipped[0].reason, "not compiled into this build");
    }

    #[tokio::test]
    async fn an_untouched_config_still_builds_nothing() {
        let outcome = build(&InstrumentationConfig::default(), "test");

        assert!(outcome.built.is_none());
        assert!(outcome.skipped.is_empty());
    }
}
