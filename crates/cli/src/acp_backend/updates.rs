//! Translating Moltis chat broadcasts into ACP `session/update` notifications.
//!
//! The gateway broadcasts a turn's progress as JSON event frames — the same
//! ones the Web UI consumes over its WebSocket. This module is the pure mapping
//! from those frames to ACP updates, kept free of gateway and protocol wiring so
//! it can be tested against literal frames.
//!
//! Two details of the wire format drive the shape here:
//!
//! - `thinking_text` carries the *accumulated* reasoning so far, not a delta,
//!   because the Web UI replaces its thinking pane wholesale on each one. ACP
//!   thought chunks are incremental, so [`FrameMapper`] keeps the previous text
//!   and emits only what was appended.
//! - Frames for every session on the box arrive on the same broadcast, so each
//!   one must be matched against the session this turn belongs to before it is
//!   forwarded. Skipping that check would leak one client's tokens into another
//!   client's turn.

use {agent_client_protocol as acp, serde_json::Value};

/// What a single broadcast frame means for the turn in progress.
///
/// Not `Eq`: `SessionUpdate` carries content blocks that are only `PartialEq`.
#[derive(Debug, PartialEq)]
pub enum FrameAction {
    /// Forward these updates to the client.
    Emit(Vec<acp::SessionUpdate>),
    /// The run reported an error; the turn should end with this message.
    Failed(String),
    /// Not for this turn, or carries nothing a client can render.
    Ignore,
}

/// Stateful mapper for one turn's frames.
///
/// Holds the reasoning text seen so far so accumulated `thinking_text` frames
/// can be converted into incremental thought chunks.
#[derive(Debug, Default)]
pub struct FrameMapper {
    reasoning_seen: String,
}

impl FrameMapper {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Maps one raw frame, as serialized onto the broadcast, for `session_key`.
    ///
    /// Unparseable frames and frames for other sessions are ignored rather than
    /// erroring: the broadcast is shared infrastructure and a malformed or
    /// unrelated frame is not this turn's problem.
    pub fn map(&mut self, raw: &str, session_key: &str) -> FrameAction {
        let Ok(frame) = serde_json::from_str::<Value>(raw) else {
            return FrameAction::Ignore;
        };
        if frame.get("event").and_then(Value::as_str) != Some("chat") {
            return FrameAction::Ignore;
        }
        let Some(payload) = frame.get("payload") else {
            return FrameAction::Ignore;
        };
        if payload.get("sessionKey").and_then(Value::as_str) != Some(session_key) {
            return FrameAction::Ignore;
        }
        self.map_payload(payload)
    }

    fn map_payload(&mut self, payload: &Value) -> FrameAction {
        let text = || payload.get("text").and_then(Value::as_str).unwrap_or("");
        match payload.get("state").and_then(Value::as_str) {
            Some("delta") => {
                let delta = text();
                if delta.is_empty() {
                    return FrameAction::Ignore;
                }
                FrameAction::Emit(vec![agent_message(delta)])
            },
            Some("thinking_text") => self.reasoning_delta(text()),
            Some("tool_call_start") => FrameAction::Emit(vec![tool_call_start(payload)]),
            Some("tool_call_end") => FrameAction::Emit(vec![tool_call_end(payload)]),
            Some("error") => FrameAction::Failed(first_non_empty(
                &[
                    payload.get("error").and_then(Value::as_str),
                    payload.get("text").and_then(Value::as_str),
                ],
                "agent run failed",
            )),
            // `user_message` echoes what the client just sent us, `queued`,
            // `iteration`, `thinking`, and the rest are Web-UI affordances with
            // no ACP equivalent. Dropping them keeps the client's transcript to
            // what the agent actually said.
            _ => FrameAction::Ignore,
        }
    }

    /// Converts an accumulated reasoning snapshot into the newly added suffix.
    fn reasoning_delta(&mut self, accumulated: &str) -> FrameAction {
        // A shorter or diverging snapshot means the provider restarted its
        // reasoning rather than extended it. Emitting the whole thing again is
        // better than emitting a nonsense suffix computed from a stale prefix.
        let addition = match accumulated.strip_prefix(self.reasoning_seen.as_str()) {
            Some(suffix) => suffix.to_string(),
            None => accumulated.to_string(),
        };
        self.reasoning_seen = accumulated.to_string();
        if addition.is_empty() {
            return FrameAction::Ignore;
        }
        FrameAction::Emit(vec![agent_thought(addition)])
    }
}

fn agent_message(text: impl Into<String>) -> acp::SessionUpdate {
    acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(acp::ContentBlock::from(
        text.into(),
    )))
}

fn agent_thought(text: impl Into<String>) -> acp::SessionUpdate {
    acp::SessionUpdate::AgentThoughtChunk(acp::ContentChunk::new(acp::ContentBlock::from(
        text.into(),
    )))
}

/// Identifier the client uses to correlate a tool call's start with its end.
///
/// Moltis labels tool calls by name within a run, so the name is what the two
/// frames have in common.
fn tool_call_id(payload: &Value) -> String {
    first_non_empty(
        &[
            payload.get("toolCallId").and_then(Value::as_str),
            payload.get("id").and_then(Value::as_str),
            payload.get("tool").and_then(Value::as_str),
            payload.get("name").and_then(Value::as_str),
        ],
        "tool",
    )
}

fn tool_name(payload: &Value) -> String {
    first_non_empty(
        &[
            payload.get("tool").and_then(Value::as_str),
            payload.get("name").and_then(Value::as_str),
        ],
        "tool",
    )
}

fn tool_call_start(payload: &Value) -> acp::SessionUpdate {
    acp::SessionUpdate::ToolCall(
        acp::ToolCall::new(
            acp::ToolCallId::from(tool_call_id(payload)),
            tool_name(payload),
        )
        .status(acp::ToolCallStatus::InProgress),
    )
}

fn tool_call_end(payload: &Value) -> acp::SessionUpdate {
    // Moltis reports a failed tool call with an `error` field; without one the
    // call completed. Reporting a failure as completed would leave the client
    // showing a green tool call for something that did not work.
    let status = if payload.get("error").is_some() {
        acp::ToolCallStatus::Failed
    } else {
        acp::ToolCallStatus::Completed
    };
    let mut fields = acp::ToolCallUpdateFields::default();
    fields.status = Some(status);
    acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
        acp::ToolCallId::from(tool_call_id(payload)),
        fields,
    ))
}

/// First non-empty candidate, or `fallback` when every candidate is absent.
fn first_non_empty(candidates: &[Option<&str>], fallback: &str) -> String {
    candidates
        .iter()
        .flatten()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, serde_json::json};

    const SESSION: &str = "acp:s1";

    fn frame(payload: Value) -> String {
        json!({ "type": "event", "event": "chat", "payload": payload }).to_string()
    }

    fn chat_frame(state: &str, text: &str) -> String {
        frame(json!({ "sessionKey": SESSION, "state": state, "text": text }))
    }

    /// Extracts the text of a single emitted content chunk.
    fn emitted_text(action: &FrameAction) -> String {
        let FrameAction::Emit(updates) = action else {
            panic!("expected an emission, got {action:?}");
        };
        assert_eq!(updates.len(), 1, "expected exactly one update");
        match &updates[0] {
            acp::SessionUpdate::AgentMessageChunk(chunk)
            | acp::SessionUpdate::AgentThoughtChunk(chunk) => match &chunk.content {
                acp::ContentBlock::Text(text) => text.text.clone(),
                other => panic!("expected text content, got {other:?}"),
            },
            other => panic!("expected a content chunk, got {other:?}"),
        }
    }

    #[test]
    fn deltas_become_agent_message_chunks() {
        let mut mapper = FrameMapper::new();
        let action = mapper.map(&chat_frame("delta", "hello "), SESSION);
        assert_eq!(emitted_text(&action), "hello ");
    }

    #[test]
    fn frames_for_other_sessions_are_ignored() {
        let mut mapper = FrameMapper::new();
        let other = frame(json!({
            "sessionKey": "acp:someone-else",
            "state": "delta",
            "text": "not yours",
        }));
        assert_eq!(mapper.map(&other, SESSION), FrameAction::Ignore);
    }

    #[test]
    fn non_chat_events_are_ignored() {
        let mut mapper = FrameMapper::new();
        let tick = json!({ "type": "event", "event": "tick", "payload": {} }).to_string();
        assert_eq!(mapper.map(&tick, SESSION), FrameAction::Ignore);
    }

    #[test]
    fn malformed_frames_are_ignored_rather_than_erroring() {
        let mut mapper = FrameMapper::new();
        assert_eq!(mapper.map("{not json", SESSION), FrameAction::Ignore);
        assert_eq!(mapper.map("null", SESSION), FrameAction::Ignore);
    }

    #[test]
    fn accumulated_reasoning_is_emitted_as_increments() {
        let mut mapper = FrameMapper::new();
        // The Web UI receives the whole reasoning each time; ACP wants only the
        // part that is new.
        let first = mapper.map(&chat_frame("thinking_text", "Let me"), SESSION);
        assert_eq!(emitted_text(&first), "Let me");
        let second = mapper.map(&chat_frame("thinking_text", "Let me think"), SESSION);
        assert_eq!(emitted_text(&second), " think");
    }

    #[test]
    fn unchanged_reasoning_emits_nothing() {
        let mut mapper = FrameMapper::new();
        mapper.map(&chat_frame("thinking_text", "same"), SESSION);
        assert_eq!(
            mapper.map(&chat_frame("thinking_text", "same"), SESSION),
            FrameAction::Ignore
        );
    }

    #[test]
    fn restarted_reasoning_is_emitted_whole() {
        let mut mapper = FrameMapper::new();
        mapper.map(&chat_frame("thinking_text", "first attempt"), SESSION);
        // Diverges from what we saw: not an extension, so the suffix arithmetic
        // would produce garbage.
        let action = mapper.map(&chat_frame("thinking_text", "different"), SESSION);
        assert_eq!(emitted_text(&action), "different");
    }

    #[test]
    fn empty_deltas_are_ignored() {
        let mut mapper = FrameMapper::new();
        assert_eq!(
            mapper.map(&chat_frame("delta", ""), SESSION),
            FrameAction::Ignore
        );
    }

    #[test]
    fn errors_surface_the_reported_message() {
        let mut mapper = FrameMapper::new();
        let raw = frame(json!({
            "sessionKey": SESSION,
            "state": "error",
            "error": "provider exploded",
        }));
        assert_eq!(
            mapper.map(&raw, SESSION),
            FrameAction::Failed("provider exploded".to_string())
        );
    }

    #[test]
    fn errors_without_a_message_still_fail_the_turn() {
        let mut mapper = FrameMapper::new();
        let raw = frame(json!({ "sessionKey": SESSION, "state": "error" }));
        assert_eq!(
            mapper.map(&raw, SESSION),
            FrameAction::Failed("agent run failed".to_string())
        );
    }

    #[test]
    fn tool_calls_start_in_progress_and_complete() {
        let mut mapper = FrameMapper::new();
        let start = frame(json!({
            "sessionKey": SESSION,
            "state": "tool_call_start",
            "tool": "read_file",
        }));
        let FrameAction::Emit(updates) = mapper.map(&start, SESSION) else {
            panic!("tool call start must be forwarded");
        };
        match &updates[0] {
            acp::SessionUpdate::ToolCall(call) => {
                assert_eq!(call.title, "read_file");
                assert_eq!(call.status, acp::ToolCallStatus::InProgress);
            },
            other => panic!("expected a tool call, got {other:?}"),
        }

        let end = frame(json!({
            "sessionKey": SESSION,
            "state": "tool_call_end",
            "tool": "read_file",
        }));
        let FrameAction::Emit(updates) = mapper.map(&end, SESSION) else {
            panic!("tool call end must be forwarded");
        };
        match &updates[0] {
            acp::SessionUpdate::ToolCallUpdate(update) => {
                assert_eq!(update.fields.status, Some(acp::ToolCallStatus::Completed));
            },
            other => panic!("expected a tool call update, got {other:?}"),
        }
    }

    #[test]
    fn a_failed_tool_call_is_not_reported_as_completed() {
        let mut mapper = FrameMapper::new();
        let end = frame(json!({
            "sessionKey": SESSION,
            "state": "tool_call_end",
            "tool": "exec",
            "error": "non-zero exit",
        }));
        let FrameAction::Emit(updates) = mapper.map(&end, SESSION) else {
            panic!("tool call end must be forwarded");
        };
        match &updates[0] {
            acp::SessionUpdate::ToolCallUpdate(update) => {
                assert_eq!(update.fields.status, Some(acp::ToolCallStatus::Failed));
            },
            other => panic!("expected a tool call update, got {other:?}"),
        }
    }

    #[test]
    fn tool_call_ids_pair_start_with_end() {
        let mut mapper = FrameMapper::new();
        let start = frame(json!({
            "sessionKey": SESSION, "state": "tool_call_start", "tool": "grep",
        }));
        let end = frame(json!({
            "sessionKey": SESSION, "state": "tool_call_end", "tool": "grep",
        }));
        let (FrameAction::Emit(started), FrameAction::Emit(ended)) =
            (mapper.map(&start, SESSION), mapper.map(&end, SESSION))
        else {
            panic!("both frames must be forwarded");
        };
        let (acp::SessionUpdate::ToolCall(call), acp::SessionUpdate::ToolCallUpdate(update)) =
            (&started[0], &ended[0])
        else {
            panic!("unexpected update kinds");
        };
        assert_eq!(
            call.tool_call_id, update.tool_call_id,
            "a client cannot pair the two without a stable id"
        );
    }

    #[test]
    fn web_ui_only_states_are_dropped() {
        let mut mapper = FrameMapper::new();
        for state in [
            "user_message",
            "queued",
            "iteration",
            "thinking",
            "thinking_done",
            "voice_pending",
            "notice",
        ] {
            assert_eq!(
                mapper.map(&chat_frame(state, "x"), SESSION),
                FrameAction::Ignore,
                "{state} should not reach an ACP client"
            );
        }
    }
}
