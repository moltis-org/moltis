//! Tests for per-run tool controls (`active_tools` and `tool_choice`).

use std::sync::Arc;

use super::helpers::*;

// ── active_tools filtering ──────────────────────────────────────────────

#[tokio::test]
async fn active_tools_hides_unselected_tool() {
    // ToolCallingProvider calls "echo_tool" on its first turn.
    // With active_tools restricting to a different name, the runner rejects
    // the call (validation error), but the model still gets a second turn
    // and returns "Done!".
    let provider = Arc::new(ToolCallingProvider {
        call_count: std::sync::atomic::AtomicUsize::new(0),
    });
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(EchoTool));

    let tool_context = serde_json::json!({
        "active_tools": ["some_other_tool"],
    });

    let result = run_agent_loop_with_context_and_limits(
        provider,
        &tools,
        "You are a test bot.",
        &UserContent::text("Hi"),
        None,
        None,
        Some(tool_context),
        None,
        None,
        AgentLoopLimits {
            max_iterations: Some(3),
        },
    )
    .await
    .unwrap();

    // The tool call was attempted (counts as 1) but rejected.
    // The model then responded with text on the next iteration.
    assert_eq!(result.text, "Done!");
    assert_eq!(result.tool_calls_made, 1);
    assert_eq!(result.iterations, 2);
}

#[tokio::test]
async fn active_tools_allows_selected_tool() {
    let provider = Arc::new(ToolCallingProvider {
        call_count: std::sync::atomic::AtomicUsize::new(0),
    });
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(EchoTool));

    let tool_context = serde_json::json!({
        "active_tools": ["echo_tool"],
    });

    let result = run_agent_loop_with_context_and_limits(
        provider,
        &tools,
        "You are a test bot.",
        &UserContent::text("Hi"),
        None,
        None,
        Some(tool_context),
        None,
        None,
        AgentLoopLimits::default(),
    )
    .await
    .unwrap();

    assert_eq!(result.text, "Done!");
    assert_eq!(result.tool_calls_made, 1);
}

// ── tool_choice = none ──────────────────────────────────────────────────

#[tokio::test]
async fn tool_choice_none_forces_text_only() {
    // With tool_choice=none the provider receives an empty tool list.
    // MockProvider returns text regardless.
    let provider = Arc::new(MockProvider {
        response_text: "text-only".into(),
    });
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(EchoTool));

    let tool_context = serde_json::json!({
        "tool_choice": { "type": "none" },
    });

    let result = run_agent_loop_with_context_and_limits(
        provider,
        &tools,
        "You are a test bot.",
        &UserContent::text("Hi"),
        None,
        None,
        Some(tool_context),
        None,
        None,
        AgentLoopLimits::default(),
    )
    .await
    .unwrap();

    assert_eq!(result.text, "text-only");
    assert_eq!(result.tool_calls_made, 0);
}

// ── tool_choice = tool referencing missing tool ─────────────────────────

#[tokio::test]
async fn tool_choice_forced_missing_tool_errors() {
    let provider = Arc::new(ToolCallingProvider {
        call_count: std::sync::atomic::AtomicUsize::new(0),
    });
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(EchoTool));

    let tool_context = serde_json::json!({
        "active_tools": ["echo_tool"],
        "tool_choice": { "type": "tool", "name": "nonexistent_tool" },
    });

    let err = run_agent_loop_with_context_and_limits(
        provider,
        &tools,
        "You are a test bot.",
        &UserContent::text("Hi"),
        None,
        None,
        Some(tool_context),
        None,
        None,
        AgentLoopLimits::default(),
    )
    .await
    .unwrap_err();

    let msg = err.to_string();
    assert!(
        msg.contains("unavailable tool"),
        "expected unavailable tool error, got: {msg}"
    );
}

// ── tool_choice = auto with active_tools ─────────────────────────────

#[tokio::test]
async fn tool_choice_auto_with_active_tools_succeeds() {
    let provider = Arc::new(ToolCallingProvider {
        call_count: std::sync::atomic::AtomicUsize::new(0),
    });
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(EchoTool));

    let tool_context = serde_json::json!({
        "active_tools": ["echo_tool"],
        "tool_choice": { "type": "auto" },
    });

    let result = run_agent_loop_with_context_and_limits(
        provider,
        &tools,
        "You are a test bot.",
        &UserContent::text("Hi"),
        None,
        None,
        Some(tool_context),
        None,
        None,
        AgentLoopLimits::default(),
    )
    .await
    .unwrap();

    assert_eq!(result.text, "Done!");
    assert_eq!(result.tool_calls_made, 1);
}

#[tokio::test]
async fn tool_choice_any_with_no_active_tools_errors() {
    let provider = Arc::new(ToolCallingProvider {
        call_count: std::sync::atomic::AtomicUsize::new(0),
    });
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(EchoTool));

    let tool_context = serde_json::json!({
        "active_tools": ["missing_tool"],
        "tool_choice": { "type": "any" },
    });

    let err = run_agent_loop_with_context_and_limits(
        provider,
        &tools,
        "You are a test bot.",
        &UserContent::text("Hi"),
        None,
        None,
        Some(tool_context),
        None,
        None,
        AgentLoopLimits::default(),
    )
    .await
    .unwrap_err();

    let msg = err.to_string();
    assert!(
        msg.contains("tool_choice any requires at least one active tool"),
        "expected any tool_choice active tool error, got: {msg}"
    );
}

// ── no tool_context defaults to empty controls ──────────────────────────

#[tokio::test]
async fn no_tool_context_runs_normally() {
    let provider = Arc::new(ToolCallingProvider {
        call_count: std::sync::atomic::AtomicUsize::new(0),
    });
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(EchoTool));

    let result = run_agent_loop_with_context_and_limits(
        provider,
        &tools,
        "You are a test bot.",
        &UserContent::text("Hi"),
        None,
        None,
        None, // no tool_context
        None,
        None,
        AgentLoopLimits::default(),
    )
    .await
    .unwrap();

    assert_eq!(result.text, "Done!");
    assert_eq!(result.tool_calls_made, 1);
}
