#![allow(clippy::unwrap_used, clippy::expect_used)]

use {
    super::*,
    crate::transcript::{ItemKind, RunState},
    moltis_protocol::chat_event::ChatFinal,
    serde_json::{Value, json},
};

fn history(value: Value) -> Vec<PersistedMessage> {
    serde_json::from_value(value).unwrap()
}

fn event(run: &str, kind: ChatEventKind) -> ChatEvent {
    ChatEvent {
        session_key: "s".into(),
        run_id: Some(run.into()),
        seq: None,
        kind,
    }
}

fn user(run: &str) -> ChatEvent {
    event(run, ChatEventKind::UserMessage {
        text: "hello".into(),
    })
}

fn delta(run: &str, text: &str) -> ChatEvent {
    event(run, ChatEventKind::Delta { text: text.into() })
}

fn final_event(run: &str) -> ChatEvent {
    event(
        run,
        ChatEventKind::Final(ChatFinal {
            text: "answer".into(),
            ..ChatFinal::default()
        }),
    )
}

#[test]
fn reconnect_replaces_partial_reply_and_marks_other_sessions_stale() {
    let mut cache = TranscriptCache::default();
    cache.insert_draft("s");
    cache.insert_draft("other");
    cache.apply(&user("r"));
    cache.apply(&delta("r", "ans"));
    assert_eq!(cache.get("s").unwrap().state(), RunState::Streaming);
    cache.invalidate();
    assert!(cache.needs_load("other"));
    let request = cache.begin_load("s").unwrap();
    assert!(cache.finish_load(
        "s",
        request,
        &history(json!([
            {"role":"user", "content":"hello", "run_id":"r"},
            {"role":"assistant", "content":"answer", "run_id":"r"},
            {"role":"user", "content":"from another client", "run_id":"r2"}
        ]))
    ));
    let transcript = cache.get("s").unwrap();
    assert_eq!(transcript.state(), RunState::Idle);
    assert_eq!(transcript.items().len(), 3);
    assert!(
        matches!(&transcript.items()[1].kind, ItemKind::Assistant(m) if m.text == "answer" && !m.streaming)
    );
    assert!(cache.needs_load("other"));
    assert!(!cache.needs_load("s"));
}

#[test]
fn stale_history_cannot_overwrite_a_new_connection() {
    let mut cache = TranscriptCache::default();
    let old = cache.begin_load("s").unwrap();
    assert!(cache.begin_load("s").is_none(), "coalesce concurrent loads");
    cache.invalidate();
    let new = cache.begin_load("s").unwrap();
    assert!(!cache.finish_load("s", old, &[]));
    assert!(!cache.fail_load("s", old));
    assert!(cache.finish_load("s", new, &[]));
}

#[test]
fn snapshot_before_send_preserves_intervening_events() {
    let mut cache = TranscriptCache::default();
    let request = cache.begin_load("s").unwrap();
    cache.apply(&user("r"));
    cache.apply(&delta("r", "ans"));
    cache.apply(&delta("r", "wer"));
    cache.apply(&final_event("r"));
    assert!(cache.finish_load("s", request, &[]));
    let transcript = cache.get("s").unwrap();
    assert_eq!(transcript.items().len(), 2);
    assert_eq!(transcript.state(), RunState::Idle);
    assert!(matches!(&transcript.items()[1].kind, ItemKind::Assistant(m) if m.text == "answer"));
}

#[test]
fn snapshot_during_run_keeps_deltas_without_duplicating_user() {
    let mut cache = TranscriptCache::default();
    // Events may arrive while sessions.list is still in flight too.
    cache.apply(&user("r"));
    let request = cache.begin_load("s").unwrap();
    cache.apply(&delta("r", "ans"));
    cache.finish_load(
        "s",
        request,
        &history(json!([
            {"role":"user", "content":"hello", "run_id":"r"}
        ])),
    );
    assert_eq!(cache.get("s").unwrap().items().len(), 2);
    assert_eq!(cache.get("s").unwrap().state(), RunState::Streaming);
    cache.apply(&delta("r", "wer"));
    cache.apply(&final_event("r"));
    assert!(
        matches!(&cache.get("s").unwrap().items()[1].kind, ItemKind::Assistant(m) if m.text == "answer")
    );
}

#[test]
fn completed_snapshot_deduplicates_buffered_and_delayed_events_by_run() {
    let mut cache = TranscriptCache::default();
    let request = cache.begin_load("s").unwrap();
    cache.apply(&user("r"));
    cache.apply(&delta("r", "answer"));
    cache.finish_load(
        "s",
        request,
        &history(json!([
            {"role":"user", "content":"hello", "run_id":"r"},
            {"role":"assistant", "content":"answer", "run_id":"r"}
        ])),
    );
    cache.apply(&final_event("r"));
    assert_eq!(cache.get("s").unwrap().items().len(), 2);
    cache.apply(&user("different-run"));
    assert_eq!(
        cache.get("s").unwrap().items().len(),
        3,
        "identical text is a distinct message"
    );
}

#[test]
fn tool_snapshot_deduplicates_pre_tool_stream_and_keeps_later_deltas() {
    let mut cache = TranscriptCache::default();
    let request = cache.begin_load("s").unwrap();
    cache.apply(&delta("r", "checking"));
    cache.apply(&event("r", ChatEventKind::ToolCallStart {
        tool_call_id: "t".into(),
        tool_name: "exec".into(),
        arguments: None,
        execution_mode: None,
    }));
    cache.apply(&event("r", ChatEventKind::ToolCallEnd {
        tool_call_id: "t".into(),
        tool_name: "exec".into(),
        success: true,
        result: None,
        error: None,
    }));
    cache.apply(&delta("r", "answer"));
    cache.finish_load("s", request, &history(json!([
        {"role":"user", "content":"hello", "run_id":"r"},
        {"role":"tool_result", "tool_call_id":"t", "tool_name":"exec", "success":true, "run_id":"r"}
    ])));
    let transcript = cache.get("s").unwrap();
    assert_eq!(transcript.items().len(), 3);
    assert!(
        matches!(&transcript.items()[2].kind, ItemKind::Assistant(m) if m.text == "answer" && m.streaming)
    );
}

#[test]
fn failed_load_can_retry_without_losing_buffered_events() {
    let mut cache = TranscriptCache::default();
    let request = cache.begin_load("s").unwrap();
    cache.apply(&user("r"));
    assert!(cache.fail_load("s", request));
    let retry = cache.begin_load("s").unwrap();
    cache.apply(&delta("r", "answer"));
    cache.finish_load("s", retry, &[]);
    assert_eq!(cache.get("s").unwrap().items().len(), 2);
}

#[test]
fn session_clear_during_load_does_not_resurrect_history() {
    let mut cache = TranscriptCache::default();
    let request = cache.begin_load("s").unwrap();
    cache.apply(&event("", ChatEventKind::SessionCleared));
    cache.apply(&user("new"));
    cache.finish_load(
        "s",
        request,
        &history(json!([
            {"role":"user", "content":"old", "run_id":"old"}
        ])),
    );
    assert_eq!(cache.get("s").unwrap().items().len(), 1);
    assert!(
        matches!(&cache.get("s").unwrap().items()[0].kind, ItemKind::User {text} if text == "hello")
    );
}
