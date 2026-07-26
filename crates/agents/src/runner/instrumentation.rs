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

use crate::model::{Usage, UserContent};

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
mod tests {
    use super::*;

    fn binding() -> ChannelBinding {
        ChannelBinding {
            surface: Some("chat".into()),
            session_kind: None,
            channel_type: Some("telegram".into()),
            account_id: Some("acct-1".into()),
            chat_id: Some("chat-1".into()),
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
}
