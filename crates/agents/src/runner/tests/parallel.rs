//! Parallel tool execution tests.

use std::sync::Arc;

use {
    super::helpers::*,
    crate::model::{ToolCall, UserContent},
};

#[tokio::test]
async fn test_parallel_tool_execution() {
    let provider = Arc::new(MultiToolProvider {
        call_count: std::sync::atomic::AtomicUsize::new(0),
        tool_calls: vec![
            ToolCall {
                id: "c1".into(),
                name: "tool_a".into(),
                arguments: serde_json::json!({}),
                metadata: None,
            },
            ToolCall {
                id: "c2".into(),
                name: "tool_b".into(),
                arguments: serde_json::json!({}),
                metadata: None,
            },
            ToolCall {
                id: "c3".into(),
                name: "tool_c".into(),
                arguments: serde_json::json!({}),
                metadata: None,
            },
        ],
    });

    let mut tools = ToolRegistry::new();
    tools.register(Box::new(SlowTool {
        tool_name: "tool_a".into(),
        delay_ms: 0,
    }));
    tools.register(Box::new(SlowTool {
        tool_name: "tool_b".into(),
        delay_ms: 0,
    }));
    tools.register(Box::new(SlowTool {
        tool_name: "tool_c".into(),
        delay_ms: 0,
    }));

    let events: Arc<std::sync::Mutex<Vec<RunnerEvent>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);
    let on_event: OnEvent = Box::new(move |event| {
        events_clone.lock().unwrap().push(event);
    });

    let uc = UserContent::text("Use all tools");
    let result = run_agent_loop(provider, &tools, "Test bot", &uc, Some(&on_event), None)
        .await
        .unwrap();

    assert_eq!(result.text, "All done");
    assert_eq!(result.tool_calls_made, 3);

    let evts = events.lock().unwrap();
    let starts: Vec<_> = evts
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, RunnerEvent::ToolCallStart { .. }))
        .map(|(i, _)| i)
        .collect();
    let ends: Vec<_> = evts
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, RunnerEvent::ToolCallEnd { .. }))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(starts.len(), 3);
    assert_eq!(ends.len(), 3);
    assert!(
        starts.iter().all(|s| ends.iter().all(|e| s < e)),
        "all starts should precede all ends"
    );
}

#[tokio::test]
async fn test_parallel_tool_one_fails() {
    let provider = Arc::new(MultiToolProvider {
        call_count: std::sync::atomic::AtomicUsize::new(0),
        tool_calls: vec![
            ToolCall {
                id: "c1".into(),
                name: "tool_a".into(),
                arguments: serde_json::json!({}),
                metadata: None,
            },
            ToolCall {
                id: "c2".into(),
                name: "fail_tool".into(),
                arguments: serde_json::json!({}),
                metadata: None,
            },
            ToolCall {
                id: "c3".into(),
                name: "tool_c".into(),
                arguments: serde_json::json!({}),
                metadata: None,
            },
        ],
    });

    let mut tools = ToolRegistry::new();
    tools.register(Box::new(SlowTool {
        tool_name: "tool_a".into(),
        delay_ms: 0,
    }));
    tools.register(Box::new(FailTool));
    tools.register(Box::new(SlowTool {
        tool_name: "tool_c".into(),
        delay_ms: 0,
    }));

    let events: Arc<std::sync::Mutex<Vec<RunnerEvent>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);
    let on_event: OnEvent = Box::new(move |event| {
        events_clone.lock().unwrap().push(event);
    });

    let uc = UserContent::text("Use all tools");
    let result = run_agent_loop(provider, &tools, "Test bot", &uc, Some(&on_event), None)
        .await
        .unwrap();

    assert_eq!(result.text, "All done");
    assert_eq!(result.tool_calls_made, 3);

    let evts = events.lock().unwrap();
    let successes = evts
        .iter()
        .filter(|e| matches!(e, RunnerEvent::ToolCallEnd { success: true, .. }))
        .count();
    let failures = evts
        .iter()
        .filter(|e| matches!(e, RunnerEvent::ToolCallEnd { success: false, .. }))
        .count();
    assert_eq!(successes, 2);
    assert_eq!(failures, 1);
}

#[tokio::test]
async fn test_parallel_execution_is_concurrent() {
    let provider = Arc::new(MultiToolProvider {
        call_count: std::sync::atomic::AtomicUsize::new(0),
        tool_calls: vec![
            ToolCall {
                id: "c1".into(),
                name: "slow_a".into(),
                arguments: serde_json::json!({}),
                metadata: None,
            },
            ToolCall {
                id: "c2".into(),
                name: "slow_b".into(),
                arguments: serde_json::json!({}),
                metadata: None,
            },
            ToolCall {
                id: "c3".into(),
                name: "slow_c".into(),
                arguments: serde_json::json!({}),
                metadata: None,
            },
        ],
    });

    let mut tools = ToolRegistry::new();
    tools.register(Box::new(SlowTool {
        tool_name: "slow_a".into(),
        delay_ms: 100,
    }));
    tools.register(Box::new(SlowTool {
        tool_name: "slow_b".into(),
        delay_ms: 100,
    }));
    tools.register(Box::new(SlowTool {
        tool_name: "slow_c".into(),
        delay_ms: 100,
    }));

    let start = std::time::Instant::now();
    let uc = UserContent::text("Use all tools");
    let result = run_agent_loop(provider, &tools, "Test bot", &uc, None, None)
        .await
        .unwrap();
    let elapsed = start.elapsed();

    assert_eq!(result.text, "All done");
    assert_eq!(result.tool_calls_made, 3);
    assert!(
        elapsed < std::time::Duration::from_millis(250),
        "parallel execution took {:?}, expected < 250ms",
        elapsed
    );
}
