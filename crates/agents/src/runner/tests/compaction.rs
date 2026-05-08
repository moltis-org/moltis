use {super::helpers::*, crate::model::ChatMessage};

#[test]
fn compact_oldest_first_compacts_earliest_tool_result() {
    let mut messages = vec![
        ChatMessage::tool("id1", "a".repeat(300)),
        ChatMessage::tool("id2", "b".repeat(300)),
    ];
    let reduced = compact_tool_results_oldest_first_in_place(&mut messages, 1);
    assert!(reduced > 0, "should have compacted something");

    let ChatMessage::Tool {
        tool_call_id,
        content,
    } = &messages[0]
    else {
        panic!("expected Tool message");
    };
    assert_eq!(tool_call_id, "id1");
    assert_eq!(content, TOOL_RESULT_COMPACTION_PLACEHOLDER);

    match &messages[1] {
        ChatMessage::Tool { content, .. } => {
            assert_ne!(content, TOOL_RESULT_COMPACTION_PLACEHOLDER);
        },
        _ => panic!("expected Tool message"),
    }
}

#[test]
fn compact_oldest_first_skips_already_compacted() {
    let mut messages = vec![
        ChatMessage::tool("id1", TOOL_RESULT_COMPACTION_PLACEHOLDER),
        ChatMessage::tool("id2", "b".repeat(300)),
    ];
    let reduced = compact_tool_results_oldest_first_in_place(&mut messages, 1);
    assert!(reduced > 0);

    match &messages[0] {
        ChatMessage::Tool { content, .. } => {
            assert_eq!(content, TOOL_RESULT_COMPACTION_PLACEHOLDER);
        },
        _ => panic!("expected Tool message"),
    }
    match &messages[1] {
        ChatMessage::Tool { content, .. } => {
            assert_eq!(content, TOOL_RESULT_COMPACTION_PLACEHOLDER);
        },
        _ => panic!("expected Tool message"),
    }
}

#[test]
fn compact_oldest_first_skips_small_results() {
    let mut messages = vec![
        ChatMessage::tool("id1", "a".repeat(50)),
        ChatMessage::tool("id2", "b".repeat(300)),
    ];
    let reduced = compact_tool_results_oldest_first_in_place(&mut messages, 1);
    assert!(reduced > 0);

    match &messages[0] {
        ChatMessage::Tool { content, .. } => {
            assert_ne!(content, TOOL_RESULT_COMPACTION_PLACEHOLDER);
            assert_eq!(content.len(), 50);
        },
        _ => panic!("expected Tool message"),
    }
    match &messages[1] {
        ChatMessage::Tool { content, .. } => {
            assert_eq!(content, TOOL_RESULT_COMPACTION_PLACEHOLDER);
        },
        _ => panic!("expected Tool message"),
    }
}

#[test]
fn compact_oldest_first_returns_zero_when_nothing_to_compact() {
    let mut messages = vec![ChatMessage::tool("id1", "short")];
    let reduced = compact_tool_results_oldest_first_in_place(&mut messages, 100);
    assert_eq!(reduced, 0);
}

#[test]
fn compact_oldest_first_returns_zero_for_zero_tokens_needed() {
    let mut messages = vec![ChatMessage::tool("id1", "a".repeat(300))];
    let reduced = compact_tool_results_oldest_first_in_place(&mut messages, 0);
    assert_eq!(reduced, 0);
}

#[test]
fn compact_oldest_first_stops_once_budget_freed() {
    let mut messages = vec![
        ChatMessage::tool("id1", "a".repeat(500)),
        ChatMessage::tool("id2", "b".repeat(500)),
        ChatMessage::tool("id3", "c".repeat(500)),
    ];
    let reduced = compact_tool_results_oldest_first_in_place(&mut messages, 1);
    assert!(reduced > 0);

    match &messages[0] {
        ChatMessage::Tool { content, .. } => {
            assert_eq!(content, TOOL_RESULT_COMPACTION_PLACEHOLDER);
        },
        _ => panic!("expected Tool message"),
    }
    match &messages[2] {
        ChatMessage::Tool { content, .. } => {
            assert_ne!(content, TOOL_RESULT_COMPACTION_PLACEHOLDER);
        },
        _ => panic!("expected Tool message"),
    }
}

#[test]
fn enforce_budget_ratio_zero_disables_compaction_ok_when_under_overflow() {
    let mut messages = vec![ChatMessage::tool("id1", "a".repeat(300))];
    let result = enforce_tool_result_context_budget(&mut messages, &[], 100_000, 0, 90);
    assert!(result.is_ok());

    match &messages[0] {
        ChatMessage::Tool { content, .. } => {
            assert_ne!(content, TOOL_RESULT_COMPACTION_PLACEHOLDER);
        },
        _ => panic!("expected Tool message"),
    }
}

#[test]
fn enforce_budget_ratio_zero_errors_on_overflow() {
    let mut messages = vec![ChatMessage::tool("id1", "a".repeat(500))];
    let result = enforce_tool_result_context_budget(&mut messages, &[], 10, 0, 90);
    assert!(result.is_err());
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("compaction disabled"),
        "error should mention compaction disabled: {msg}"
    );
}

#[test]
fn enforce_budget_compacts_when_over_compaction_threshold() {
    let mut messages = vec![
        ChatMessage::tool("id1", "a".repeat(300)),
        ChatMessage::tool("id2", "b".repeat(300)),
    ];
    let result = enforce_tool_result_context_budget(&mut messages, &[], 100, 75, 90);
    assert!(result.is_ok());

    let compacted = messages
        .iter()
        .filter(|message| {
            matches!(message, ChatMessage::Tool { content, .. } if content == TOOL_RESULT_COMPACTION_PLACEHOLDER)
        })
        .count();
    assert!(compacted > 0, "at least one message should be compacted");
}

#[test]
fn enforce_budget_errors_when_over_overflow_even_after_compaction() {
    let mut messages = vec![
        ChatMessage::tool("id1", TOOL_RESULT_COMPACTION_PLACEHOLDER),
        ChatMessage::tool("id2", "tiny"),
    ];
    let result = enforce_tool_result_context_budget(&mut messages, &[], 5, 75, 90);
    assert!(result.is_err());
}

#[test]
fn enforce_budget_noop_when_no_tool_results() {
    let mut messages = vec![ChatMessage::user("hello")];
    let result = enforce_tool_result_context_budget(&mut messages, &[], 100, 75, 90);
    assert!(result.is_ok());

    match &messages[0] {
        ChatMessage::User { .. } => {},
        _ => panic!("expected User message"),
    }
}

#[test]
fn enforce_budget_noop_when_context_window_zero() {
    let mut messages = vec![ChatMessage::tool("id1", "a".repeat(300))];
    let result = enforce_tool_result_context_budget(&mut messages, &[], 0, 75, 90);
    assert!(result.is_ok());
}
