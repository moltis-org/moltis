#![allow(clippy::unwrap_used, clippy::expect_used)]
// Every case here drives the OTLP transport, which a build without that feature
// does not have. The Langfuse cases use the Langfuse *profile* over the same
// transport, so `otlp` is the only feature they need.
#![cfg(feature = "otlp")]
//! End-to-end tests over the real export path.
//!
//! The unit tests check the mapping in isolation; these drive a recorder
//! through a live `BatchSink` into an HTTP server and assert on the bytes that
//! actually leave the process. The content-separation guarantee is only worth
//! anything if it holds on the wire, not just in a mapping function.

use std::{sync::Arc, time::Duration};

use {
    moltis_observability::{
        BatchConfig, BatchSink, ExportProfile, ObservationKind, ObservationSink, RecorderSettings,
        TokenUsage, TraceScope, TurnRecorder,
        exporters::otlp::{OtlpConfig, OtlpTransport},
    },
    serde_json::Value,
    wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    },
};

/// The prompt text used throughout, so leak assertions are unambiguous.
const SECRET_PROMPT: &str = "please deploy the production database migration";
const SECRET_ANSWER: &str = "migration applied to prod-db-01";

fn scope() -> TraceScope {
    TraceScope {
        session_id: Some("agent:main:channel:telegram:account:private-bot:peer:user:99".into()),
        user_id: Some("telegram:99".into()),
        tags: vec!["channel:telegram".into()],
        environment: Some("production".into()),
        release: Some("20260726.01".into()),
        version: None,
    }
}

/// Spin up a mock collector and return it plus the captured request bodies.
async fn collector() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/traces"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    server
}

/// Every OTLP body the mock server received, parsed.
async fn captured_bodies(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter_map(|req| serde_json::from_slice::<Value>(&req.body).ok())
        .collect()
}

/// Install a sink pointed at `server` with `profile`, run one scripted agent
/// turn through the recorder, then flush and return the raw request bodies.
async fn run_turn_against(server: &MockServer, profile: ExportProfile) -> Vec<Value> {
    let transport = Arc::new(OtlpTransport::new(OtlpConfig {
        name: "test".into(),
        endpoint: format!("{}/v1/traces", server.uri()),
        headers: Default::default(),
        timeout: Duration::from_secs(5),
        service_name: "moltis".into(),
        service_version: "20260726.01".into(),
        environment: Some("production".into()),
        profile,
    }));
    let batch_sink = Arc::new(BatchSink::spawn(transport, BatchConfig {
        flush_interval: Duration::from_millis(20),
        ..Default::default()
    }));

    {
        // Deliberately not the global sink: these tests run in parallel in one
        // process, and a shared global means one test's spans land in another
        // test's collector.
        let recorder = TurnRecorder::begin_with_sink(
            batch_sink.clone(),
            "agent-run",
            scope(),
            RecorderSettings::default(),
        )
        .expect("recorder should start");
        recorder.set_input(Value::String(SECRET_PROMPT.into()));
        recorder.set_metadata("tenant", serde_json::json!("acme"));

        let mut generation = recorder.step(ObservationKind::Generation, "anthropic/claude-opus-4");
        generation.set_model("claude-opus-4");
        generation.set_input(serde_json::json!([{ "role": "user", "content": SECRET_PROMPT }]));
        generation.mark_first_token();
        generation.set_usage(TokenUsage::from_provider_totals(120, 40, 800, 15));
        generation.set_output(Value::String(SECRET_ANSWER.into()));
        let generation_id = generation.id();
        generation.finish();

        let mut tool = recorder.step_under(ObservationKind::Tool, "exec", generation_id);
        tool.set_input(serde_json::json!({ "cmd": "psql", "api_key": "sk-live-abcdefghijkl" }));
        tool.set_output(Value::String(SECRET_ANSWER.into()));
        tool.finish();

        recorder.set_output(Value::String(SECRET_ANSWER.into()));
        recorder.finish();
    }

    batch_sink
        .flush(Duration::from_secs(5))
        .await
        .expect("flush should succeed");

    captured_bodies(server).await
}

/// Every span across every captured request.
fn all_spans(bodies: &[Value]) -> Vec<&Value> {
    bodies
        .iter()
        .filter_map(|b| b["resourceSpans"].as_array())
        .flatten()
        .filter_map(|rs| rs["scopeSpans"].as_array())
        .flatten()
        .filter_map(|ss| ss["spans"].as_array())
        .flatten()
        .collect()
}

fn span_attr<'a>(span: &'a Value, key: &str) -> Option<&'a Value> {
    span["attributes"]
        .as_array()?
        .iter()
        .find(|kv| kv["key"] == key)
        .map(|kv| &kv["value"])
}

#[tokio::test]
async fn langfuse_profile_delivers_the_full_conversation() {
    let server = collector().await;
    let bodies = run_turn_against(&server, ExportProfile::langfuse()).await;

    assert!(!bodies.is_empty(), "expected at least one export request");
    let raw = serde_json::to_string(&bodies).expect("serializable");

    // Langfuse is useless without the conversation, so it must be present.
    assert!(
        raw.contains(SECRET_PROMPT),
        "prompt missing from Langfuse export"
    );
    assert!(
        raw.contains(SECRET_ANSWER),
        "completion missing from Langfuse export"
    );

    // Immutable OTLP receives only the completed generation.
    let spans = all_spans(&bodies);
    let generation = spans
        .iter()
        .find(|s| span_attr(s, "langfuse.observation.model.name").is_some())
        .expect("completed generation span present");

    assert_eq!(
        span_attr(generation, "langfuse.observation.model.name"),
        Some(&serde_json::json!({ "stringValue": "claude-opus-4" }))
    );
    assert!(span_attr(generation, "langfuse.observation.completion_start_time").is_some());

    let root = spans
        .iter()
        .find(|span| span.get("parentSpanId").is_none())
        .expect("root span present");
    assert_eq!(
        span_attr(root, "langfuse.observation.type"),
        Some(&serde_json::json!({ "stringValue": "agent" }))
    );
    assert_ne!(root["startTimeUnixNano"], root["endTimeUnixNano"]);
    assert!(span_attr(root, "langfuse.observation.input").is_some());
    assert!(span_attr(root, "langfuse.observation.output").is_some());
}

#[tokio::test]
async fn generic_otlp_profile_never_puts_conversation_on_the_wire() {
    let server = collector().await;
    let bodies = run_turn_against(&server, ExportProfile::otel_generic()).await;

    assert!(!bodies.is_empty(), "expected at least one export request");
    let raw = serde_json::to_string(&bodies).expect("serializable");

    // The guarantee that makes it safe to point this at a shared APM.
    assert!(
        !raw.contains(SECRET_PROMPT),
        "prompt text reached a generic OTLP backend"
    );
    assert!(
        !raw.contains(SECRET_ANSWER),
        "completion text reached a generic OTLP backend"
    );
    assert!(
        !raw.contains("langfuse."),
        "langfuse-specific attributes reached a generic OTLP backend"
    );
    // Per-user cardinality is a billing and compliance problem in an APM.
    assert!(!raw.contains("telegram:99"), "user id reached an APM");
    assert!(
        !raw.contains("private-bot"),
        "channel account id reached an APM"
    );
    assert!(
        !raw.contains("gen_ai.conversation.id"),
        "channel session key reached an APM"
    );
}

#[tokio::test]
async fn generic_otlp_profile_still_delivers_operational_signal() {
    let server = collector().await;
    let bodies = run_turn_against(&server, ExportProfile::otel_generic()).await;
    let raw = serde_json::to_string(&bodies).expect("serializable");

    // Withholding content must not mean withholding everything: latency,
    // model and token counts are the whole point of the APM export.
    assert!(raw.contains("claude-opus-4"), "model missing");
    assert!(
        raw.contains("gen_ai.usage.input_tokens"),
        "token counts missing"
    );
    assert!(raw.contains("moltis.input.bytes"), "payload sizes missing");
    assert!(!raw.contains("gen_ai.conversation.id"));
}

#[tokio::test]
async fn tool_credentials_are_redacted_even_for_langfuse() {
    let server = collector().await;
    let bodies = run_turn_against(&server, ExportProfile::langfuse()).await;
    let raw = serde_json::to_string(&bodies).expect("serializable");

    // The tool argument carried both a benign key and an API key. Redaction
    // runs before serialization, so the credential must never reach the wire
    // even on the backend that receives everything else.
    assert!(
        !raw.contains("sk-live-abcdefghijkl"),
        "tool credential leaked into the export"
    );
    assert!(raw.contains("REDACTED"), "expected a redaction marker");
    assert!(raw.contains("psql"), "benign tool argument should survive");
}

#[tokio::test]
async fn the_span_tree_is_correctly_nested() {
    let server = collector().await;
    let bodies = run_turn_against(&server, ExportProfile::langfuse()).await;
    let spans = all_spans(&bodies);

    let trace_ids: std::collections::HashSet<&str> =
        spans.iter().filter_map(|s| s["traceId"].as_str()).collect();
    assert_eq!(
        trace_ids.len(),
        1,
        "one turn must produce exactly one trace"
    );

    let root = spans
        .iter()
        .find(|s| s.get("parentSpanId").is_none())
        .expect("a root span must exist");
    let root_id = root["spanId"].as_str().expect("root has an id");

    // The generation hangs off the root, and the tool hangs off the
    // generation. A flat tree would lose which call issued which tool.
    let generation = spans
        .iter()
        .find(|s| s["name"] == "anthropic/claude-opus-4")
        .expect("generation span present");
    assert_eq!(generation["parentSpanId"].as_str(), Some(root_id));

    let tool = spans
        .iter()
        .find(|s| s["name"] == "exec")
        .expect("tool span present");
    assert_eq!(
        tool["parentSpanId"].as_str(),
        generation["spanId"].as_str(),
        "tool must nest under the generation that called it"
    );
}

#[tokio::test]
async fn every_completed_wire_id_is_exported_once() {
    let server = collector().await;
    let bodies = run_turn_against(&server, ExportProfile::langfuse()).await;
    let spans = all_spans(&bodies);
    let ids: std::collections::HashSet<(&str, &str)> = spans
        .iter()
        .filter_map(|span| Some((span["traceId"].as_str()?, span["spanId"].as_str()?)))
        .collect();

    assert_eq!(spans.len(), 3, "root, generation, and tool expected");
    assert_eq!(ids.len(), spans.len(), "duplicate OTLP wire span ID");

    for span in &spans {
        assert_eq!(
            span_attr(span, "langfuse.trace.name"),
            Some(&serde_json::json!({ "stringValue": "agent-run" }))
        );
        let metadata = span_attr(span, "langfuse.trace.metadata")
            .and_then(|value| value["stringValue"].as_str())
            .map(|value| serde_json::from_str::<Value>(value).expect("valid metadata"))
            .expect("filterable trace metadata");
        assert_eq!(metadata["tenant"], "acme");
    }
}

#[tokio::test]
async fn cache_tokens_survive_the_round_trip_unsummed() {
    let server = collector().await;
    let bodies = run_turn_against(&server, ExportProfile::langfuse()).await;
    let spans = all_spans(&bodies);

    let usage = spans
        .iter()
        .find_map(|s| span_attr(s, "langfuse.observation.usage_details"))
        .and_then(|v| v["stringValue"].as_str())
        .map(|s| serde_json::from_str::<Value>(s).expect("valid json"))
        .expect("usage details present on the wire");

    // Folding cache reads into `input` would inflate the priced fresh-input
    // count by nearly an order of magnitude on a cache-heavy run.
    assert_eq!(usage["input"], 120);
    assert_eq!(usage["input_cache_read"], 800);
    assert_eq!(usage["input_cache_write"], 15);
    assert_eq!(usage["total"], 975);
}

#[tokio::test]
async fn an_unreachable_backend_does_not_stall_the_turn() {
    // Port 1 is reserved and refuses connections immediately.
    let transport = Arc::new(OtlpTransport::new(OtlpConfig {
        name: "dead".into(),
        endpoint: "http://127.0.0.1:1/v1/traces".into(),
        timeout: Duration::from_millis(50),
        profile: ExportProfile::langfuse(),
        ..Default::default()
    }));
    let batch_sink = Arc::new(BatchSink::spawn(transport, BatchConfig {
        max_retries: 1,
        initial_backoff: Duration::from_millis(1),
        max_backoff: Duration::from_millis(2),
        flush_interval: Duration::from_millis(20),
        ..Default::default()
    }));
    let started = std::time::Instant::now();
    let recorder = TurnRecorder::begin_with_sink(
        batch_sink.clone(),
        "agent-run",
        scope(),
        RecorderSettings::default(),
    )
    .expect("recorder should start");
    for _ in 0..200 {
        recorder.step(ObservationKind::Tool, "exec").finish();
    }
    recorder.finish();
    let elapsed = started.elapsed();

    // Recording is a queue push; a dead backend must not be felt by the agent.
    assert!(
        elapsed < Duration::from_secs(1),
        "recording blocked for {elapsed:?} against a dead backend"
    );
}
