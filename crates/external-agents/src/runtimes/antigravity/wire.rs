//! Tolerant decoder for `agy -p --output-format stream-json` NDJSON frames.

#![allow(dead_code)]

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(super) enum AgyState {
    Active,
    Done,
    Error,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum AgyStepType {
    UserInput,
    AgentResponse,
    Tool,
    Checkpoint,
    SystemMessage,
    ErrorMessage,
    Subagent,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct AgyUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub thinking_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct AgyToolError {
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct AgyToolInfo {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub parameters: Option<serde_json::Value>,
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub error: Option<AgyToolError>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct AgySubagent {
    #[serde(default)]
    pub type_name: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub initial_prompt: String,
    #[serde(default)]
    pub conversation_id: String,
    #[serde(default)]
    pub log_uri: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct AgySubagentInfo {
    #[serde(default)]
    pub subagents: Vec<AgySubagent>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct AgyInit {
    #[serde(default)]
    pub conversation_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct AgyStepUpdate {
    #[serde(default)]
    pub conversation_id: String,
    #[serde(default)]
    pub step_index: u64,
    pub state: AgyState,
    pub step_type: AgyStepType,
    #[serde(default)]
    pub text_delta: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_info: Option<AgyToolInfo>,
    #[serde(default)]
    pub subagent_info: Option<AgySubagentInfo>,
    #[serde(default)]
    pub usage: Option<AgyUsage>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct AgyResult {
    #[serde(default)]
    pub conversation_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub response: String,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub num_turns: u64,
    #[serde(default)]
    pub usage: Option<AgyUsage>,
}

#[derive(Debug, Clone)]
pub(super) enum AgyEvent {
    Init(AgyInit),
    StepUpdate(AgyStepUpdate),
    Result(AgyResult),
}

#[derive(Deserialize)]
struct Envelope {
    event: String,
    #[serde(default)]
    init: Option<serde_json::Value>,
    #[serde(default)]
    step_update: Option<serde_json::Value>,
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(default)]
    conversation_id: Option<String>,
}

/// Unknown frames and non-JSON notices are deliberately ignored so an AGY
/// self-update cannot abort a running turn merely by adding a field or event.
pub(super) fn parse_line(line: &str) -> Option<AgyEvent> {
    let line = line.trim();
    if !line.starts_with('{') {
        return None;
    }
    let envelope: Envelope = serde_json::from_str(line).ok()?;
    match envelope.event.as_str() {
        "init" => {
            let mut init: AgyInit = serde_json::from_value(envelope.init?).ok()?;
            if init.conversation_id.is_empty() {
                init.conversation_id = envelope.conversation_id.unwrap_or_default();
            }
            Some(AgyEvent::Init(init))
        },
        "step_update" => Some(AgyEvent::StepUpdate(
            serde_json::from_value(envelope.step_update?).ok()?,
        )),
        "result" => Some(AgyEvent::Result(
            serde_json::from_value(envelope.result?).ok()?,
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_session_id_from_init_envelope() {
        let event =
            parse_line(r#"{"event":"init","conversation_id":"conv-1","init":{"cwd":"/tmp"}}"#);
        let Some(AgyEvent::Init(init)) = event else {
            panic!("expected init event");
        };
        assert_eq!(init.conversation_id, "conv-1");
    }

    #[test]
    fn parses_tool_and_usage_fields() {
        let event = parse_line(
            r#"{"event":"step_update","step_update":{"conversation_id":"conv-1","step_index":3,"state":"DONE","step_type":"tool","tool_name":"run_command","tool_info":{"name":"run_command","parameters":{"CommandLine":"pwd"},"output":"/tmp\n"},"usage":{"input_tokens":4,"output_tokens":2,"total_tokens":6}}}"#,
        );
        let Some(AgyEvent::StepUpdate(step)) = event else {
            panic!("expected step update");
        };
        assert_eq!(step.state, AgyState::Done);
        assert_eq!(step.step_type, AgyStepType::Tool);
        assert_eq!(step.usage.map(|usage| usage.total_tokens), Some(6));
    }

    #[test]
    fn unknown_and_plain_text_frames_are_non_fatal() {
        assert!(parse_line("AGY notice").is_none());
        assert!(parse_line(r#"{"event":"future_event","payload":{}}"#).is_none());
        let event = parse_line(
            r#"{"event":"step_update","step_update":{"step_index":9,"state":"DONE","step_type":"future_step"}}"#,
        );
        assert!(matches!(
            event,
            Some(AgyEvent::StepUpdate(AgyStepUpdate {
                step_type: AgyStepType::Unknown,
                ..
            }))
        ));
    }
}
