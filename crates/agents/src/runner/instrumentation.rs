//! Bridge between the agent loop and `moltis-observability`.
//!
//! Keeps trace-scope derivation out of the runner bodies: the loops call
//! [`begin_turn`] once and then open steps. Every function here degrades to a
//! no-op when instrumentation is disabled, so the runner needs no `cfg` gates.

use {
    moltis_common::hooks::ChannelBinding,
    moltis_observability::{
        ObservationKind, RecorderSettings, TokenUsage, TraceScope, TurnRecorder,
    },
};

use crate::{
    model::{
        AgentToolControls, ChatMessage, ContentPart, LlmProvider, ProviderIdentity, Usage,
        UserContent,
    },
    runner::{AgentRunError, AgentRunResult, tool_result::persisted_tool_result_failure},
};

/// Derive the trace scope from the session and channel context.
///
/// The session key becomes the backend's session id, so a Langfuse session
/// view lines up one-to-one with a Moltis conversation. Channel provenance
/// becomes tags, and the sender becomes the user id — namespaced by channel so
/// `telegram:42` and `slack:42` are never conflated into one person.
#[must_use]
pub fn trace_scope(
    session_key: &str,
    channel: Option<&ChannelBinding>,
    environment: String,
    release: String,
) -> TraceScope {
    let mut tags = Vec::new();
    let mut user_id = None;

    if let Some(binding) = channel {
        if let Some(channel_type) = &binding.channel_type {
            tags.push(format!("channel:{channel_type}"));
        }
        if let Some(surface) = &binding.surface {
            tags.push(format!("surface:{surface}"));
        }
        if let Some(chat_type) = &binding.chat_type {
            tags.push(format!("chat:{chat_type}"));
        }
        if let Some(sender) = &binding.sender_id {
            user_id = Some(binding.channel_type.as_ref().map_or_else(
                || sender.clone(),
                |channel_type| format!("{channel_type}:{sender}"),
            ));
        }
    }

    // The agent id is the leading segment of `agent:<id>:...`.
    if let Some(agent_id) = session_key
        .strip_prefix("agent:")
        .and_then(|rest| rest.split(':').next())
        && !agent_id.is_empty()
    {
        tags.push(format!("agent:{agent_id}"));
    }

    TraceScope {
        session_id: (!session_key.is_empty()).then(|| session_key.to_string()),
        user_id,
        tags,
        environment: Some(environment),
        release: Some(release),
        version: None,
    }
}

/// Recorder settings derived from `[instrumentation]`.
///
/// Capture switches come from the Langfuse sub-table because it is the only
/// backend that receives payload bodies at all; the APM profiles gate content
/// independently through their own `content` mode.
#[must_use]
pub fn recorder_settings(config: &moltis_config::InstrumentationConfig) -> RecorderSettings {
    RecorderSettings {
        redaction: moltis_observability::RedactionPolicy::from_needles(&config.redact),
        capture_input: config.langfuse.capture_input,
        capture_output: config.langfuse.capture_output,
        capture_tool_io: config.langfuse.capture_tool_io,
        sample_rate: config.sample_rate.clamp(0.0, 1.0),
    }
}

/// Release identifier reported to backends, defaulting to the running version.
#[must_use]
pub fn release(config: &moltis_config::InstrumentationConfig) -> String {
    config
        .release
        .clone()
        .unwrap_or_else(|| moltis_config::VERSION.to_string())
}

/// Begin recording an agent run, if instrumentation is enabled.
///
/// Returns `None` when no sink is installed or the turn was not sampled, in
/// which case every downstream call is skipped by the caller's `Option` checks.
#[must_use]
pub fn begin_turn(
    session_key: &str,
    trace_correlation_key: Option<&str>,
    channel: Option<&ChannelBinding>,
    provider: &str,
    model: &str,
    user_content: &UserContent,
    settings: RecorderSettings,
    environment: String,
    release: String,
) -> Option<TurnRecorder> {
    let scope = trace_scope(session_key, channel, environment, release);
    let recorder = TurnRecorder::begin("agent-run", scope, settings)?;

    // Only top-level chat runs provide a correlation key. Sub-agents deliberately
    // omit it, so they cannot overwrite the trace used to attribute the parent
    // run's delivered reply.
    if let Some(key) = trace_correlation_key {
        moltis_observability::remember_trace(key, recorder.trace_id());
    }

    recorder.set_metadata("provider", serde_json::Value::String(provider.to_string()));
    recorder.set_metadata("model", serde_json::Value::String(model.to_string()));
    recorder.set_input(user_content_to_json(user_content));
    Some(recorder)
}

/// Render turn input for the trace.
///
/// Multimodal turns report only their shape: the image bytes are already
/// carried elsewhere and inlining base64 here would dwarf every other
/// attribute in the payload.
#[must_use]
pub fn user_content_to_json(content: &UserContent) -> serde_json::Value {
    match content {
        UserContent::Text(text) => serde_json::Value::String(text.clone()),
        UserContent::Multimodal(parts) => serde_json::json!({
            "type": "multimodal",
            "parts": parts.len(),
        }),
    }
}

/// Convert provider usage counters into the observability model.
///
/// Provider `input_tokens` is already exclusive of cached tokens, so the
/// buckets are carried across as-is rather than being summed: Langfuse prices
/// cache reads and writes differently from fresh input, and folding them
/// together silently inflates reported cost.
#[must_use]
pub const fn to_token_usage(usage: &Usage) -> TokenUsage {
    TokenUsage::from_provider_totals(
        usage.input_tokens,
        usage.output_tokens,
        usage.cache_read_tokens,
        usage.cache_write_tokens,
    )
}

/// Name for the generation step of a given iteration.
#[must_use]
pub fn generation_name(provider: &str, model: &str) -> String {
    format!("{provider}/{model}")
}

/// Attach request parameters already exposed by the provider and agent model.
pub fn set_generation_parameters(
    step: &mut moltis_observability::StepGuard,
    provider: &dyn LlmProvider,
    controls: &AgentToolControls,
) {
    if let Some(effort) = provider.reasoning_effort() {
        step.set_model_parameter("reasoning_effort", serde_json::json!(effort.as_str()));
    }
    if let Some(tool_choice) = &controls.tool_choice {
        step.set_model_parameter("tool_choice", serde_json::json!(tool_choice));
    }
}

/// Open a generation observation for the concrete provider selected for an attempt.
pub fn begin_generation_step(
    recorder: Option<&TurnRecorder>,
    identity: &ProviderIdentity,
    provider: &dyn LlmProvider,
    controls: &AgentToolControls,
    iteration: usize,
    tool_count: usize,
    messages: &[ChatMessage],
) -> Option<moltis_observability::StepGuard> {
    recorder.map(|recorder| {
        let mut step = recorder.step(
            ObservationKind::Generation,
            generation_name(&identity.provider, &identity.model),
        );
        step.set_model(identity.model.clone());
        step.set_metadata("provider", serde_json::json!(identity.provider));
        set_generation_parameters(&mut step, provider, controls);
        step.set_metadata("iteration", serde_json::json!(iteration));
        step.set_metadata("tool_count", serde_json::json!(tool_count));
        step.set_input(serde_json::Value::Array(
            messages.iter().map(message_to_telemetry_value).collect(),
        ));
        step
    })
}

/// Serialize a provider message without copying inline image payloads into telemetry.
fn message_to_telemetry_value(message: &ChatMessage) -> serde_json::Value {
    let ChatMessage::User {
        content: UserContent::Multimodal(parts),
        name,
    } = message
    else {
        return message.to_openai_value();
    };

    let content = parts
        .iter()
        .map(|part| match part {
            ContentPart::Text(text) => serde_json::json!({ "type": "text", "text": text }),
            ContentPart::Image { media_type, .. } => serde_json::json!({
                "type": "image",
                "media_type": media_type,
                "data": "[omitted]",
            }),
        })
        .collect::<Vec<_>>();
    let mut value = serde_json::json!({ "role": "user", "content": content });
    if let Some(sanitized) = name
        .as_ref()
        .and_then(|name| ChatMessage::sanitize_message_name(name))
    {
        value["name"] = serde_json::Value::String(sanitized);
    }
    value
}

/// Extract a provider's terminal reason from a raw streaming event.
#[must_use]
pub fn finish_reason(raw: &serde_json::Value) -> Option<String> {
    raw.pointer("/choices/0/finish_reason")
        .or_else(|| raw.get("stop_reason"))
        .or_else(|| raw.pointer("/response/status"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

/// Extract separately reported reasoning-token usage from a raw event.
#[must_use]
pub fn reasoning_tokens(raw: &serde_json::Value) -> Option<u32> {
    [
        "/usage/completion_tokens_details/reasoning_tokens",
        "/usage/output_tokens_details/reasoning_tokens",
        "/response/usage/output_tokens_details/reasoning_tokens",
        "/choices/0/usage/completion_tokens_details/reasoning_tokens",
    ]
    .iter()
    .find_map(|pointer| raw.pointer(pointer).and_then(serde_json::Value::as_u64))
    .map(|tokens| u32::try_from(tokens).unwrap_or(u32::MAX))
}

/// Close the trace root for every return path from an agent loop.
pub fn finish_turn(
    recorder: Option<&TurnRecorder>,
    result: &Result<AgentRunResult, AgentRunError>,
) {
    let Some(recorder) = recorder else {
        return;
    };
    match result {
        Ok(run) => {
            recorder.set_output(serde_json::Value::String(run.text.clone()));
            recorder.set_metadata("iterations", serde_json::json!(run.iterations));
            recorder.set_metadata("tool_calls", serde_json::json!(run.tool_calls_made));
            recorder.finish();
        },
        Err(error) => recorder.finish_with_error(error.to_string()),
    }
}

/// Close a tool observation with the exact content persisted for the model.
pub fn finish_tool_step(
    step: Option<moltis_observability::StepGuard>,
    persisted_output: &str,
    success: bool,
    error: Option<&str>,
    persisted_result: &serde_json::Value,
) {
    let Some(mut step) = step else {
        return;
    };
    step.set_output(serde_json::Value::String(persisted_output.to_string()));
    let failure = error
        .map(str::to_string)
        .or_else(|| persisted_tool_result_failure(persisted_result));
    if !success || failure.is_some() {
        step.fail(failure.unwrap_or_else(|| "tool execution failed".to_string()));
    }
    step.finish();
}

/// The observation kind a tool call should be recorded as.
///
/// Retrieval tools are reported as `RETRIEVER` so backends that special-case
/// RAG steps light up, rather than showing every tool as an opaque `TOOL`.
#[must_use]
pub fn tool_observation_kind(tool_name: &str) -> ObservationKind {
    const RETRIEVAL_TOOLS: &[&str] = &[
        "memory_search",
        "memory_query",
        "code_search",
        "code_index_search",
        "web_search",
        "search",
    ];
    if RETRIEVAL_TOOLS.contains(&tool_name) {
        ObservationKind::Retriever
    } else {
        ObservationKind::Tool
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use moltis_observability::{Event, ObservationSink, RecorderSettings, RedactionPolicy};

    use super::*;

    #[derive(Default)]
    struct CollectingSink {
        events: Mutex<Vec<Event>>,
    }

    #[async_trait::async_trait]
    impl ObservationSink for CollectingSink {
        fn name(&self) -> &str {
            "agent-test"
        }

        fn record(&self, event: Event) {
            self.events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(event);
        }

        async fn flush(&self, _timeout: Duration) -> anyhow::Result<()> {
            Ok(())
        }
    }

    fn test_recorder(sink: Arc<CollectingSink>) -> TurnRecorder {
        TurnRecorder::begin_with_sink(sink, "agent-run", TraceScope::default(), RecorderSettings {
            redaction: RedactionPolicy::default(),
            capture_input: true,
            capture_output: true,
            capture_tool_io: true,
            sample_rate: 1.0,
        })
        .expect("sampled recorder")
    }

    fn binding() -> ChannelBinding {
        ChannelBinding {
            surface: Some("chat".into()),
            session_kind: None,
            channel_type: Some("telegram".into()),
            account_id: Some("acct-1".into()),
            chat_id: Some("chat-1".into()),
            outbound_to: Some("chat-1".into()),
            chat_type: Some("dm".into()),
            sender_id: Some("42".into()),
        }
    }

    #[test]
    fn session_key_becomes_the_backend_session_id() {
        let scope = trace_scope(
            "agent:main:main",
            None,
            "production".into(),
            "20260726.01".into(),
        );
        assert_eq!(scope.session_id.as_deref(), Some("agent:main:main"));
    }

    #[test]
    fn empty_session_keys_are_omitted_rather_than_grouped() {
        // An empty string would collapse every unattributed turn into one
        // giant session in the backend's session view.
        let scope = trace_scope("", None, "production".into(), "1.0".into());
        assert!(scope.session_id.is_none());
    }

    #[test]
    fn user_id_is_namespaced_by_channel() {
        // Otherwise Telegram user 42 and Slack user 42 merge into one person.
        let scope = trace_scope(
            "agent:main:main",
            Some(&binding()),
            "production".into(),
            "1.0".into(),
        );
        assert_eq!(scope.user_id.as_deref(), Some("telegram:42"));
    }

    #[test]
    fn sender_without_a_channel_type_is_used_verbatim() {
        let unnamed = ChannelBinding {
            channel_type: None,
            ..binding()
        };
        let scope = trace_scope(
            "agent:main:main",
            Some(&unnamed),
            "prod".into(),
            "1.0".into(),
        );
        assert_eq!(scope.user_id.as_deref(), Some("42"));
    }

    #[test]
    fn channel_provenance_and_agent_become_tags() {
        let scope = trace_scope(
            "agent:support:channel:telegram:account:a:peer:user:42",
            Some(&binding()),
            "production".into(),
            "1.0".into(),
        );

        assert!(scope.tags.contains(&"channel:telegram".to_string()));
        assert!(scope.tags.contains(&"surface:chat".to_string()));
        assert!(scope.tags.contains(&"chat:dm".to_string()));
        assert!(scope.tags.contains(&"agent:support".to_string()));
    }

    #[test]
    fn a_turn_with_no_channel_still_produces_a_usable_scope() {
        let scope = trace_scope("agent:main:main", None, "staging".into(), "1.0".into());

        assert!(scope.user_id.is_none());
        assert_eq!(scope.tags, vec!["agent:main".to_string()]);
        assert_eq!(scope.environment.as_deref(), Some("staging"));
    }

    #[test]
    fn malformed_session_keys_do_not_produce_an_empty_agent_tag() {
        let scope = trace_scope("agent:", None, "prod".into(), "1.0".into());
        assert!(
            !scope.tags.iter().any(|t| t == "agent:"),
            "empty agent tag is noise in the backend's tag filter"
        );
    }

    #[test]
    fn usage_conversion_preserves_cache_buckets_separately() {
        let usage = Usage {
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 900,
            cache_write_tokens: 20,
        };
        let converted = to_token_usage(&usage);

        assert_eq!(converted.input, 100);
        assert_eq!(converted.cache_read, 900);
        assert_eq!(converted.cache_write, 20);
        // Summing cache into input would inflate the priced fresh-input count.
        assert_ne!(converted.input, 1020);
    }

    #[test]
    fn multimodal_input_reports_shape_not_image_bytes() {
        let json = user_content_to_json(&UserContent::Multimodal(Vec::new()));
        assert_eq!(json["type"], "multimodal");
        assert_eq!(json["parts"], 0);
    }

    #[test]
    fn generation_messages_omit_inline_image_data() {
        let sentinel = "sensitive-base64-payload";
        let message = ChatMessage::user_multimodal_named(
            vec![
                ContentPart::Text("describe this".into()),
                ContentPart::Image {
                    media_type: "image/png".into(),
                    data: sentinel.into(),
                },
            ],
            "Channel User",
        );

        let value = message_to_telemetry_value(&message);
        let encoded = value.to_string();
        assert!(!encoded.contains(sentinel));
        assert!(!encoded.contains("data:image/png;base64"));
        assert_eq!(value["content"][1]["media_type"], "image/png");
        assert_eq!(value["content"][1]["data"], "[omitted]");
        assert_eq!(value["name"], "Channel_User");
    }

    #[test]
    fn text_input_is_carried_verbatim() {
        let json = user_content_to_json(&UserContent::Text("hello".into()));
        assert_eq!(json, serde_json::Value::String("hello".into()));
    }

    #[test]
    fn retrieval_tools_are_recorded_as_retriever_observations() {
        assert_eq!(
            tool_observation_kind("memory_search"),
            ObservationKind::Retriever
        );
        assert_eq!(
            tool_observation_kind("code_search"),
            ObservationKind::Retriever
        );
        assert_eq!(tool_observation_kind("exec"), ObservationKind::Tool);
    }

    #[test]
    fn generation_name_identifies_provider_and_model() {
        assert_eq!(
            generation_name("anthropic", "claude-opus-4"),
            "anthropic/claude-opus-4"
        );
    }

    #[test]
    fn finish_reason_supports_chat_anthropic_and_responses_shapes() {
        assert_eq!(
            finish_reason(&serde_json::json!({"choices": [{"finish_reason": "tool_calls"}]}))
                .as_deref(),
            Some("tool_calls")
        );
        assert_eq!(
            finish_reason(&serde_json::json!({"stop_reason": "end_turn"})).as_deref(),
            Some("end_turn")
        );
        assert_eq!(
            finish_reason(&serde_json::json!({"response": {"status": "completed"}})).as_deref(),
            Some("completed")
        );
    }

    #[test]
    fn reasoning_tokens_supports_chat_and_responses_usage() {
        assert_eq!(
            reasoning_tokens(&serde_json::json!({
                "usage": {"completion_tokens_details": {"reasoning_tokens": 42}}
            })),
            Some(42)
        );
        assert_eq!(
            reasoning_tokens(&serde_json::json!({
                "response": {"usage": {"output_tokens_details": {"reasoning_tokens": 84}}}
            })),
            Some(84)
        );
    }

    #[test]
    fn collecting_sink_receives_successful_and_failed_turn_completions() {
        let sink = Arc::new(CollectingSink::default());
        let successful = test_recorder(Arc::clone(&sink));
        finish_turn(
            Some(&successful),
            &Ok(AgentRunResult {
                text: "finished".into(),
                iterations: 2,
                tool_calls_made: 1,
                usage: Usage::default(),
                request_usage: Usage::default(),
                raw_llm_responses: Vec::new(),
            }),
        );

        let failed = test_recorder(Arc::clone(&sink));
        finish_turn(
            Some(&failed),
            &Err(AgentRunError::Other(anyhow::anyhow!("provider failed"))),
        );

        let traces = sink
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter_map(|event| match event {
                Event::Trace(trace) => Some((**trace).clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(traces.iter().any(|trace| {
            trace.output == Some(serde_json::Value::String("finished".into()))
                && trace.metadata.get("iterations") == Some(&serde_json::json!(2))
        }));
        assert!(traces.iter().any(|trace| {
            trace.metadata.get("error") == Some(&serde_json::json!("provider failed"))
        }));
    }

    #[test]
    fn collecting_sink_gets_persisted_tool_output_and_logical_failure() {
        let sink = Arc::new(CollectingSink::default());
        let recorder = test_recorder(Arc::clone(&sink));
        let mut step = recorder.step(ObservationKind::Tool, "browser");
        step.set_input(serde_json::json!({"url": "https://example.com"}));
        let persisted = serde_json::json!({"result": {"success": false, "secret": "[removed]"}});
        let output = persisted.to_string();

        finish_tool_step(Some(step), &output, true, None, &persisted);

        let events = sink
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tool = events.iter().find_map(|event| match event {
            Event::ObservationEnd(observation) if observation.kind == ObservationKind::Tool => {
                Some(observation)
            },
            _ => None,
        });
        let tool = tool.expect("completed tool observation");
        assert_eq!(tool.output, Some(serde_json::Value::String(output)));
        assert_eq!(tool.level, moltis_observability::Level::Error);
        assert_eq!(
            tool.status_message.as_deref(),
            Some("tool returned success: false")
        );
    }
}
