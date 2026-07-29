//! Backend-agnostic trace model.
//!
//! The shape follows Langfuse's observation taxonomy because it is the richest
//! of the backends we target: OTLP exporters can always project a richer model
//! down onto spans, but the reverse loses tool/generation/retriever semantics.

use std::collections::BTreeMap;

use {
    serde::{Deserialize, Serialize},
    time::OffsetDateTime,
    uuid::Uuid,
};

// ── Identifiers ─────────────────────────────────────────────────────────────

/// Trace identifier. One trace per agent run (one user turn).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TraceId(pub String);

/// Observation identifier, unique within a trace.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ObservationId(pub String);

impl TraceId {
    /// Generate a fresh random trace id.
    #[must_use]
    pub fn generate() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Lowercase 32-char hex form required by the OTLP wire format.
    #[must_use]
    pub fn as_otel_hex(&self) -> String {
        otel_hex(&self.0, 32)
    }
}

impl ObservationId {
    /// Generate a fresh random observation id.
    #[must_use]
    pub fn generate() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Lowercase 16-char hex form required by the OTLP wire format.
    #[must_use]
    pub fn as_otel_hex(&self) -> String {
        otel_hex(&self.0, 16)
    }
}

/// Project an arbitrary id string onto a fixed-width lowercase hex string.
///
/// UUIDs already carry 32 hex digits once dashes are stripped, so the common
/// case is a pure filter. Non-UUID ids (or ids shorter than `width`) are padded
/// deterministically so the same input always yields the same OTLP id.
fn otel_hex(raw: &str, width: usize) -> String {
    let mut hex: String = raw
        .chars()
        .filter(char::is_ascii_hexdigit)
        .map(|c| c.to_ascii_lowercase())
        .take(width)
        .collect();
    if hex.len() < width {
        // Deterministic padding: repeat the digest of the raw id.
        let filler = format!("{:016x}", fnv1a64(raw.as_bytes()));
        while hex.len() < width {
            let need = width - hex.len();
            let take = need.min(filler.len());
            hex.push_str(&filler[..take]);
        }
    }
    hex
}

/// FNV-1a 64-bit. Used only to derive deterministic padding for OTLP ids, never
/// for anything security-relevant.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

// ── Taxonomy ────────────────────────────────────────────────────────────────

/// Observation kind. Mirrors Langfuse's observation taxonomy so the exporter
/// never has to guess at a mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ObservationKind {
    /// Generic unit of work.
    Span,
    /// Point-in-time occurrence with no duration.
    Event,
    /// An LLM completion call.
    Generation,
    /// An agent run, possibly containing nested agents.
    Agent,
    /// A tool invocation.
    Tool,
    /// A fixed multi-step sequence.
    Chain,
    /// A retrieval/search step (memory, code index).
    Retriever,
    /// A quality-evaluation step.
    Evaluator,
    /// An embedding computation.
    Embedding,
    /// A policy check that may block execution.
    Guardrail,
}

impl ObservationKind {
    /// Langfuse OTLP wire representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Span => "span",
            Self::Event => "event",
            Self::Generation => "generation",
            Self::Agent => "agent",
            Self::Tool => "tool",
            Self::Chain => "chain",
            Self::Retriever => "retriever",
            Self::Evaluator => "evaluator",
            Self::Embedding => "embedding",
            Self::Guardrail => "guardrail",
        }
    }

    /// Whether Langfuse treats this kind as generation-like, i.e. carrying
    /// model, usage and cost fields.
    #[must_use]
    pub const fn is_generation_like(self) -> bool {
        matches!(
            self,
            Self::Generation
                | Self::Agent
                | Self::Tool
                | Self::Chain
                | Self::Retriever
                | Self::Evaluator
                | Self::Embedding
                | Self::Guardrail
        )
    }
}

/// Severity attached to an observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Level {
    /// Verbose diagnostic detail.
    Debug,
    /// Normal operation.
    #[default]
    Default,
    /// Recovered from something unexpected.
    Warning,
    /// Failed.
    Error,
}

impl Level {
    /// Langfuse wire representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "DEBUG",
            Self::Default => "DEFAULT",
            Self::Warning => "WARNING",
            Self::Error => "ERROR",
        }
    }
}

// ── Usage and cost ──────────────────────────────────────────────────────────

/// Token usage for a generation.
///
/// `input` is the count of *fresh* (non-cached) input tokens. Callers that
/// receive a provider total which already includes cache reads must subtract
/// before constructing this type — see [`TokenUsage::from_provider_totals`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Fresh input tokens.
    pub input: u32,
    /// Generated output tokens.
    pub output: u32,
    /// Tokens served from the provider's prompt cache.
    pub cache_read: u32,
    /// Tokens written into the provider's prompt cache.
    pub cache_write: u32,
    /// Reasoning tokens, when the provider reports them separately.
    pub reasoning: u32,
}

impl TokenUsage {
    /// Build usage from provider counters where `input` is already exclusive of
    /// cached tokens (Anthropic's `input_tokens` behaves this way).
    #[must_use]
    pub const fn from_provider_totals(
        input: u32,
        output: u32,
        cache_read: u32,
        cache_write: u32,
    ) -> Self {
        Self {
            input,
            output,
            cache_read,
            cache_write,
            reasoning: 0,
        }
    }

    /// Total tokens across every bucket.
    #[must_use]
    pub const fn total(&self) -> u32 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write)
    }

    /// Whether every counter is zero, in which case the field is omitted from
    /// the payload rather than reporting a misleading zero-token generation.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.input == 0
            && self.output == 0
            && self.cache_read == 0
            && self.cache_write == 0
            && self.reasoning == 0
    }

    /// Langfuse `usageDetails` map.
    ///
    /// Key names match what Langfuse's ingestion recognises for cache-aware
    /// cost attribution.
    #[must_use]
    pub fn to_usage_details(self) -> BTreeMap<String, u32> {
        let mut map = BTreeMap::new();
        map.insert("input".to_string(), self.input);
        map.insert("output".to_string(), self.output);
        if self.cache_read > 0 {
            map.insert("input_cache_read".to_string(), self.cache_read);
        }
        if self.cache_write > 0 {
            map.insert("input_cache_write".to_string(), self.cache_write);
        }
        if self.reasoning > 0 {
            map.insert("output_reasoning_tokens".to_string(), self.reasoning);
        }
        map.insert("total".to_string(), self.total());
        map
    }
}

// ── Records ─────────────────────────────────────────────────────────────────

/// Free-form metadata attached to traces and observations.
///
/// `BTreeMap` keeps key order stable, which makes payload assertions in tests
/// deterministic.
pub type Metadata = BTreeMap<String, serde_json::Value>;

/// Trace-scoped attributes.
///
/// These are duplicated onto every observation rather than held only on the
/// trace. OTLP has no trace-level attribute concept — backends reconstruct
/// trace metadata from the spans — so an exporter that kept this only on the
/// trace record would lose user and session attribution whenever a trace event
/// was dropped, batched separately, or evicted from a cache. Duplication costs
/// a few bytes per span and makes exporters stateless.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceScope {
    /// Groups related traces in the backend's session view.
    pub session_id: Option<String>,
    /// End-user identity, when known.
    pub user_id: Option<String>,
    /// Filterable labels.
    pub tags: Vec<String>,
    /// Deployment environment (`production`, `staging`, ...).
    pub environment: Option<String>,
    /// Build release identifier.
    pub release: Option<String>,
    /// Application-level version, e.g. an agent preset version.
    pub version: Option<String>,
}

/// A trace: one agent run, corresponding to one user turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceRecord {
    /// Unique trace id.
    pub id: TraceId,
    /// Human-readable trace name.
    pub name: String,
    /// Wall-clock start.
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
    /// Wall-clock end; `None` until the turn is complete.
    #[serde(with = "time::serde::rfc3339::option")]
    pub end_time: Option<OffsetDateTime>,
    /// Trace-scoped attributes.
    pub scope: TraceScope,
    /// Structured context.
    pub metadata: Metadata,
    /// Turn input, subject to redaction and capture settings.
    pub input: Option<serde_json::Value>,
    /// Turn output, subject to redaction and capture settings.
    pub output: Option<serde_json::Value>,
    /// Whether the backend should expose this trace publicly.
    pub public: bool,
}

impl TraceRecord {
    /// Start a new trace with the given name at the current instant.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: TraceId::generate(),
            name: name.into(),
            timestamp: OffsetDateTime::now_utc(),
            end_time: None,
            scope: TraceScope::default(),
            metadata: Metadata::new(),
            input: None,
            output: None,
            public: false,
        }
    }

    /// Stamp the trace end time as now.
    pub fn finish(&mut self) {
        self.end_time = Some(OffsetDateTime::now_utc());
    }
}

/// A single observation within a trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationRecord {
    /// Unique observation id.
    pub id: ObservationId,
    /// Owning trace.
    pub trace_id: TraceId,
    /// Enclosing observation, when nested.
    pub parent_id: Option<ObservationId>,
    /// What kind of work this represents.
    pub kind: ObservationKind,
    /// Human-readable name.
    pub name: String,
    /// Owning trace's display name, copied for stateless OTLP export.
    pub trace_name: Option<String>,
    /// Owning trace's filterable metadata, copied for stateless OTLP export.
    pub trace_metadata: Metadata,
    /// Trace-scoped attributes, copied from the owning trace.
    pub scope: TraceScope,
    /// Wall-clock start.
    #[serde(with = "time::serde::rfc3339")]
    pub start_time: OffsetDateTime,
    /// Wall-clock end; `None` while still running.
    #[serde(with = "time::serde::rfc3339::option")]
    pub end_time: Option<OffsetDateTime>,
    /// Time the first output token arrived, for streaming generations.
    #[serde(with = "time::serde::rfc3339::option")]
    pub completion_start_time: Option<OffsetDateTime>,
    /// Severity.
    pub level: Level,
    /// Error or warning detail.
    pub status_message: Option<String>,
    /// Step input, subject to redaction and capture settings.
    pub input: Option<serde_json::Value>,
    /// Step output, subject to redaction and capture settings.
    pub output: Option<serde_json::Value>,
    /// Structured context.
    pub metadata: Metadata,
    /// Model identifier, for generation-like observations.
    pub model: Option<String>,
    /// Sampling parameters, for generation-like observations.
    pub model_parameters: Metadata,
    /// Token usage, for generation-like observations.
    pub usage: Option<TokenUsage>,
    /// Managed-prompt name this generation was rendered from.
    pub prompt_name: Option<String>,
    /// Managed-prompt version this generation was rendered from.
    pub prompt_version: Option<i32>,
}

impl ObservationRecord {
    /// Start an observation of `kind` in `trace_id` at the current instant.
    #[must_use]
    pub fn start(trace_id: TraceId, kind: ObservationKind, name: impl Into<String>) -> Self {
        Self {
            id: ObservationId::generate(),
            trace_id,
            parent_id: None,
            kind,
            name: name.into(),
            trace_name: None,
            trace_metadata: Metadata::new(),
            scope: TraceScope::default(),
            start_time: OffsetDateTime::now_utc(),
            end_time: None,
            completion_start_time: None,
            level: Level::Default,
            status_message: None,
            input: None,
            output: None,
            metadata: Metadata::new(),
            model: None,
            model_parameters: Metadata::new(),
            usage: None,
            prompt_name: None,
            prompt_version: None,
        }
    }

    /// Set the parent observation, producing a nested observation.
    #[must_use]
    pub fn with_parent(mut self, parent: Option<ObservationId>) -> Self {
        self.parent_id = parent;
        self
    }

    /// Copy trace-scoped attributes onto this observation.
    #[must_use]
    pub fn with_scope(mut self, scope: TraceScope) -> Self {
        self.scope = scope;
        self
    }

    /// Copy trace identity and filterable metadata onto this observation.
    #[must_use]
    pub fn with_trace_context(mut self, name: String, metadata: Metadata) -> Self {
        self.trace_name = Some(name);
        self.trace_metadata = metadata;
        self
    }

    /// Stamp the end time as now.
    pub fn finish(&mut self) {
        self.end_time = Some(OffsetDateTime::now_utc());
    }

    /// Mark this observation as failed with the given message.
    pub fn fail(&mut self, message: impl Into<String>) {
        self.level = Level::Error;
        self.status_message = Some(message.into());
    }
}

/// Kinds of score value a backend accepts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ScoreValue {
    /// Numeric score.
    Numeric(f64),
    /// Categorical score, e.g. "helpful".
    Categorical(String),
    /// Boolean score, e.g. end-user approval.
    Boolean(bool),
}

/// A quality score attached to a trace or observation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreRecord {
    /// Unique score id.
    pub id: String,
    /// Trace the score applies to.
    pub trace_id: TraceId,
    /// Specific observation, when the score is narrower than the whole trace.
    pub observation_id: Option<ObservationId>,
    /// Score name, e.g. `user-feedback`.
    pub name: String,
    /// The score itself.
    pub value: ScoreValue,
    /// Optional rationale.
    pub comment: Option<String>,
    /// Deployment environment.
    pub environment: Option<String>,
}

/// Delete a previously recorded score through the same ordered queue used for
/// score creation. Keeping both mutations on one transport prevents an older
/// queued create from racing a direct deletion and recreating the score.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoreDeleteRecord {
    /// Trace retained for sink routing and diagnostics.
    pub trace_id: TraceId,
    /// Stable Langfuse score id to delete.
    pub score_id: String,
}

impl ScoreDeleteRecord {
    #[must_use]
    pub fn new(trace_id: TraceId, score_id: impl Into<String>) -> Self {
        Self {
            trace_id,
            score_id: score_id.into(),
        }
    }
}

impl ScoreRecord {
    /// Build a score with a generated id.
    #[must_use]
    pub fn new(trace_id: TraceId, name: impl Into<String>, value: ScoreValue) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            trace_id,
            observation_id: None,
            name: name.into(),
            value,
            comment: None,
            environment: None,
        }
    }
}

/// A single unit of work handed to a sink.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    /// A trace completed.
    Trace(Box<TraceRecord>),
    /// An observation finished.
    ObservationEnd(Box<ObservationRecord>),
    /// A score was produced.
    Score(Box<ScoreRecord>),
    /// A score was retracted.
    ScoreDelete(Box<ScoreDeleteRecord>),
}

impl Event {
    /// The trace this event belongs to.
    #[must_use]
    pub fn trace_id(&self) -> &TraceId {
        match self {
            Self::Trace(t) => &t.id,
            Self::ObservationEnd(o) => &o.trace_id,
            Self::Score(s) => &s.trace_id,
            Self::ScoreDelete(s) => &s.trace_id,
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn usage_details_split_cache_buckets() {
        let usage = TokenUsage::from_provider_totals(100, 50, 900, 20);
        let details = usage.to_usage_details();

        // `input` must stay exclusive of cache buckets: Langfuse prices
        // cache reads and writes at different rates than fresh input, so
        // folding them together silently inflates the reported cost.
        assert_eq!(details.get("input"), Some(&100));
        assert_eq!(details.get("output"), Some(&50));
        assert_eq!(details.get("input_cache_read"), Some(&900));
        assert_eq!(details.get("input_cache_write"), Some(&20));
        assert_eq!(details.get("total"), Some(&1070));
    }

    #[test]
    fn usage_details_omit_zero_cache_buckets() {
        let usage = TokenUsage::from_provider_totals(10, 5, 0, 0);
        let details = usage.to_usage_details();

        assert!(!details.contains_key("input_cache_read"));
        assert!(!details.contains_key("input_cache_write"));
        assert_eq!(details.get("total"), Some(&15));
    }

    #[test]
    fn usage_total_saturates_instead_of_overflowing() {
        let usage = TokenUsage::from_provider_totals(u32::MAX, u32::MAX, 0, 0);
        assert_eq!(usage.total(), u32::MAX);
    }

    #[test]
    fn trace_id_renders_as_32_hex_digits() {
        let hex = TraceId::generate().as_otel_hex();
        assert_eq!(hex.len(), 32);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn observation_id_renders_as_16_hex_digits() {
        let hex = ObservationId::generate().as_otel_hex();
        assert_eq!(hex.len(), 16);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn otel_hex_is_deterministic_for_non_uuid_ids() {
        let id = ObservationId("not-a-uuid".to_string());
        assert_eq!(id.as_otel_hex(), id.as_otel_hex());
        assert_eq!(id.as_otel_hex().len(), 16);
    }

    #[test]
    fn otel_hex_pads_short_ids_to_full_width() {
        // "ab" contributes two hex digits; the rest must be filled.
        let id = TraceId("ab".to_string());
        let hex = id.as_otel_hex();
        assert_eq!(hex.len(), 32);
        assert!(hex.starts_with("ab"));
    }

    #[test]
    fn generation_like_kinds_match_langfuse() {
        assert!(ObservationKind::Generation.is_generation_like());
        assert!(ObservationKind::Tool.is_generation_like());
        assert!(ObservationKind::Agent.is_generation_like());
        assert!(!ObservationKind::Span.is_generation_like());
        assert!(!ObservationKind::Event.is_generation_like());
    }

    #[test]
    fn observation_fail_sets_error_level_and_message() {
        let mut obs =
            ObservationRecord::start(TraceId::generate(), ObservationKind::Generation, "llm-call");
        obs.fail("rate limited");

        assert_eq!(obs.level, Level::Error);
        assert_eq!(obs.status_message.as_deref(), Some("rate limited"));
    }
}
