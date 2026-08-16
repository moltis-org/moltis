//! Instrumentation (tracing/observability) configuration.
//!
//! One `[instrumentation]` section configures every backend, because they are
//! fed from a single instrumentation pass in the agent runtime. Each backend
//! has its own sub-table and its own content policy — see the `profile` module
//! in `moltis-observability` for why an LLM observability product and an
//! infrastructure APM must not receive the same payload.

use {
    secrecy::Secret,
    serde::{Deserialize, Serialize},
};

use crate::schema::{deserialize_option_secret, serialize_option_secret};

fn default_true() -> bool {
    true
}

fn default_sample_rate() -> f64 {
    1.0
}

fn default_flush_interval_ms() -> u64 {
    5_000
}

fn default_queue_capacity() -> usize {
    10_000
}

fn default_max_batch_bytes() -> usize {
    3_000_000
}

fn default_timeout_secs() -> u64 {
    10
}

fn default_langfuse_host() -> String {
    "https://cloud.langfuse.com".to_string()
}

/// How much conversation content a backend receives.
///
/// Mirrors `moltis_observability::profile::ContentCapture`; kept as a separate
/// type so `moltis-config` does not depend on the observability crate, and
/// converted at the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentCaptureMode {
    /// Send prompts, completions, tool arguments and tool results.
    Full,
    /// Send structural metadata only: sizes, counts, model, timings.
    #[default]
    MetadataOnly,
    /// Send neither content nor content-derived metadata.
    None,
}

impl ContentCaptureMode {
    /// Accepted values, for config validation messages.
    pub const ALL: &'static [&'static str] = &["full", "metadata_only", "none"];
}

fn default_link_retention_days() -> u32 {
    30
}

/// End-user feedback collected from chat reactions.
///
/// A thumb on a reply is the cheapest quality signal available, but attributing
/// one to the turn that produced it needs the reply's trace to still be on
/// record when the reaction arrives — hence the retention window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FeedbackSettings {
    /// Collect reaction feedback. On by default once instrumentation is
    /// enabled: a scoring pipeline with no scores in it is not useful.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Reaction tokens counted as approval. Empty uses the built-in list.
    ///
    /// Accepts raw emoji or shortcodes; skin tone and presentation selectors
    /// are ignored when matching.
    #[serde(default)]
    pub positive: Vec<String>,
    /// Reaction tokens counted as disapproval. Empty uses the built-in list.
    #[serde(default)]
    pub negative: Vec<String>,
    /// How long a reply stays attributable to its trace, in days.
    ///
    /// Reactions often arrive long after the reply. Kept bounded because the
    /// link table would otherwise grow without limit.
    #[serde(default = "default_link_retention_days")]
    pub link_retention_days: u32,
}

impl Default for FeedbackSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            positive: Vec::new(),
            negative: Vec::new(),
            link_retention_days: default_link_retention_days(),
        }
    }
}

/// Top-level instrumentation configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct InstrumentationConfig {
    /// Master switch. Disabled by default: every backend here ships data off
    /// the machine, so it must be an explicit choice rather than something a
    /// user discovers after the fact.
    #[serde(default)]
    pub enabled: bool,
    /// Deployment environment reported to every backend.
    #[serde(default = "default_environment")]
    pub environment: String,
    /// Build release identifier. Defaults to the running Moltis version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<String>,
    /// Fraction of turns to trace, in `0.0..=1.0`.
    #[serde(default = "default_sample_rate")]
    pub sample_rate: f64,
    /// Extra object keys whose values are redacted before export. Merged with
    /// the built-in list; it cannot be narrowed.
    #[serde(default)]
    pub redact: Vec<String>,
    /// Nonzero bounded export queue depth. Events are dropped once this fills
    /// rather than blocking the agent loop.
    #[serde(default = "default_queue_capacity")]
    pub queue_capacity: usize,
    /// Nonzero maximum time an event waits before being flushed.
    #[serde(default = "default_flush_interval_ms")]
    pub flush_interval_ms: u64,
    /// Nonzero maximum estimated batch size in bytes before a forced flush.
    #[serde(default = "default_max_batch_bytes")]
    pub max_batch_bytes: usize,
    /// Langfuse backend.
    #[serde(default)]
    pub langfuse: LangfuseSettings,
    /// Generic OTLP backend (Grafana Tempo/Alloy, Honeycomb, a collector).
    #[serde(default)]
    pub otlp: OtlpSettings,
    /// Datadog backend, via Datadog's OTLP intake.
    #[serde(default)]
    pub datadog: DatadogSettings,
    /// End-user reaction feedback.
    #[serde(default)]
    pub feedback: FeedbackSettings,
}

fn default_environment() -> String {
    "production".to_string()
}

impl Default for InstrumentationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            environment: default_environment(),
            release: None,
            sample_rate: default_sample_rate(),
            redact: Vec::new(),
            queue_capacity: default_queue_capacity(),
            flush_interval_ms: default_flush_interval_ms(),
            max_batch_bytes: default_max_batch_bytes(),
            langfuse: LangfuseSettings::default(),
            otlp: OtlpSettings::default(),
            datadog: DatadogSettings::default(),
            feedback: FeedbackSettings::default(),
        }
    }
}

impl InstrumentationConfig {
    /// Whether any backend is switched on.
    #[must_use]
    pub fn any_backend_enabled(&self) -> bool {
        self.enabled && (self.langfuse.enabled || self.otlp.enabled || self.datadog.enabled)
    }
}

/// Langfuse-specific settings.
///
/// Langfuse is LLM-native: it wants the whole conversation, and its cost,
/// session and prompt-version views are built on that. Content capture is
/// therefore on by default here, unlike the APM backends.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LangfuseSettings {
    /// Whether to export to Langfuse.
    #[serde(default)]
    pub enabled: bool,
    /// Base host. Set to a self-hosted URL to keep data on-premises.
    #[serde(default = "default_langfuse_host")]
    pub host: String,
    /// Project public key.
    #[serde(default)]
    pub public_key: String,
    /// Project secret key.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_option_secret",
        deserialize_with = "deserialize_option_secret"
    )]
    pub secret_key: Option<Secret<String>>,
    /// Whether to send turn and step inputs.
    #[serde(default = "default_true")]
    pub capture_input: bool,
    /// Whether to send turn and step outputs.
    #[serde(default = "default_true")]
    pub capture_output: bool,
    /// Whether to send tool arguments and results. Separate from the input and
    /// output switches because tool arguments are the likeliest place for
    /// credentials to appear.
    #[serde(default = "default_true")]
    pub capture_tool_io: bool,
    /// Nonzero per-request timeout in seconds.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

impl Default for LangfuseSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            host: default_langfuse_host(),
            public_key: String::new(),
            secret_key: None,
            capture_input: true,
            capture_output: true,
            capture_tool_io: true,
            timeout_secs: default_timeout_secs(),
        }
    }
}

/// Generic OTLP backend settings.
///
/// Targets an OpenTelemetry collector, Grafana Alloy/Tempo, Honeycomb, or
/// anything else speaking OTLP/HTTP. Content capture defaults to
/// `metadata_only`: these are operational tools, and prompt bodies there mean
/// unbounded span size, cardinality pressure, per-byte ingest billing, and
/// conversation content sitting in a system nobody scoped for it.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OtlpSettings {
    /// Whether to export over OTLP.
    #[serde(default)]
    pub enabled: bool,
    /// Full traces endpoint, e.g. `http://localhost:4318/v1/traces`.
    #[serde(default)]
    pub endpoint: String,
    /// Extra headers, typically for authentication.
    #[serde(default)]
    pub headers: std::collections::BTreeMap<String, String>,
    /// How much conversation content this backend receives.
    #[serde(default)]
    pub content: ContentCaptureMode,
    /// Whether to attach end-user identity. Off by default: high-cardinality
    /// in an APM's index and often a compliance question.
    #[serde(default)]
    pub emit_user_id: bool,
    /// Nonzero per-request timeout in seconds.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

impl std::fmt::Debug for OtlpSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OtlpSettings")
            .field("enabled", &self.enabled)
            .field("endpoint", &self.endpoint)
            .field("header_names", &self.headers.keys().collect::<Vec<_>>())
            .field("content", &self.content)
            .field("emit_user_id", &self.emit_user_id)
            .field("timeout_secs", &self.timeout_secs)
            .finish()
    }
}

impl Default for OtlpSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: String::new(),
            headers: std::collections::BTreeMap::new(),
            content: ContentCaptureMode::MetadataOnly,
            emit_user_id: false,
            timeout_secs: default_timeout_secs(),
        }
    }
}

/// Datadog backend settings.
///
/// Datadog ingests OTLP either through the Datadog Agent (default
/// `http://localhost:4318/v1/traces`) or its direct intake. Tags are dropped
/// for this backend because Datadog indexes span tags and bills on custom
/// metric cardinality.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DatadogSettings {
    /// Whether to export to Datadog.
    #[serde(default)]
    pub enabled: bool,
    /// OTLP endpoint exposed by the Datadog Agent or intake.
    #[serde(default = "default_datadog_endpoint")]
    pub endpoint: String,
    /// Datadog API key, required only when posting to the intake directly
    /// rather than through a local Agent.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_option_secret",
        deserialize_with = "deserialize_option_secret"
    )]
    pub api_key: Option<Secret<String>>,
    /// Datadog service name.
    #[serde(default = "default_datadog_service")]
    pub service: String,
    /// How much conversation content this backend receives.
    #[serde(default)]
    pub content: ContentCaptureMode,
    /// Nonzero per-request timeout in seconds.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_datadog_endpoint() -> String {
    "http://localhost:4318/v1/traces".to_string()
}

fn default_datadog_service() -> String {
    "moltis".to_string()
}

impl Default for DatadogSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: default_datadog_endpoint(),
            api_key: None,
            service: default_datadog_service(),
            content: ContentCaptureMode::MetadataOnly,
            timeout_secs: default_timeout_secs(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use secrecy::ExposeSecret;

    use super::*;

    #[test]
    fn instrumentation_is_off_by_default() {
        // Unlike every other Moltis feature flag: enabling this ships
        // conversation content to a third party.
        let config = InstrumentationConfig::default();
        assert!(!config.enabled);
        assert!(!config.langfuse.enabled);
        assert!(!config.otlp.enabled);
        assert!(!config.datadog.enabled);
        assert!(!config.any_backend_enabled());
    }

    #[test]
    fn master_switch_gates_every_backend() {
        let mut config = InstrumentationConfig {
            enabled: false,
            ..Default::default()
        };
        config.langfuse.enabled = true;

        assert!(
            !config.any_backend_enabled(),
            "master switch must override per-backend flags"
        );

        config.enabled = true;
        assert!(config.any_backend_enabled());
    }

    #[test]
    fn langfuse_captures_content_by_default_but_apms_do_not() {
        // The asymmetry is the point: Langfuse is useless without content,
        // an APM is actively harmed by it.
        assert!(LangfuseSettings::default().capture_input);
        assert!(LangfuseSettings::default().capture_tool_io);
        assert_eq!(
            OtlpSettings::default().content,
            ContentCaptureMode::MetadataOnly
        );
        assert_eq!(
            DatadogSettings::default().content,
            ContentCaptureMode::MetadataOnly
        );
    }

    #[test]
    fn apm_backends_do_not_emit_user_id_by_default() {
        assert!(!OtlpSettings::default().emit_user_id);
    }

    #[test]
    fn secret_key_round_trips_through_toml() {
        let toml = r#"
            enabled = true
            host = "https://self-hosted.example.com"
            public_key = "pk-lf-1"
            secret_key = "sk-lf-1"
        "#;
        let parsed: LangfuseSettings = toml::from_str(toml).expect("valid toml");

        assert_eq!(parsed.host, "https://self-hosted.example.com");
        assert_eq!(
            parsed.secret_key.as_ref().map(ExposeSecret::expose_secret),
            Some(&"sk-lf-1".to_string())
        );
    }

    #[test]
    fn debug_output_does_not_leak_the_secret_key() {
        let settings = LangfuseSettings {
            secret_key: Some(Secret::new("sk-lf-supersecret".to_string())),
            ..Default::default()
        };
        let rendered = format!("{settings:?}");

        assert!(
            !rendered.contains("sk-lf-supersecret"),
            "secret leaked into Debug: {rendered}"
        );
    }

    #[test]
    fn debug_output_does_not_leak_otlp_header_values() {
        let settings = OtlpSettings {
            headers: std::collections::BTreeMap::from([(
                "Authorization".into(),
                "Bearer super-secret".into(),
            )]),
            ..Default::default()
        };
        let rendered = format!("{settings:?}");

        assert!(rendered.contains("Authorization"));
        assert!(!rendered.contains("super-secret"));
    }

    #[test]
    fn datadog_defaults_to_the_local_agent_endpoint() {
        // The overwhelmingly common deployment: Agent on localhost, no API key
        // needed in Moltis at all.
        let settings = DatadogSettings::default();
        assert_eq!(settings.endpoint, "http://localhost:4318/v1/traces");
        assert!(settings.api_key.is_none());
    }

    #[test]
    fn content_mode_parses_every_documented_value() {
        for value in ContentCaptureMode::ALL {
            let toml = format!("content = \"{value}\"");
            toml::from_str::<OtlpSettings>(&toml)
                .unwrap_or_else(|e| panic!("{value} should parse: {e}"));
        }
    }

    #[test]
    fn partial_config_fills_in_defaults() {
        // Operators write a two-line section; everything else must default.
        let config: InstrumentationConfig = toml::from_str(
            r#"
            enabled = true
            [langfuse]
            enabled = true
            public_key = "pk"
            "#,
        )
        .expect("valid toml");

        assert!(config.enabled);
        assert_eq!(config.sample_rate, 1.0);
        assert_eq!(config.environment, "production");
        assert_eq!(config.langfuse.host, default_langfuse_host());
        assert_eq!(config.queue_capacity, 10_000);
    }
}
