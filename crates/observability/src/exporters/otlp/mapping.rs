//! Projection of the internal trace model onto OTLP spans.
//!
//! Every emission decision is gated on the sink's [`ExportProfile`], so the
//! same agent run yields a content-rich payload for Langfuse and a lean
//! operational span for Datadog or Grafana. See `crate::profile` for why.

use std::collections::HashSet;

use time::OffsetDateTime;

use {
    super::{attributes as attr, wire},
    crate::{
        model::{Event, Level, ObservationRecord, TraceRecord, TraceScope},
        profile::ExportProfile,
    },
};

/// Instrumentation scope name reported to backends.
pub const SCOPE_NAME: &str = "moltis";

/// Identity and policy applied to one export batch.
#[derive(Debug, Clone)]
pub struct ExportContext {
    /// Value reported as `service.name`.
    pub service_name: String,
    /// Value reported as `service.version`.
    pub service_version: String,
    /// Deployment environment.
    pub environment: Option<String>,
    /// What this backend is allowed to receive.
    pub profile: ExportProfile,
}

/// Convert an RFC3339 instant to nanoseconds since the Unix epoch.
fn unix_nanos(at: OffsetDateTime) -> String {
    let nanos = at.unix_timestamp_nanos();
    // OTLP treats the field as unsigned; pre-epoch instants cannot occur here
    // but clamping is cheaper than reasoning about a negative timestamp
    // silently wrapping in a collector.
    if nanos < 0 {
        return "0".to_string();
    }
    nanos.to_string()
}

/// Truncate an attribute value to the profile's ceiling on a char boundary.
fn clamp(text: String, max: usize) -> String {
    if max == 0 || text.len() <= max {
        return text;
    }
    const SUFFIX: &str = "...";
    if max <= SUFFIX.len() {
        return SUFFIX[..max].to_string();
    }
    let mut end = max - SUFFIX.len();
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{SUFFIX}", &text[..end])
}

/// Serialize a value for an attribute slot that expects JSON text.
fn json_attr(key: &str, value: &serde_json::Value, max: usize) -> Option<wire::KeyValue> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => Some(wire::KeyValue::string(key, clamp(s.clone(), max))),
        other => bounded_json(other, max).map(|s| wire::KeyValue::string(key, s)),
    }
}

/// Serialize structured JSON without ever truncating it into invalid syntax.
fn bounded_json(value: &serde_json::Value, max: usize) -> Option<String> {
    let serialized = serde_json::to_string(value).ok()?;
    if max == 0 || serialized.len() <= max {
        return Some(serialized);
    }

    let marker = serde_json::json!({
        "moltis_truncated": true,
        "original_bytes": serialized.len(),
    })
    .to_string();
    if marker.len() <= max {
        return Some(marker);
    }
    (max >= 4).then(|| "null".to_string())
}

fn push_structured_attr(
    out: &mut Vec<wire::KeyValue>,
    key: &str,
    value: &serde_json::Value,
    max: usize,
) {
    if let Some(serialized) = bounded_json(value, max) {
        out.push(wire::KeyValue::string(key, serialized));
    }
}

/// Approximate size of a payload, for backends that get counts but not content.
fn payload_size(value: &serde_json::Value) -> i64 {
    match value {
        serde_json::Value::String(s) => s.len() as i64,
        other => serde_json::to_string(other)
            .map(|s| s.len() as i64)
            .unwrap_or(0),
    }
}

/// Append attributes carried by every span in a trace.
fn push_scope_attrs(out: &mut Vec<wire::KeyValue>, scope: &TraceScope, profile: &ExportProfile) {
    if let Some(session) = &scope.session_id
        && profile.emit_session_id
    {
        if profile.emits_langfuse_attrs() {
            out.push(wire::KeyValue::string(attr::SESSION_ID, session.clone()));
        }
        // GenAI semconv's own conversation grouping, understood everywhere.
        out.push(wire::KeyValue::string(
            attr::GEN_AI_CONVERSATION_ID,
            session.clone(),
        ));
    }
    if let Some(user) = &scope.user_id
        && profile.emit_user_id
    {
        out.push(wire::KeyValue::string(attr::USER_ID, user.clone()));
    }
    if !scope.tags.is_empty() && profile.emit_tags && profile.emits_langfuse_attrs() {
        out.push(wire::KeyValue::string_array(attr::TRACE_TAGS, &scope.tags));
    }
    if let Some(env) = &scope.environment {
        if profile.emits_langfuse_attrs() {
            out.push(wire::KeyValue::string(attr::ENVIRONMENT, env.clone()));
        }
        out.push(wire::KeyValue::string(
            attr::DEPLOYMENT_ENVIRONMENT,
            env.clone(),
        ));
    }
    if !profile.emits_langfuse_attrs() {
        return;
    }
    if let Some(release) = &scope.release {
        out.push(wire::KeyValue::string(attr::RELEASE, release.clone()));
    }
    if let Some(version) = &scope.version {
        out.push(wire::KeyValue::string(attr::VERSION, version.clone()));
    }
}

/// Build the span representing a trace's root.
///
/// Langfuse reconstructs the trace from its spans, so the completed root is a
/// real AGENT observation and also carries the trace-level compatibility attrs.
#[must_use]
pub fn trace_to_span(trace: &TraceRecord, ctx: &ExportContext) -> wire::Span {
    let profile = &ctx.profile;
    let mut attributes = Vec::new();
    let error_message = trace
        .metadata
        .get("error")
        .and_then(serde_json::Value::as_str);

    if profile.emits_langfuse_attrs() {
        attributes.push(wire::KeyValue::string(attr::TRACE_NAME, trace.name.clone()));
        attributes.push(wire::KeyValue::string(attr::OBSERVATION_TYPE, "agent"));
        attributes.push(wire::KeyValue::string(
            attr::OBSERVATION_LEVEL,
            if error_message.is_some() {
                Level::Error.as_str()
            } else {
                Level::Default.as_str()
            },
        ));
        if let Some(message) = error_message {
            attributes.push(wire::KeyValue::string(
                attr::OBSERVATION_STATUS_MESSAGE,
                clamp(message.to_string(), profile.max_attribute_bytes),
            ));
        }
    } else {
        attributes.push(wire::KeyValue::string(attr::GEN_AI_OPERATION_NAME, "agent"));
    }
    push_scope_attrs(&mut attributes, &trace.scope, profile);

    if profile.emits_bodies() {
        if let Some(input) = &trace.input
            && let Some(kv) = json_attr(attr::TRACE_INPUT, input, profile.max_attribute_bytes)
        {
            attributes.push(kv.clone());
            if let Some(observation) =
                json_attr(attr::OBSERVATION_INPUT, input, profile.max_attribute_bytes)
            {
                attributes.push(observation);
            }
        }
        if let Some(output) = &trace.output
            && let Some(kv) = json_attr(attr::TRACE_OUTPUT, output, profile.max_attribute_bytes)
        {
            attributes.push(kv.clone());
            if let Some(observation) = json_attr(
                attr::OBSERVATION_OUTPUT,
                output,
                profile.max_attribute_bytes,
            ) {
                attributes.push(observation);
            }
        }
    } else if profile.emits_content_metadata() {
        // Size without content: enough to spot a runaway prompt in an APM
        // without copying the prompt itself into it.
        if let Some(input) = &trace.input {
            attributes.push(wire::KeyValue::int(
                "moltis.input.bytes",
                payload_size(input),
            ));
        }
        if let Some(output) = &trace.output {
            attributes.push(wire::KeyValue::int(
                "moltis.output.bytes",
                payload_size(output),
            ));
        }
    }

    if profile.emits_langfuse_attrs() {
        if !trace.metadata.is_empty() {
            let metadata = serde_json::to_value(&trace.metadata).unwrap_or_default();
            push_structured_attr(
                &mut attributes,
                attr::TRACE_METADATA,
                &metadata,
                profile.max_attribute_bytes,
            );
            push_structured_attr(
                &mut attributes,
                attr::OBSERVATION_METADATA,
                &metadata,
                profile.max_attribute_bytes,
            );
        }
        if trace.public {
            attributes.push(wire::KeyValue::bool(attr::TRACE_PUBLIC, true));
        }
    }

    let start = unix_nanos(trace.timestamp);
    wire::Span {
        trace_id: trace.id.as_otel_hex(),
        // Deriving the root span id from the trace id keeps it stable across
        // repeated trace updates, so the backend upserts instead of creating
        // a fresh root span each time.
        span_id: crate::model::ObservationId(trace.id.0.clone()).as_otel_hex(),
        parent_span_id: None,
        name: trace.name.clone(),
        kind: wire::SPAN_KIND_INTERNAL,
        end_time_unix_nano: unix_nanos(trace.end_time.unwrap_or(trace.timestamp)),
        start_time_unix_nano: start,
        attributes,
        status: error_message.map_or_else(wire::Status::unset, |message| {
            wire::Status::error(Some(message.to_string()))
        }),
    }
}

/// Build the span representing an observation.
#[must_use]
pub fn observation_to_span(obs: &ObservationRecord, ctx: &ExportContext) -> wire::Span {
    let profile = &ctx.profile;
    let mut attributes = Vec::new();

    if profile.emits_langfuse_attrs() {
        if let Some(trace_name) = &obs.trace_name {
            attributes.push(wire::KeyValue::string(attr::TRACE_NAME, trace_name.clone()));
        }
        if !obs.trace_metadata.is_empty() {
            let metadata = serde_json::to_value(&obs.trace_metadata).unwrap_or_default();
            push_structured_attr(
                &mut attributes,
                attr::TRACE_METADATA,
                &metadata,
                profile.max_attribute_bytes,
            );
        }
        attributes.push(wire::KeyValue::string(
            attr::OBSERVATION_TYPE,
            obs.kind.as_str(),
        ));
        attributes.push(wire::KeyValue::string(
            attr::OBSERVATION_LEVEL,
            obs.level.as_str(),
        ));
    } else {
        // Ops backends still need to group by step kind; a lowercase
        // operation name is the GenAI-conventional way to express it.
        attributes.push(wire::KeyValue::string(
            attr::GEN_AI_OPERATION_NAME,
            obs.kind.as_str(),
        ));
    }

    push_scope_attrs(&mut attributes, &obs.scope, profile);

    // Error detail is operational, not conversational: every backend gets it.
    if let Some(message) = &obs.status_message {
        let key = if profile.emits_langfuse_attrs() {
            attr::OBSERVATION_STATUS_MESSAGE
        } else {
            "error.message"
        };
        attributes.push(wire::KeyValue::string(
            key,
            clamp(message.clone(), profile.max_attribute_bytes),
        ));
    }

    push_payload_attrs(&mut attributes, obs, profile);
    push_model_attrs(&mut attributes, obs, profile);
    push_usage_attrs(&mut attributes, obs, profile);

    if profile.emits_langfuse_attrs() {
        push_prompt_attrs(&mut attributes, obs);
        if !obs.metadata.is_empty() {
            let metadata = serde_json::to_value(&obs.metadata).unwrap_or_default();
            push_structured_attr(
                &mut attributes,
                attr::OBSERVATION_METADATA,
                &metadata,
                profile.max_attribute_bytes,
            );
        }
    }

    if matches!(
        obs.kind,
        crate::model::ObservationKind::Tool | crate::model::ObservationKind::Retriever
    ) {
        attributes.push(wire::KeyValue::string(
            attr::GEN_AI_TOOL_NAME,
            obs.name.clone(),
        ));
    }

    let status = if obs.level == Level::Error {
        wire::Status::error(obs.status_message.clone())
    } else {
        wire::Status::unset()
    };

    wire::Span {
        trace_id: obs.trace_id.as_otel_hex(),
        span_id: obs.id.as_otel_hex(),
        parent_span_id: Some(
            obs.parent_id
                .as_ref()
                .map_or_else(
                    || crate::model::ObservationId(obs.trace_id.0.clone()),
                    Clone::clone,
                )
                .as_otel_hex(),
        ),
        name: obs.name.clone(),
        kind: wire::SPAN_KIND_INTERNAL,
        start_time_unix_nano: unix_nanos(obs.start_time),
        // A still-running observation reports its start as its end; the update
        // that arrives when it finishes carries the real end time.
        end_time_unix_nano: unix_nanos(obs.end_time.unwrap_or(obs.start_time)),
        attributes,
        status,
    }
}

/// Input and output payloads, or their sizes.
fn push_payload_attrs(
    out: &mut Vec<wire::KeyValue>,
    obs: &ObservationRecord,
    profile: &ExportProfile,
) {
    if profile.emits_bodies() {
        if let Some(input) = &obs.input
            && let Some(kv) = json_attr(attr::OBSERVATION_INPUT, input, profile.max_attribute_bytes)
        {
            out.push(kv);
        }
        if let Some(output) = &obs.output
            && let Some(kv) = json_attr(
                attr::OBSERVATION_OUTPUT,
                output,
                profile.max_attribute_bytes,
            )
        {
            out.push(kv);
        }
        return;
    }
    if !profile.emits_content_metadata() {
        return;
    }
    if let Some(input) = &obs.input {
        out.push(wire::KeyValue::int(
            "moltis.input.bytes",
            payload_size(input),
        ));
    }
    if let Some(output) = &obs.output {
        out.push(wire::KeyValue::int(
            "moltis.output.bytes",
            payload_size(output),
        ));
    }
}

/// Model identity and sampling parameters.
fn push_model_attrs(
    out: &mut Vec<wire::KeyValue>,
    obs: &ObservationRecord,
    profile: &ExportProfile,
) {
    let Some(model) = &obs.model else {
        return;
    };
    if profile.emits_langfuse_attrs() {
        out.push(wire::KeyValue::string(
            attr::OBSERVATION_MODEL,
            model.clone(),
        ));
    }
    // Model identity is low-cardinality and operationally essential, so it
    // goes to every backend.
    out.push(wire::KeyValue::string(
        attr::GEN_AI_REQUEST_MODEL,
        model.clone(),
    ));
    out.push(wire::KeyValue::string(
        attr::GEN_AI_RESPONSE_MODEL,
        model.clone(),
    ));

    if obs.model_parameters.is_empty() {
        return;
    }
    if profile.emits_langfuse_attrs() {
        let parameters = serde_json::to_value(&obs.model_parameters).unwrap_or_default();
        push_structured_attr(
            out,
            attr::OBSERVATION_MODEL_PARAMETERS,
            &parameters,
            profile.max_attribute_bytes,
        );
    }
    // Promote the two parameters GenAI consumers actually chart.
    if let Some(temp) = obs
        .model_parameters
        .get("temperature")
        .and_then(serde_json::Value::as_f64)
    {
        out.push(wire::KeyValue::double(
            attr::GEN_AI_REQUEST_TEMPERATURE,
            temp,
        ));
    }
    if let Some(max) = obs
        .model_parameters
        .get("max_tokens")
        .and_then(serde_json::Value::as_i64)
    {
        out.push(wire::KeyValue::int(attr::GEN_AI_REQUEST_MAX_TOKENS, max));
    }
}

/// Token usage and cost.
fn push_usage_attrs(
    out: &mut Vec<wire::KeyValue>,
    obs: &ObservationRecord,
    profile: &ExportProfile,
) {
    if !profile.emit_usage {
        return;
    }

    if let Some(completion_start) = obs.completion_start_time
        && profile.emits_langfuse_attrs()
    {
        // Langfuse parses this as an ISO-8601 instant, not a nanosecond count.
        if let Ok(text) = completion_start.format(&time::format_description::well_known::Rfc3339) {
            out.push(wire::KeyValue::string(
                attr::OBSERVATION_COMPLETION_START_TIME,
                text,
            ));
        }
    }

    if let Some(usage) = obs.usage.filter(|u| !u.is_empty()) {
        if profile.emits_langfuse_attrs() {
            // Only Langfuse prices the cache buckets; elsewhere this is an
            // opaque JSON blob nothing can chart.
            let details = usage.to_usage_details();
            let details = serde_json::to_value(details).unwrap_or_default();
            push_structured_attr(
                out,
                attr::OBSERVATION_USAGE_DETAILS,
                &details,
                profile.max_attribute_bytes,
            );
        }
        out.push(wire::KeyValue::int(
            attr::GEN_AI_USAGE_INPUT_TOKENS,
            i64::from(usage.input),
        ));
        out.push(wire::KeyValue::int(
            attr::GEN_AI_USAGE_OUTPUT_TOKENS,
            i64::from(usage.output),
        ));
    }

    // No cost attribute: Langfuse maintains versioned model definitions and
    // prices spend from the model plus these counts, and no other backend has
    // a price table to send one to.
}

/// Managed-prompt linkage. Langfuse-only: no other backend models it.
fn push_prompt_attrs(out: &mut Vec<wire::KeyValue>, obs: &ObservationRecord) {
    if let Some(name) = &obs.prompt_name {
        out.push(wire::KeyValue::string(
            attr::OBSERVATION_PROMPT_NAME,
            name.clone(),
        ));
    }
    if let Some(version) = obs.prompt_version {
        out.push(wire::KeyValue::int(
            attr::OBSERVATION_PROMPT_VERSION,
            i64::from(version),
        ));
    }
}

/// Resource attributes describing this Moltis instance.
#[must_use]
pub fn resource_attributes(ctx: &ExportContext) -> Vec<wire::KeyValue> {
    let mut attrs = vec![
        wire::KeyValue::string(attr::SERVICE_NAME, ctx.service_name.clone()),
        wire::KeyValue::string(attr::SERVICE_VERSION, ctx.service_version.clone()),
    ];
    if let Some(env) = &ctx.environment {
        attrs.push(wire::KeyValue::string(
            attr::DEPLOYMENT_ENVIRONMENT,
            env.clone(),
        ));
        if ctx.profile.emits_langfuse_attrs() {
            attrs.push(wire::KeyValue::string(attr::ENVIRONMENT, env.clone()));
        }
    }
    attrs
}

/// Convert a batch of events into an OTLP export request.
///
/// Score events carry no span representation and are skipped here; they are
/// delivered through the backend's scoring API instead.
#[must_use]
pub fn batch_to_request(events: &[Event], ctx: &ExportContext) -> wire::ExportTraceServiceRequest {
    let mut exported_ids = HashSet::new();
    let spans: Vec<wire::Span> = events
        .iter()
        .filter_map(|event| match event {
            Event::Trace(trace) if trace.end_time.is_some() => Some(trace_to_span(trace, ctx)),
            Event::ObservationEnd(obs) if obs.end_time.is_some() => {
                Some(observation_to_span(obs, ctx))
            },
            Event::Trace(_) | Event::ObservationEnd(_) => None,
            Event::Score(_) | Event::ScoreDelete(_) => None,
        })
        .filter(|span| exported_ids.insert((span.trace_id.clone(), span.span_id.clone())))
        .collect();

    wire::ExportTraceServiceRequest {
        resource_spans: vec![wire::ResourceSpans {
            resource: wire::Resource {
                attributes: resource_attributes(ctx),
            },
            scope_spans: vec![wire::ScopeSpans {
                scope: wire::InstrumentationScope {
                    name: SCOPE_NAME.to_string(),
                    version: ctx.service_version.clone(),
                },
                spans,
            }],
        }],
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use {
        super::*,
        crate::model::{
            ObservationId, ObservationKind, ScoreRecord, ScoreValue, TokenUsage, TraceId,
        },
    };

    fn ctx(profile: ExportProfile) -> ExportContext {
        ExportContext {
            service_name: "moltis".into(),
            service_version: "20260726.01".into(),
            environment: Some("production".into()),
            profile,
        }
    }

    fn langfuse_ctx() -> ExportContext {
        ctx(ExportProfile::langfuse())
    }

    fn otel_ctx() -> ExportContext {
        ctx(ExportProfile::otel_generic())
    }

    fn attr_value(span: &wire::Span, key: &str) -> Option<serde_json::Value> {
        let json = serde_json::to_value(span).ok()?;
        json["attributes"]
            .as_array()?
            .iter()
            .find(|kv| kv["key"] == key)
            .map(|kv| kv["value"].clone())
    }

    fn scope() -> TraceScope {
        TraceScope {
            session_id: Some("agent:main:main".into()),
            user_id: Some("telegram:42".into()),
            tags: vec!["telegram".into(), "agent:main".into()],
            environment: Some("production".into()),
            release: Some("20260726.01".into()),
            version: Some("preset-v3".into()),
        }
    }

    fn generation() -> ObservationRecord {
        let mut obs = ObservationRecord::start(
            TraceId::generate(),
            ObservationKind::Generation,
            "anthropic/claude",
        )
        .with_scope(scope());
        obs.model = Some("claude-opus-4".into());
        obs.usage = Some(TokenUsage::from_provider_totals(100, 50, 900, 20));
        obs.input = Some(json!({ "messages": [{ "role": "user", "content": "secret plan" }] }));
        obs.output = Some(json!("here is the answer"));
        obs.finish();
        obs
    }

    // ── Profile separation: the core of the Langfuse/APM split ──────────

    #[test]
    fn langfuse_receives_conversation_bodies() {
        let span = observation_to_span(&generation(), &langfuse_ctx());
        assert!(attr_value(&span, attr::OBSERVATION_INPUT).is_some());
        assert!(attr_value(&span, attr::OBSERVATION_OUTPUT).is_some());
    }

    #[test]
    fn generic_otel_never_receives_conversation_bodies() {
        // The guarantee that makes it safe to point this at a shared APM.
        let span = observation_to_span(&generation(), &otel_ctx());
        let rendered = serde_json::to_string(&span).expect("serializable");

        assert!(attr_value(&span, attr::OBSERVATION_INPUT).is_none());
        assert!(attr_value(&span, attr::OBSERVATION_OUTPUT).is_none());
        assert!(
            !rendered.contains("secret plan"),
            "prompt content leaked into an APM payload"
        );
        assert!(!rendered.contains("here is the answer"));
    }

    #[test]
    fn generic_otel_receives_payload_sizes_instead_of_payloads() {
        // Enough to alert on a runaway prompt without copying the prompt.
        let span = observation_to_span(&generation(), &otel_ctx());
        assert!(matches!(
            attr_value(&span, "moltis.input.bytes"),
            Some(v) if v["intValue"].is_string()
        ));
    }

    #[test]
    fn content_none_emits_neither_bodies_nor_sizes() {
        let profile = ExportProfile {
            content: crate::profile::ContentCapture::None,
            ..ExportProfile::otel_generic()
        };
        let span = observation_to_span(&generation(), &ctx(profile));

        assert!(attr_value(&span, attr::OBSERVATION_INPUT).is_none());
        assert!(attr_value(&span, "moltis.input.bytes").is_none());
    }

    #[test]
    fn generic_otel_omits_langfuse_vocabulary() {
        let span = observation_to_span(&generation(), &otel_ctx());
        let rendered = serde_json::to_string(&span).expect("serializable");
        assert!(
            !rendered.contains("langfuse."),
            "langfuse-specific attributes are noise in an APM"
        );
    }

    #[test]
    fn generic_otel_omits_user_and_session_ids() {
        let span = observation_to_span(&generation(), &otel_ctx());

        // Per-user cardinality is a billing and compliance problem in an APM.
        assert!(attr_value(&span, attr::USER_ID).is_none());
        // Channel session keys contain account and peer identifiers and are
        // therefore no safer than the explicit user id.
        assert!(attr_value(&span, attr::GEN_AI_CONVERSATION_ID).is_none());
    }

    #[test]
    fn datadog_profile_drops_tags() {
        let span = observation_to_span(&generation(), &ctx(ExportProfile::datadog()));
        assert!(attr_value(&span, attr::TRACE_TAGS).is_none());
    }

    #[test]
    fn generic_otel_expresses_step_kind_as_gen_ai_operation() {
        let mut obs = generation();
        obs.kind = ObservationKind::Retriever;
        let span = observation_to_span(&obs, &otel_ctx());

        assert_eq!(
            attr_value(&span, attr::GEN_AI_OPERATION_NAME),
            Some(json!({ "stringValue": "retriever" }))
        );
        assert!(attr_value(&span, attr::OBSERVATION_TYPE).is_none());
    }

    #[test]
    fn model_and_error_reach_every_backend() {
        // Operationally essential and low-cardinality, so never gated.
        let mut obs = generation();
        obs.fail("provider returned 500");

        for context in [langfuse_ctx(), otel_ctx(), ctx(ExportProfile::datadog())] {
            let span = observation_to_span(&obs, &context);
            assert!(attr_value(&span, attr::GEN_AI_REQUEST_MODEL).is_some());
            assert_eq!(span.status.code, wire::STATUS_ERROR);
        }
    }

    #[test]
    fn token_counts_reach_every_backend_but_usage_details_do_not() {
        let obs = generation();

        let otel = observation_to_span(&obs, &otel_ctx());
        assert!(attr_value(&otel, attr::GEN_AI_USAGE_INPUT_TOKENS).is_some());
        // The cache-aware breakdown is a Langfuse concept; an APM gets counts.
        assert!(attr_value(&otel, attr::OBSERVATION_USAGE_DETAILS).is_none());

        let lf = observation_to_span(&obs, &langfuse_ctx());
        assert!(attr_value(&lf, attr::GEN_AI_USAGE_INPUT_TOKENS).is_some());
        assert!(attr_value(&lf, attr::OBSERVATION_USAGE_DETAILS).is_some());
    }

    #[test]
    fn usage_can_be_disabled_entirely() {
        let profile = ExportProfile {
            emit_usage: false,
            ..ExportProfile::otel_generic()
        };
        let span = observation_to_span(&generation(), &ctx(profile));
        assert!(attr_value(&span, attr::GEN_AI_USAGE_INPUT_TOKENS).is_none());
    }

    #[test]
    fn prompt_linkage_is_langfuse_only() {
        let mut obs = generation();
        obs.prompt_name = Some("system-prompt".into());
        obs.prompt_version = Some(7);

        assert!(
            attr_value(
                &observation_to_span(&obs, &langfuse_ctx()),
                attr::OBSERVATION_PROMPT_NAME
            )
            .is_some()
        );
        assert!(
            attr_value(
                &observation_to_span(&obs, &otel_ctx()),
                attr::OBSERVATION_PROMPT_NAME
            )
            .is_none()
        );
    }

    #[test]
    fn trace_bodies_are_withheld_from_an_apm() {
        let mut trace = TraceRecord::new("turn");
        trace.input = Some(json!("private question"));
        trace.output = Some(json!("private answer"));

        let rendered =
            serde_json::to_string(&trace_to_span(&trace, &otel_ctx())).expect("serializable");
        assert!(!rendered.contains("private question"));
        assert!(!rendered.contains("private answer"));
    }

    #[test]
    fn attribute_values_are_clamped_to_the_profile_ceiling() {
        let mut obs = generation();
        obs.output = Some(json!("x".repeat(100_000)));
        let profile = ExportProfile {
            max_attribute_bytes: 100,
            ..ExportProfile::langfuse()
        };
        let span = observation_to_span(&obs, &ctx(profile));
        let value = attr_value(&span, attr::OBSERVATION_OUTPUT)
            .and_then(|v| v["stringValue"].as_str().map(str::to_string))
            .expect("output present");

        assert!(
            value.len() <= 110,
            "clamped value was {} bytes",
            value.len()
        );
    }

    #[test]
    fn clamping_does_not_split_multibyte_characters() {
        // Slicing blindly at a byte offset would panic on this input.
        let clamped = clamp("ααααααααα".to_string(), 5);
        assert!(clamped.ends_with("..."));
        assert!(clamped.len() <= 5);
    }

    // ── Shared structural behaviour ─────────────────────────────────────

    #[test]
    fn observation_type_uses_langfuse_taxonomy() {
        let mut obs = generation();
        obs.kind = ObservationKind::Tool;
        let span = observation_to_span(&obs, &langfuse_ctx());

        assert_eq!(
            attr_value(&span, attr::OBSERVATION_TYPE),
            Some(json!({ "stringValue": "tool" }))
        );
    }

    #[test]
    fn tool_spans_carry_gen_ai_tool_name_on_every_backend() {
        let mut obs = generation();
        obs.kind = ObservationKind::Tool;
        obs.name = "exec".into();

        for context in [langfuse_ctx(), otel_ctx()] {
            assert_eq!(
                attr_value(&observation_to_span(&obs, &context), attr::GEN_AI_TOOL_NAME),
                Some(json!({ "stringValue": "exec" }))
            );
        }
    }

    #[test]
    fn scope_attributes_ride_on_every_span() {
        // OTLP has no trace-level attribute concept, so a backend filtering by
        // session sees only what each span itself carries.
        let span = observation_to_span(&generation(), &langfuse_ctx());

        assert_eq!(
            attr_value(&span, attr::SESSION_ID),
            Some(json!({ "stringValue": "agent:main:main" }))
        );
        assert_eq!(
            attr_value(&span, attr::USER_ID),
            Some(json!({ "stringValue": "telegram:42" }))
        );
        assert_eq!(
            attr_value(&span, attr::RELEASE),
            Some(json!({ "stringValue": "20260726.01" }))
        );
    }

    #[test]
    fn usage_details_carry_cache_buckets_separately() {
        let span = observation_to_span(&generation(), &langfuse_ctx());
        let raw = attr_value(&span, attr::OBSERVATION_USAGE_DETAILS)
            .and_then(|v| v["stringValue"].as_str().map(str::to_string))
            .expect("usage details present");
        let parsed: serde_json::Value = serde_json::from_str(&raw).expect("valid json");

        assert_eq!(parsed["input"], 100);
        assert_eq!(parsed["input_cache_read"], 900);
        assert_eq!(parsed["input_cache_write"], 20);
    }

    #[test]
    fn empty_usage_is_omitted_rather_than_reported_as_zero() {
        let mut obs = generation();
        obs.usage = Some(TokenUsage::default());
        let span = observation_to_span(&obs, &langfuse_ctx());

        // A zero-token generation distorts the backend's cost charts.
        assert!(attr_value(&span, attr::OBSERVATION_USAGE_DETAILS).is_none());
        assert!(attr_value(&span, attr::GEN_AI_USAGE_INPUT_TOKENS).is_none());
    }

    #[test]
    fn error_level_sets_otel_status_code_two() {
        let mut obs = generation();
        obs.fail("provider returned 500");
        let span = observation_to_span(&obs, &langfuse_ctx());

        assert_eq!(span.status.code, wire::STATUS_ERROR);
        assert_eq!(
            span.status.message.as_deref(),
            Some("provider returned 500")
        );
    }

    #[test]
    fn apm_gets_error_message_under_the_conventional_key() {
        let mut obs = generation();
        obs.fail("provider returned 500");
        let span = observation_to_span(&obs, &otel_ctx());

        assert_eq!(
            attr_value(&span, "error.message"),
            Some(json!({ "stringValue": "provider returned 500" }))
        );
    }

    #[test]
    fn non_error_levels_leave_status_unset() {
        let span = observation_to_span(&generation(), &langfuse_ctx());
        assert_eq!(span.status.code, wire::STATUS_UNSET);
    }

    #[test]
    fn orphan_observations_are_parented_to_the_trace_root() {
        // Otherwise a top-level observation becomes a second root span and the
        // backend renders two disconnected traces.
        let obs = generation();
        let span = observation_to_span(&obs, &langfuse_ctx());
        let expected = ObservationId(obs.trace_id.0.clone()).as_otel_hex();

        assert_eq!(span.parent_span_id, Some(expected));
    }

    #[test]
    fn nested_observations_point_at_their_parent() {
        let parent = generation();
        let child =
            ObservationRecord::start(parent.trace_id.clone(), ObservationKind::Tool, "exec")
                .with_parent(Some(parent.id.clone()));

        let span = observation_to_span(&child, &langfuse_ctx());
        assert_eq!(span.parent_span_id, Some(parent.id.as_otel_hex()));
    }

    #[test]
    fn trace_root_span_id_is_derived_from_the_trace_id() {
        // A fresh random root id per update would create duplicate roots.
        let trace = TraceRecord::new("turn");
        let first = trace_to_span(&trace, &langfuse_ctx());
        let second = trace_to_span(&trace, &langfuse_ctx());

        assert_eq!(first.span_id, second.span_id);
        assert!(first.parent_span_id.is_none());
    }

    #[test]
    fn completed_trace_is_a_duration_bearing_agent_observation() {
        let mut trace = TraceRecord::new("turn");
        trace.timestamp = OffsetDateTime::UNIX_EPOCH;
        trace.end_time = Some(OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(2));
        trace.input = Some(json!({ "role": "user", "content": "hello" }));
        trace.output = Some(json!({ "role": "assistant", "content": "hi" }));
        let span = trace_to_span(&trace, &langfuse_ctx());

        assert_eq!(span.start_time_unix_nano, "0");
        assert_eq!(span.end_time_unix_nano, "2000000000");
        assert_eq!(
            attr_value(&span, attr::OBSERVATION_TYPE),
            Some(json!({ "stringValue": "agent" }))
        );
        assert!(attr_value(&span, attr::OBSERVATION_INPUT).is_some());
        assert!(attr_value(&span, attr::OBSERVATION_OUTPUT).is_some());
        // Trace attrs are retained because Langfuse builds its trace view from
        // them independently of the root observation view.
        assert!(attr_value(&span, attr::TRACE_INPUT).is_some());
        assert!(attr_value(&span, attr::TRACE_OUTPUT).is_some());
    }

    #[test]
    fn child_spans_carry_filterable_trace_context() {
        let mut obs = generation();
        obs.trace_name = Some("support-agent".into());
        obs.trace_metadata.insert("tenant".into(), json!("acme"));
        let span = observation_to_span(&obs, &langfuse_ctx());

        assert_eq!(
            attr_value(&span, attr::TRACE_NAME),
            Some(json!({ "stringValue": "support-agent" }))
        );
        let metadata_attr = attr_value(&span, attr::TRACE_METADATA).expect("trace metadata");
        let metadata = metadata_attr["stringValue"]
            .as_str()
            .map(|value| serde_json::from_str::<serde_json::Value>(value).expect("valid json"))
            .expect("trace metadata string");
        assert_eq!(metadata["tenant"], "acme");
    }

    #[test]
    fn string_inputs_are_not_double_encoded() {
        let mut trace = TraceRecord::new("turn");
        trace.input = Some(json!("what is the weather"));
        let span = trace_to_span(&trace, &langfuse_ctx());

        // Wrapping a plain string in JSON quotes renders as `"\"text\""` in the
        // backend UI, on every single trace.
        assert_eq!(
            attr_value(&span, attr::TRACE_INPUT),
            Some(json!({ "stringValue": "what is the weather" }))
        );
    }

    #[test]
    fn structured_inputs_are_json_encoded() {
        let mut trace = TraceRecord::new("turn");
        trace.input = Some(json!({ "role": "user" }));
        let span = trace_to_span(&trace, &langfuse_ctx());

        assert_eq!(
            attr_value(&span, attr::TRACE_INPUT),
            Some(json!({ "stringValue": "{\"role\":\"user\"}" }))
        );
    }

    #[test]
    fn null_payloads_are_dropped() {
        let mut trace = TraceRecord::new("turn");
        trace.input = Some(serde_json::Value::Null);
        let span = trace_to_span(&trace, &langfuse_ctx());
        assert!(attr_value(&span, attr::TRACE_INPUT).is_none());
    }

    #[test]
    fn completion_start_time_is_rfc3339_not_nanos() {
        let mut obs = generation();
        obs.completion_start_time = Some(OffsetDateTime::UNIX_EPOCH);
        let span = observation_to_span(&obs, &langfuse_ctx());

        assert_eq!(
            attr_value(&span, attr::OBSERVATION_COMPLETION_START_TIME),
            Some(json!({ "stringValue": "1970-01-01T00:00:00Z" }))
        );
    }

    #[test]
    fn running_observations_report_end_equal_to_start() {
        let mut obs = generation();
        obs.end_time = None;
        let span = observation_to_span(&obs, &langfuse_ctx());
        assert_eq!(span.start_time_unix_nano, span.end_time_unix_nano);
    }

    #[test]
    fn model_parameters_promote_temperature_and_max_tokens() {
        let mut obs = generation();
        obs.model_parameters
            .insert("temperature".into(), json!(0.7));
        obs.model_parameters
            .insert("max_tokens".into(), json!(4096));
        let span = observation_to_span(&obs, &langfuse_ctx());

        assert_eq!(
            attr_value(&span, attr::GEN_AI_REQUEST_TEMPERATURE),
            Some(json!({ "doubleValue": 0.7 }))
        );
        assert_eq!(
            attr_value(&span, attr::GEN_AI_REQUEST_MAX_TOKENS),
            Some(json!({ "intValue": "4096" }))
        );
    }

    #[test]
    fn in_progress_spans_are_dropped_for_all_immutable_span_backends() {
        let mut obs = generation();
        obs.end_time = None;
        let events = vec![Event::ObservationEnd(Box::new(obs))];

        let lf = batch_to_request(&events, &langfuse_ctx());
        assert!(lf.resource_spans[0].scope_spans[0].spans.is_empty());

        // Tempo and Datadog would render this and the later completion as two
        // separate spans with the same id.
        let otel = batch_to_request(&events, &otel_ctx());
        assert!(otel.resource_spans[0].scope_spans[0].spans.is_empty());
    }

    #[test]
    fn completed_spans_reach_every_backend() {
        let events = vec![Event::ObservationEnd(Box::new(generation()))];
        for context in [langfuse_ctx(), otel_ctx()] {
            let request = batch_to_request(&events, &context);
            assert_eq!(request.resource_spans[0].scope_spans[0].spans.len(), 1);
        }
    }

    #[test]
    fn scores_produce_no_spans() {
        let events = vec![Event::Score(Box::new(ScoreRecord::new(
            TraceId::generate(),
            "user-feedback",
            ScoreValue::Numeric(1.0),
        )))];
        let request = batch_to_request(&events, &langfuse_ctx());

        assert!(request.resource_spans[0].scope_spans[0].spans.is_empty());
    }

    #[test]
    fn batch_carries_resource_and_scope_identity() {
        let mut trace = TraceRecord::new("turn");
        trace.finish();
        let events = vec![Event::Trace(Box::new(trace))];
        let request = batch_to_request(&events, &langfuse_ctx());
        let json = serde_json::to_value(&request).expect("serializable");

        assert_eq!(
            json["resourceSpans"][0]["scopeSpans"][0]["scope"]["name"],
            SCOPE_NAME
        );
        let names: Vec<&str> = json["resourceSpans"][0]["resource"]["attributes"]
            .as_array()
            .map(|a| a.iter().filter_map(|kv| kv["key"].as_str()).collect())
            .unwrap_or_default();
        assert!(names.contains(&attr::SERVICE_NAME));
        assert!(names.contains(&attr::DEPLOYMENT_ENVIRONMENT));
    }

    #[test]
    fn pre_epoch_timestamps_clamp_to_zero() {
        let mut trace = TraceRecord::new("turn");
        trace.timestamp = OffsetDateTime::UNIX_EPOCH - time::Duration::days(1);
        let span = trace_to_span(&trace, &langfuse_ctx());

        // OTLP treats the field as unsigned; a negative value wraps to a
        // far-future timestamp in collectors.
        assert_eq!(span.start_time_unix_nano, "0");
    }

    #[test]
    fn incomplete_roots_and_duplicate_completed_ids_are_filtered() {
        let incomplete = TraceRecord::new("turn");
        let mut complete = incomplete.clone();
        complete.finish();
        let observation = generation();
        let events = vec![
            Event::Trace(Box::new(incomplete)),
            Event::Trace(Box::new(complete.clone())),
            Event::Trace(Box::new(complete)),
            Event::ObservationEnd(Box::new(observation.clone())),
            Event::ObservationEnd(Box::new(observation)),
        ];
        let request = batch_to_request(&events, &langfuse_ctx());
        let spans = &request.resource_spans[0].scope_spans[0].spans;
        let ids: HashSet<_> = spans.iter().map(|span| span.span_id.as_str()).collect();

        assert_eq!(spans.len(), 2);
        assert_eq!(ids.len(), spans.len(), "each wire id must occur once");
    }

    #[test]
    fn oversized_structured_attributes_remain_valid_json_within_the_limit() {
        let mut obs = generation();
        obs.output = Some(json!({ "items": ["x".repeat(500), "y".repeat(500)] }));
        obs.metadata.insert("large".into(), json!("z".repeat(500)));
        let profile = ExportProfile {
            max_attribute_bytes: 80,
            ..ExportProfile::langfuse()
        };
        let span = observation_to_span(&obs, &ctx(profile));

        for key in [attr::OBSERVATION_OUTPUT, attr::OBSERVATION_METADATA] {
            let attribute = attr_value(&span, key).expect("attribute present");
            let value = attribute["stringValue"].as_str().expect("string value");
            assert!(value.len() <= 80);
            serde_json::from_str::<serde_json::Value>(value).expect("valid bounded json");
        }
    }
}
