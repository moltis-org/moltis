#![allow(clippy::unwrap_used, clippy::expect_used)]

use {
    moltis_protocol::chat_event::{ChatErrorInfo, ChatFinal, PartialMessage},
    serde_json::json,
};

use super::*;

fn history(values: Value) -> Vec<PersistedMessage> {
    serde_json::from_value(values).unwrap()
}

fn delta(text: &str) -> ChatEventKind {
    ChatEventKind::Delta { text: text.into() }
}

fn final_text(text: &str) -> ChatEventKind {
    ChatEventKind::Final(ChatFinal {
        text: text.into(),
        model: "gpt".into(),
        output_tokens: 10,
        duration_ms: 2000,
        ..ChatFinal::default()
    })
}

fn kinds(t: &Transcript) -> Vec<&ItemKind> {
    t.items().iter().map(|i| &i.kind).collect()
}

fn assistant(t: &Transcript, index: usize) -> &AssistantMessage {
    match &t.items()[index].kind {
        ItemKind::Assistant(m) => m,
        other => panic!("expected assistant, got {other:?}"),
    }
}

#[test]
fn history_maps_messages() {
    let t = Transcript::from_history(&history(json!([
        {"role": "system", "content": "sys"},
        {"role": "user", "content": [{"type": "text", "text": "look"}, {"type": "image_url", "image_url": {"url": "data:"}}]},
        {"role": "assistant", "content": "", "tool_calls": []},
        {"role": "tool_result", "tool_call_id": "t1", "tool_name": "exec",
         "arguments": {"command": "ls -la\nsecond line"}, "success": true,
         "result": {"stdout": "a\nb\n", "stderr": "", "exit_code": 0}},
        {"role": "tool", "tool_call_id": "t1", "content": "raw"},
        {"role": "assistant", "content": "Done", "model": "m", "outputTokens": 4, "durationMs": 1000},
        {"role": "notice", "content": "heads up"},
    ])));
    let items = kinds(&t);
    assert_eq!(items.len(), 4);
    assert_eq!(items[0], &ItemKind::User {
        text: "look\n[image]".into()
    });
    let ItemKind::Tool(tool) = items[1] else {
        panic!("expected tool");
    };
    assert_eq!(tool.summary, "ls -la");
    assert_eq!(tool.status, ToolStatus::Succeeded);
    assert_eq!(tool.output.as_deref(), Some("a\nb"));
    let done = assistant(&t, 2);
    assert_eq!(done.text, "Done");
    assert_eq!(done.usage.as_ref().unwrap().tokens_per_second(), Some(4.0));
    assert!(matches!(items[3], ItemKind::Notice { .. }));
    assert_eq!(t.state(), RunState::Idle);
}

#[test]
fn streaming_turn() {
    let mut t = Transcript::default();
    assert_eq!(
        t.apply(&ChatEventKind::UserMessage { text: "hi".into() }),
        Change::Appended { at: 0, count: 1 }
    );
    assert_eq!(t.state(), RunState::Thinking);
    assert_eq!(t.apply(&ChatEventKind::Thinking), Change::None);
    assert_eq!(t.apply(&delta("Hel")), Change::Appended { at: 1, count: 1 });
    assert_eq!(t.apply(&delta("lo")), Change::Updated { index: 1 });
    assert_eq!(t.state(), RunState::Streaming);
    assert!(assistant(&t, 1).streaming);
    assert_eq!(assistant(&t, 1).text, "Hello");

    assert_eq!(t.apply(&final_text("Hello!")), Change::Updated { index: 1 });
    let done = assistant(&t, 1);
    assert_eq!(done.text, "Hello!", "final text is authoritative");
    assert!(!done.streaming);
    assert_eq!(done.model.as_deref(), Some("gpt"));
    assert_eq!(t.state(), RunState::Idle);
}

#[test]
fn reasoning_replaces_previous() {
    let mut t = Transcript::default();
    t.apply(&ChatEventKind::ThinkingText { text: "a".into() });
    t.apply(&ChatEventKind::ThinkingText { text: "ab".into() });
    assert_eq!(assistant(&t, 0).reasoning.as_deref(), Some("ab"));
    assert_eq!(t.items().len(), 1);
}

#[test]
fn tool_calls_split_drafts() {
    let mut t = Transcript::default();
    t.apply(&delta("Let me check."));
    let change = t.apply(&ChatEventKind::ToolCallStart {
        tool_call_id: "t1".into(),
        tool_name: "exec".into(),
        arguments: Some(json!({"command": "date"})),
        execution_mode: None,
    });
    assert_eq!(change, Change::Reset);
    assert_eq!(t.state(), RunState::RunningTool);
    assert!(!assistant(&t, 0).streaming);

    let change = t.apply(&ChatEventKind::ToolCallEnd {
        tool_call_id: "t1".into(),
        tool_name: "exec".into(),
        success: true,
        result: Some(json!({"stdout": "Mon\n"})),
        error: None,
    });
    assert_eq!(change, Change::Updated { index: 1 });
    let ItemKind::Tool(tool) = &t.items()[1].kind else {
        panic!("expected tool");
    };
    assert_eq!(tool.output.as_deref(), Some("Mon"));

    t.apply(&delta("It is Monday."));
    assert_eq!(
        t.items().len(),
        3,
        "deltas after a tool start a new message"
    );
    t.apply(&final_text("It is Monday."));
    assert_eq!(assistant(&t, 2).text, "It is Monday.");
    assert_eq!(assistant(&t, 0).text, "Let me check.");
}

#[test]
fn tool_without_preceding_text_appends() {
    let mut t = Transcript::default();
    let change = t.apply(&ChatEventKind::ToolCallStart {
        tool_call_id: "t1".into(),
        tool_name: "web_fetch".into(),
        arguments: Some(json!({"url": "https://x"})),
        execution_mode: None,
    });
    assert_eq!(change, Change::Appended { at: 0, count: 1 });
    let change = t.apply(&ChatEventKind::ToolCallEnd {
        tool_call_id: "t1".into(),
        tool_name: "web_fetch".into(),
        success: false,
        result: None,
        error: Some(json!({"message": "blocked"})),
    });
    assert_eq!(change, Change::Updated { index: 0 });
    let ItemKind::Tool(tool) = &t.items()[0].kind else {
        panic!("expected tool");
    };
    assert_eq!(tool.status, ToolStatus::Failed);
    assert_eq!(tool.output.as_deref(), Some("blocked"));
    assert_eq!(tool.summary, "https://x");
}

#[test]
fn unknown_tool_end_is_ignored() {
    let mut t = Transcript::default();
    let change = t.apply(&ChatEventKind::ToolCallEnd {
        tool_call_id: "nope".into(),
        tool_name: String::new(),
        success: true,
        result: None,
        error: None,
    });
    assert_eq!(change, Change::None);
}

#[test]
fn final_without_draft_appends_and_empty_final_is_ignored() {
    let mut t = Transcript::default();
    assert_eq!(t.apply(&final_text("")), Change::None);
    assert_eq!(t.apply(&final_text("Answer")), Change::Appended {
        at: 0,
        count: 1
    });
    assert_eq!(assistant(&t, 0).text, "Answer");
}

#[test]
fn error_closes_draft() {
    let mut t = Transcript::default();
    t.apply(&delta("partial"));
    let change = t.apply(&ChatEventKind::Error {
        error: ChatErrorInfo {
            detail: Some("quota exceeded".into()),
            ..ChatErrorInfo::default()
        },
    });
    assert_eq!(change, Change::Reset);
    assert!(!assistant(&t, 0).streaming);
    assert_eq!(t.items()[1].kind, ItemKind::Error {
        message: "quota exceeded".into()
    });
    assert_eq!(t.state(), RunState::Idle);
}

#[test]
fn error_without_draft_appends() {
    let mut t = Transcript::default();
    let change = t.apply(&ChatEventKind::Error {
        error: ChatErrorInfo::default(),
    });
    assert_eq!(change, Change::Appended { at: 0, count: 1 });
}

#[test]
fn empty_draft_is_removed() {
    let mut t = Transcript::default();
    t.apply(&ChatEventKind::UserMessage { text: "q".into() });
    t.apply(&delta(""));
    assert_eq!(t.items().len(), 2);
    t.apply(&ChatEventKind::Aborted {
        partial_message: None,
        message_index: None,
    });
    assert_eq!(t.items().len(), 1);
}

#[test]
fn abort_marks_draft() {
    let mut t = Transcript::default();
    t.apply(&delta("half"));
    let change = t.apply(&ChatEventKind::Aborted {
        partial_message: None,
        message_index: None,
    });
    assert_eq!(change, Change::Updated { index: 0 });
    let message = assistant(&t, 0);
    assert!(message.aborted);
    assert!(!message.streaming);
}

#[test]
fn abort_uses_partial_when_nothing_streamed() {
    let mut t = Transcript::default();
    t.apply(&ChatEventKind::UserMessage { text: "q".into() });
    t.apply(&ChatEventKind::Aborted {
        partial_message: Some(PartialMessage {
            content: Some("so far".into()),
            ..PartialMessage::default()
        }),
        message_index: Some(1),
    });
    assert_eq!(assistant(&t, 1).text, "so far");
    assert!(assistant(&t, 1).aborted);
}

#[test]
fn notices_retries_and_queue() {
    let mut t = Transcript::default();
    t.apply(&ChatEventKind::Retrying {
        error: ChatErrorInfo {
            title: Some("Rate limited".into()),
            ..ChatErrorInfo::default()
        },
        retry_after_ms: Some(1500),
    });
    t.apply(&ChatEventKind::Notice {
        title: "Auto-continue".into(),
        message: "continuing".into(),
    });
    t.apply(&ChatEventKind::Queued);
    let ItemKind::Notice { message, .. } = &t.items()[0].kind else {
        panic!("expected notice");
    };
    assert_eq!(message, "Rate limited — retrying in 2s");
    assert_eq!(t.items().len(), 3);
    assert_eq!(t.apply(&ChatEventKind::Other), Change::None);
    assert_eq!(t.apply(&ChatEventKind::QueueCleared), Change::None);
}

#[test]
fn session_cleared_resets_but_keeps_ids_unique() {
    let mut t = Transcript::default();
    t.apply(&ChatEventKind::UserMessage { text: "a".into() });
    let first_id = t.items()[0].id;
    assert_eq!(t.apply(&ChatEventKind::SessionCleared), Change::Reset);
    assert!(t.items().is_empty());
    t.apply(&ChatEventKind::UserMessage { text: "b".into() });
    assert_ne!(t.items()[0].id, first_id);
}

#[test]
fn long_output_is_truncated_on_char_boundary() {
    let long = "é".repeat(MAX_TOOL_OUTPUT);
    let out = tool_output(&json!(long));
    assert!(out.ends_with('…'));
    assert!(out.len() <= MAX_TOOL_OUTPUT + '…'.len_utf8());
}

#[test]
fn argument_summary_fallbacks() {
    assert_eq!(argument_summary(&json!({})), "");
    assert_eq!(argument_summary(&json!({"x": 1})), r#"{"x":1}"#);
    assert_eq!(argument_summary(&json!({"query": "rust"})), "rust");
    assert_eq!(error_text(&json!("plain")), "plain");
    assert_eq!(error_text(&json!({"detail": "d"})), "d");
    assert_eq!(error_text(&json!({"code": 1})), r#"{"code":1}"#);
}

#[test]
fn tokens_per_second_needs_data() {
    let usage = Usage {
        input_tokens: 1,
        output_tokens: 0,
        duration_ms: 10,
    };
    assert_eq!(usage.tokens_per_second(), None);
}
