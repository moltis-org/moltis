//! Typed payloads of the `chat` WebSocket event.
//!
//! The gateway emits one `chat` event per streaming step, discriminated by its
//! `state` field. Clients deserialize them into [`ChatEvent`]; states this
//! version does not know about land in [`ChatEventKind::Other`] so older
//! clients keep working against newer daemons.

use serde::{Deserialize, Serialize};

/// Name of the WebSocket event carrying [`ChatEvent`] payloads.
pub const CHAT_EVENT: &str = "chat";

/// One `chat` event payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatEvent {
    #[serde(default)]
    pub session_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Client-assigned sequence number of the user message being answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    #[serde(flatten)]
    pub kind: ChatEventKind,
}

/// The state-specific part of a [`ChatEvent`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ChatEventKind {
    /// A user message was accepted (also echoed for messages sent elsewhere).
    UserMessage {
        #[serde(default)]
        text: String,
    },
    /// The model started thinking.
    Thinking,
    /// The model finished thinking.
    ThinkingDone,
    /// The reasoning text so far; replaces any previous reasoning of the run.
    ThinkingText {
        #[serde(default)]
        text: String,
    },
    /// A chunk of answer text to append.
    Delta {
        #[serde(default)]
        text: String,
    },
    ToolCallStart {
        #[serde(default)]
        tool_call_id: String,
        #[serde(default)]
        tool_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arguments: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        execution_mode: Option<String>,
    },
    ToolCallEnd {
        #[serde(default)]
        tool_call_id: String,
        #[serde(default)]
        tool_name: String,
        #[serde(default)]
        success: bool,
        /// Tool-specific output (`stdout`, `stderr`, `exit_code`, ...).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<serde_json::Value>,
    },
    /// The run finished and the assistant message was persisted.
    Final(ChatFinal),
    Error {
        #[serde(default)]
        error: ChatErrorInfo,
    },
    /// A provider error that is retried after `retry_after_ms`.
    Retrying {
        #[serde(default)]
        error: ChatErrorInfo,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retry_after_ms: Option<u64>,
    },
    Aborted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        partial_message: Option<PartialMessage>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_index: Option<usize>,
    },
    /// An informational banner inside the transcript.
    Notice {
        #[serde(default)]
        title: String,
        #[serde(default)]
        message: String,
    },
    /// The message was queued behind an active run.
    Queued,
    QueueCleared,
    SessionCleared,
    /// Any state this client version does not understand.
    #[serde(other)]
    Other,
}

/// Payload of the `final` state.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChatFinal {
    pub text: String,
    pub model: String,
    pub provider: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_write_tokens: u32,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
}

/// Structured provider/runner error attached to `error` and `retrying`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChatErrorInfo {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

impl ChatErrorInfo {
    /// The most specific human-readable text available.
    #[must_use]
    pub fn display_text(&self) -> &str {
        self.detail
            .as_deref()
            .or(self.message.as_deref())
            .or(self.title.as_deref())
            .unwrap_or("Unknown error")
    }
}

/// What was streamed before a run was aborted.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PartialMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, serde_json::json};

    fn parse(value: serde_json::Value) -> ChatEvent {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn delta_parses_with_common_fields() {
        let event = parse(json!({
            "runId": "r1", "sessionKey": "main", "state": "delta", "text": "Hel", "seq": 3
        }));
        assert_eq!(event.session_key, "main");
        assert_eq!(event.run_id.as_deref(), Some("r1"));
        assert_eq!(event.seq, Some(3));
        assert_eq!(event.kind, ChatEventKind::Delta { text: "Hel".into() });
    }

    #[test]
    fn final_parses_usage() {
        let event = parse(json!({
            "runId": "r1", "sessionKey": "main", "state": "final", "text": "Hello",
            "model": "gpt", "provider": "openai", "inputTokens": 10, "outputTokens": 5,
            "cacheReadTokens": 0, "cacheWriteTokens": 0, "durationMs": 1200,
            "messageIndex": 4, "replyMedium": "text", "iterations": 1
        }));
        let ChatEventKind::Final(done) = event.kind else {
            panic!("expected final");
        };
        assert_eq!(done.text, "Hello");
        assert_eq!(done.output_tokens, 5);
        assert_eq!(done.duration_ms, 1200);
        assert_eq!(done.message_index, Some(4));
    }

    #[test]
    fn tool_calls_parse() {
        let start = parse(json!({
            "sessionKey": "main", "state": "tool_call_start", "toolCallId": "t1",
            "toolName": "exec", "arguments": {"command": "ls"}, "executionMode": "sandbox"
        }));
        assert!(matches!(
            start.kind,
            ChatEventKind::ToolCallStart { ref tool_name, ref execution_mode, .. }
                if tool_name == "exec" && execution_mode.as_deref() == Some("sandbox")
        ));
        let end = parse(json!({
            "sessionKey": "main", "state": "tool_call_end", "toolCallId": "t1",
            "toolName": "exec", "success": false, "error": {"message": "boom"}
        }));
        assert!(matches!(end.kind, ChatEventKind::ToolCallEnd {
            success: false,
            ..
        }));
    }

    #[test]
    fn error_display_prefers_detail() {
        let event = parse(json!({
            "sessionKey": "main", "state": "error",
            "error": {"type": "rate_limit_exceeded", "title": "Rate limited", "detail": "Slow down"}
        }));
        let ChatEventKind::Error { error } = event.kind else {
            panic!("expected error");
        };
        assert_eq!(error.kind.as_deref(), Some("rate_limit_exceeded"));
        assert_eq!(error.display_text(), "Slow down");
        assert_eq!(ChatErrorInfo::default().display_text(), "Unknown error");
    }

    #[test]
    fn unknown_state_is_other() {
        let event = parse(json!({"sessionKey": "main", "state": "sub_agent_start", "x": 1}));
        assert_eq!(event.kind, ChatEventKind::Other);
    }

    #[test]
    fn unit_states_parse() {
        for (state, kind) in [
            ("thinking", ChatEventKind::Thinking),
            ("thinking_done", ChatEventKind::ThinkingDone),
            ("queued", ChatEventKind::Queued),
            ("session_cleared", ChatEventKind::SessionCleared),
        ] {
            assert_eq!(parse(json!({"sessionKey": "s", "state": state})).kind, kind);
        }
    }

    #[test]
    fn aborted_keeps_partial() {
        let event = parse(json!({
            "sessionKey": "s", "state": "aborted", "runId": "r",
            "partialMessage": {"content": "half"}, "messageIndex": 2
        }));
        let ChatEventKind::Aborted {
            partial_message,
            message_index,
        } = event.kind
        else {
            panic!("expected aborted");
        };
        assert_eq!(partial_message.unwrap().content.as_deref(), Some("half"));
        assert_eq!(message_index, Some(2));
    }

    #[test]
    fn round_trips() {
        let event = ChatEvent {
            session_key: "main".into(),
            run_id: Some("r".into()),
            seq: None,
            kind: ChatEventKind::Notice {
                title: "Auto-continue".into(),
                message: "continuing".into(),
            },
        };
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["state"], "notice");
        assert_eq!(parse(value), event);
    }
}
