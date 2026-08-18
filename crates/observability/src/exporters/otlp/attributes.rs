//! Attribute names emitted on every exported span.
//!
//! Two vocabularies are written side by side:
//!
//! * `gen_ai.*` — the OpenTelemetry GenAI semantic conventions, understood by
//!   Grafana, Datadog, Honeycomb and anything else that speaks OTel.
//! * `langfuse.*` — Langfuse's own mapping, which carries the richer
//!   observation taxonomy (AGENT/TOOL/RETRIEVER/...), usage and cost detail,
//!   and managed-prompt linkage that the GenAI conventions have no slot for.
//!
//! Emitting both costs a few hundred bytes per span and means a single
//! instrumentation pass serves every backend. Unknown attributes are ignored by
//! conformant backends, so the extra keys are inert where unrecognised.
//!
//! The `langfuse.*` names are taken verbatim from
//! `packages/shared/src/server/otel/attributes.ts` in the Langfuse source,
//! which is ahead of the published documentation page.

// ── Langfuse: trace scope ───────────────────────────────────────────────────

/// Trace display name.
pub const TRACE_NAME: &str = "langfuse.trace.name";
/// End-user identity. Primary spelling; `langfuse.user.id` is the legacy alias.
pub const USER_ID: &str = "user.id";
/// Session grouping. Primary spelling; `langfuse.session.id` is the legacy alias.
pub const SESSION_ID: &str = "session.id";
/// Filterable trace labels.
pub const TRACE_TAGS: &str = "langfuse.trace.tags";
/// Trace-level input payload.
pub const TRACE_INPUT: &str = "langfuse.trace.input";
/// Trace-level output payload.
pub const TRACE_OUTPUT: &str = "langfuse.trace.output";
/// Trace-level structured metadata.
pub const TRACE_METADATA: &str = "langfuse.trace.metadata";
/// Whether the trace is publicly viewable.
pub const TRACE_PUBLIC: &str = "langfuse.trace.public";

// ── Langfuse: observation ───────────────────────────────────────────────────

/// Observation taxonomy slot (`GENERATION`, `TOOL`, `AGENT`, ...).
pub const OBSERVATION_TYPE: &str = "langfuse.observation.type";
/// Observation severity.
pub const OBSERVATION_LEVEL: &str = "langfuse.observation.level";
/// Error or warning detail.
pub const OBSERVATION_STATUS_MESSAGE: &str = "langfuse.observation.status_message";
/// Observation input payload.
pub const OBSERVATION_INPUT: &str = "langfuse.observation.input";
/// Observation output payload.
pub const OBSERVATION_OUTPUT: &str = "langfuse.observation.output";
/// Observation structured metadata.
pub const OBSERVATION_METADATA: &str = "langfuse.observation.metadata";
/// Time the first output token arrived.
pub const OBSERVATION_COMPLETION_START_TIME: &str = "langfuse.observation.completion_start_time";
/// Model identifier.
pub const OBSERVATION_MODEL: &str = "langfuse.observation.model.name";
/// Sampling parameters.
pub const OBSERVATION_MODEL_PARAMETERS: &str = "langfuse.observation.model.parameters";
/// Token usage breakdown.
pub const OBSERVATION_USAGE_DETAILS: &str = "langfuse.observation.usage_details";
/// Managed-prompt name this generation was rendered from.
pub const OBSERVATION_PROMPT_NAME: &str = "langfuse.observation.prompt.name";
/// Managed-prompt version this generation was rendered from.
pub const OBSERVATION_PROMPT_VERSION: &str = "langfuse.observation.prompt.version";

// ── Langfuse: general ───────────────────────────────────────────────────────

/// Deployment environment.
pub const ENVIRONMENT: &str = "langfuse.environment";
/// Build release identifier.
pub const RELEASE: &str = "langfuse.release";
/// Application-level version.
pub const VERSION: &str = "langfuse.version";

// ── OpenTelemetry GenAI semantic conventions ────────────────────────────────

/// Provider name, e.g. `anthropic`.
pub const GEN_AI_SYSTEM: &str = "gen_ai.system";
/// Operation being performed, e.g. `chat`.
pub const GEN_AI_OPERATION_NAME: &str = "gen_ai.operation.name";
/// Model requested.
pub const GEN_AI_REQUEST_MODEL: &str = "gen_ai.request.model";
/// Model that actually served the request.
pub const GEN_AI_RESPONSE_MODEL: &str = "gen_ai.response.model";
/// Requested sampling temperature.
pub const GEN_AI_REQUEST_TEMPERATURE: &str = "gen_ai.request.temperature";
/// Requested output token ceiling.
pub const GEN_AI_REQUEST_MAX_TOKENS: &str = "gen_ai.request.max_tokens";
/// Fresh input tokens consumed.
pub const GEN_AI_USAGE_INPUT_TOKENS: &str = "gen_ai.usage.input_tokens";
/// Output tokens generated.
pub const GEN_AI_USAGE_OUTPUT_TOKENS: &str = "gen_ai.usage.output_tokens";
/// Conversation grouping id.
pub const GEN_AI_CONVERSATION_ID: &str = "gen_ai.conversation.id";
/// Tool name, for tool spans.
pub const GEN_AI_TOOL_NAME: &str = "gen_ai.tool.name";

// ── Resource attributes ─────────────────────────────────────────────────────

/// Emitting service name.
pub const SERVICE_NAME: &str = "service.name";
/// Emitting service version.
pub const SERVICE_VERSION: &str = "service.version";
/// Deployment environment, OTel's own spelling.
pub const DEPLOYMENT_ENVIRONMENT: &str = "deployment.environment.name";
