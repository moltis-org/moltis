//! Agent instrumentation with pluggable backends.
//!
//! One instrumentation pass in the agent runtime feeds any number of backends
//! through [`sink::ObservationSink`]. What each backend actually receives is
//! governed by its [`profile::ExportProfile`], because an LLM observability
//! product and an infrastructure APM want very different things:
//!
//! * **Langfuse** gets the full conversation — prompts, completions, tool
//!   arguments and results — plus the observation taxonomy, cache-aware token
//!   usage, and reaction feedback. Langfuse infers cost from the model and
//!   usage details.
//! * **Grafana, Datadog, Honeycomb** get operational shape only: latency,
//!   errors, model, token counts. No conversation content, no per-user
//!   cardinality.
//!
//! See `docs/src/instrumentation.md` for the operator-facing guide.

pub mod builder;
pub mod exporters;
pub mod feedback;
pub mod model;
pub mod profile;
pub mod recent;
pub mod recorder;
pub mod redact;
pub mod runtime;
pub mod sink;

pub use {
    builder::{BuildOutcome, BuiltInstrumentation, SkippedBackend, build},
    feedback::{
        FeedbackSignal, FeedbackVocabulary, USER_FEEDBACK_SCORE, feedback_score, feedback_score_id,
    },
    model::{
        Event, Level, ObservationId, ObservationKind, ObservationRecord, ScoreDeleteRecord,
        ScoreRecord, ScoreValue, TokenUsage, TraceId, TraceRecord, TraceScope,
    },
    profile::{ContentCapture, ExportProfile, Vocabulary},
    recent::{recent_trace, remember_trace},
    recorder::{RecorderSettings, StepGuard, TurnRecorder, wait_for_active_turns},
    redact::RedactionPolicy,
    runtime::{BatchConfig, BatchSink, SinkStatsSnapshot, Transport, TransportError},
    sink::{
        ObservationSink, SinkFanout, clear_global_sink, global_sink, is_enabled, record,
        set_global_sink,
    },
};
