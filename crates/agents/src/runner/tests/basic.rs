//! Basic runner tests: parsing, shell commands, sanitization, tool results, vision.

use std::sync::Arc;

use {
    super::helpers::*,
    crate::{
        model::{ChatMessage, CompletionResponse, LlmProvider, StreamEvent, ToolCall, Usage},
        tool_parsing::new_synthetic_tool_call_id,
    },
    anyhow::Result,
    async_trait::async_trait,
    std::pin::Pin,
    tokio_stream::Stream,
};

// ── parse_tool_call_from_text tests (delegates to tool_parsing) ──

#[test]
fn test_parse_tool_call_basic() {
    let text = "```tool_call\n{\"tool\": \"exec\", \"arguments\": {\"command\": \"ls\"}}\n```";
    let (tc, remaining) = parse_tool_call_from_text(text).unwrap();
    assert_eq!(tc.name, "exec");
    assert_eq!(tc.arguments["command"], "ls");
    assert!(tc.id.len() <= 40);
    assert!(remaining.is_none() || remaining.as_deref() == Some(""));
}

#[test]
fn test_parse_tool_call_with_surrounding_text() {
    let text = "I'll run ls for you.\n```tool_call\n{\"tool\": \"exec\", \"arguments\": {\"command\": \"ls\"}}\n```\nHere you go.";
    let (tc, remaining) = parse_tool_call_from_text(text).unwrap();
    assert_eq!(tc.name, "exec");
    let remaining = remaining.unwrap();
    assert!(remaining.contains("I'll run ls"));
    assert!(remaining.contains("Here you go"));
}

#[test]
fn test_parse_tool_call_no_block() {
    let text = "I would run ls but I can't.";
    assert!(parse_tool_call_from_text(text).is_none());
}

#[test]
fn test_parse_tool_call_invalid_json() {
    let text = "```tool_call\nnot json\n```";
    assert!(parse_tool_call_from_text(text).is_none());
}

#[test]
fn test_parse_tool_call_function_block() {
    let text = "<function=process>\n<parameter=action>\nstart\n</parameter>\n<parameter=command>\npwd\n</parameter>\n</function>";
    let (tc, remaining) = parse_tool_call_from_text(text).unwrap();
    assert_eq!(tc.name, "process");
    assert_eq!(tc.arguments["action"], "start");
    assert_eq!(tc.arguments["command"], "pwd");
    assert!(tc.id.len() <= 40);
    assert!(remaining.is_none() || remaining.as_deref() == Some(""));
}

#[test]
fn test_new_synthetic_tool_call_id_is_openai_compatible() {
    let id = new_synthetic_tool_call_id("forced");
    assert!(id.starts_with("forced_"));
    assert!(id.len() <= 40);

    let long_prefix_id = new_synthetic_tool_call_id(
        "prefix_that_is_intentionally_way_too_long_for_openai_tool_call_ids",
    );
    assert!(long_prefix_id.len() <= 40);
}

#[test]
fn test_parse_tool_call_function_block_with_wrapper_and_text() {
    let text = "I'll do it.\n<tool_call>\n<function=process>\n<parameter=action>start</parameter>\n<parameter=command>pwd</parameter>\n</function>\n</tool_call>\nDone.";
    let (tc, remaining) = parse_tool_call_from_text(text).unwrap();
    assert_eq!(tc.name, "process");
    assert_eq!(tc.arguments["action"], "start");
    assert_eq!(tc.arguments["command"], "pwd");
    let remaining = remaining.unwrap();
    assert!(remaining.contains("I'll do it."));
    assert!(remaining.contains("Done."));
    assert!(!remaining.contains("<tool_call>"));
    assert!(!remaining.contains("</tool_call>"));
}

#[test]
fn test_explicit_shell_command_requires_sh_prefix() {
    let uc = UserContent::text("pwd");
    assert!(explicit_shell_command_from_user_content(&uc).is_none());
}

#[test]
fn test_explicit_shell_command_extracts_command() {
    let uc = UserContent::text("/sh pwd");
    assert_eq!(
        explicit_shell_command_from_user_content(&uc).as_deref(),
        Some("pwd")
    );
}

#[test]
fn test_explicit_shell_command_supports_telegram_style_bot_mention() {
    let uc = UserContent::text("/sh@MoltisBot uname -a");
    assert_eq!(
        explicit_shell_command_from_user_content(&uc).as_deref(),
        Some("uname -a")
    );
}

#[test]
fn test_resolve_agent_max_iterations_falls_back_for_zero() {
    assert_eq!(
        resolve_agent_max_iterations(0),
        DEFAULT_AGENT_MAX_ITERATIONS
    );
}

// ── Tests ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_simple_text_response() {
    let provider = Arc::new(MockProvider {
        response_text: "Hello!".into(),
    });
    let tools = ToolRegistry::new();
    let uc = UserContent::text("Hi");
    let result = run_agent_loop(provider, &tools, "You are a test bot.", &uc, None, None)
        .await
        .unwrap();
    assert_eq!(result.text, "Hello!");
    assert_eq!(result.iterations, 1);
    assert_eq!(result.tool_calls_made, 0);
}

struct StreamingUsageProvider;

#[async_trait]
impl LlmProvider for StreamingUsageProvider {
    fn name(&self) -> &str {
        "streaming-usage"
    }

    fn id(&self) -> &str {
        "streaming-usage-model"
    }

    async fn complete(
        &self,
        _messages: &[ChatMessage],
        _tools: &[serde_json::Value],
    ) -> Result<CompletionResponse> {
        Ok(CompletionResponse {
            text: Some("unused".into()),
            tool_calls: vec![],
            usage: Usage::default(),
        })
    }

    fn stream(
        &self,
        _messages: Vec<ChatMessage>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        Box::pin(tokio_stream::empty())
    }

    fn stream_with_tools(
        &self,
        _messages: Vec<ChatMessage>,
        _tools: Vec<serde_json::Value>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        Box::pin(tokio_stream::iter(vec![
            StreamEvent::Delta("cached reply".into()),
            StreamEvent::Done(Usage {
                input_tokens: 13_047,
                output_tokens: 17,
                cache_read_tokens: 12_800,
                cache_write_tokens: 64,
            }),
        ]))
    }
}

#[tokio::test]
async fn test_streaming_runner_preserves_cache_usage() {
    let provider = Arc::new(StreamingUsageProvider);
    let tools = ToolRegistry::new();
    let uc = UserContent::text("another");

    let result = run_agent_loop_streaming(
        provider,
        &tools,
        "You are a test bot.",
        &uc,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(result.text, "cached reply");
    assert_eq!(result.iterations, 1);
    assert_eq!(result.tool_calls_made, 0);
    assert_eq!(result.usage.input_tokens, 13_047);
    assert_eq!(result.usage.output_tokens, 17);
    assert_eq!(result.usage.cache_read_tokens, 12_800);
    assert_eq!(result.usage.cache_write_tokens, 64);
    assert_eq!(result.request_usage.input_tokens, 13_047);
    assert_eq!(result.request_usage.output_tokens, 17);
    assert_eq!(result.request_usage.cache_read_tokens, 12_800);
    assert_eq!(result.request_usage.cache_write_tokens, 64);
}

struct NonStreamingUsageProvider {
    call_count: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl LlmProvider for NonStreamingUsageProvider {
    fn name(&self) -> &str {
        "non-streaming-usage"
    }

    fn id(&self) -> &str {
        "non-streaming-usage-model"
    }

    fn supports_tools(&self) -> bool {
        true
    }

    async fn complete(
        &self,
        _messages: &[ChatMessage],
        _tools: &[serde_json::Value],
    ) -> Result<CompletionResponse> {
        let count = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        if count == 0 {
            Ok(CompletionResponse {
                text: None,
                tool_calls: vec![ToolCall {
                    id: "call_usage_1".into(),
                    name: "echo_tool".into(),
                    arguments: serde_json::json!({"text": "hi"}),
                    metadata: None,
                }],
                usage: Usage {
                    input_tokens: 100,
                    output_tokens: 10,
                    cache_read_tokens: 80,
                    cache_write_tokens: 8,
                },
            })
        } else {
            Ok(CompletionResponse {
                text: Some("Done with cache.".into()),
                tool_calls: vec![],
                usage: Usage {
                    input_tokens: 40,
                    output_tokens: 5,
                    cache_read_tokens: 32,
                    cache_write_tokens: 3,
                },
            })
        }
    }

    fn stream(
        &self,
        _messages: Vec<ChatMessage>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        Box::pin(tokio_stream::empty())
    }
}

#[tokio::test]
async fn test_non_streaming_runner_preserves_total_and_request_cache_usage() {
    let provider = Arc::new(NonStreamingUsageProvider {
        call_count: std::sync::atomic::AtomicUsize::new(0),
    });
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(EchoTool));

    let uc = UserContent::text("Use the tool");
    let result = run_agent_loop(provider, &tools, "You are a test bot.", &uc, None, None)
        .await
        .unwrap();

    assert_eq!(result.text, "Done with cache.");
    assert_eq!(result.iterations, 2);
    assert_eq!(result.tool_calls_made, 1);
    assert_eq!(result.usage.input_tokens, 140);
    assert_eq!(result.usage.output_tokens, 15);
    assert_eq!(result.usage.cache_read_tokens, 112);
    assert_eq!(result.usage.cache_write_tokens, 11);
    assert_eq!(result.request_usage.input_tokens, 40);
    assert_eq!(result.request_usage.output_tokens, 5);
    assert_eq!(result.request_usage.cache_read_tokens, 32);
    assert_eq!(result.request_usage.cache_write_tokens, 3);
}

#[tokio::test]
async fn test_tool_call_loop() {
    let provider = Arc::new(ToolCallingProvider {
        call_count: std::sync::atomic::AtomicUsize::new(0),
    });
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(EchoTool));

    let uc = UserContent::text("Use the tool");
    let result = run_agent_loop(provider, &tools, "You are a test bot.", &uc, None, None)
        .await
        .unwrap();

    assert_eq!(result.text, "Done!");
    assert_eq!(result.iterations, 2);
    assert_eq!(result.tool_calls_made, 1);
}

/// Mock provider that calls the "exec" tool (native) and verifies result fed back.
struct ExecSimulatingProvider {
    call_count: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl LlmProvider for ExecSimulatingProvider {
    fn name(&self) -> &str {
        "mock"
    }

    fn id(&self) -> &str {
        "mock-model"
    }

    fn supports_tools(&self) -> bool {
        true
    }

    async fn complete(
        &self,
        messages: &[ChatMessage],
        _tools: &[serde_json::Value],
    ) -> Result<CompletionResponse> {
        let count = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if count == 0 {
            Ok(CompletionResponse {
                text: None,
                tool_calls: vec![ToolCall {
                    id: "call_exec_1".into(),
                    name: "exec".into(),
                    arguments: serde_json::json!({"command": "echo hello"}),
                    metadata: None,
                }],
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    ..Default::default()
                },
            })
        } else {
            let tool_content = messages
                .iter()
                .find_map(|m| {
                    if let ChatMessage::Tool { content, .. } = m {
                        Some(content.as_str())
                    } else {
                        None
                    }
                })
                .unwrap_or("");
            let parsed: serde_json::Value = serde_json::from_str(tool_content).unwrap();
            let stdout = parsed["result"]["stdout"].as_str().unwrap_or("");
            assert!(stdout.contains("hello"));
            assert_eq!(parsed["result"]["exit_code"].as_i64().unwrap(), 0);
            Ok(CompletionResponse {
                text: Some(format!("The output was: {}", stdout.trim())),
                tool_calls: vec![],
                usage: Usage {
                    input_tokens: 20,
                    output_tokens: 10,
                    ..Default::default()
                },
            })
        }
    }

    fn stream(
        &self,
        _messages: Vec<ChatMessage>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        Box::pin(tokio_stream::empty())
    }
}

#[tokio::test]
async fn test_exec_tool_end_to_end() {
    let provider = Arc::new(ExecSimulatingProvider {
        call_count: std::sync::atomic::AtomicUsize::new(0),
    });
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(TestExecTool));

    let events: Arc<std::sync::Mutex<Vec<RunnerEvent>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);
    let on_event: OnEvent = Box::new(move |event| {
        events_clone.lock().unwrap().push(event);
    });

    let uc = UserContent::text("Run echo hello");
    let result = run_agent_loop(
        provider,
        &tools,
        "You are a test bot.",
        &uc,
        Some(&on_event),
        None,
    )
    .await
    .unwrap();

    assert!(result.text.contains("hello"), "got: {}", result.text);
    assert_eq!(result.iterations, 2);
    assert_eq!(result.tool_calls_made, 1);

    let evts = events.lock().unwrap();
    let has = |name: &str| {
        evts.iter().any(|e| {
            matches!(
                (e, name),
                (RunnerEvent::Thinking, "thinking")
                    | (RunnerEvent::ToolCallStart { .. }, "tool_call_start")
                    | (RunnerEvent::ToolCallEnd { .. }, "tool_call_end")
            )
        })
    };
    assert!(has("tool_call_start"));
    assert!(has("tool_call_end"));
    assert!(has("thinking"));

    let tool_end = evts
        .iter()
        .find(|e| matches!(e, RunnerEvent::ToolCallEnd { .. }));
    if let Some(RunnerEvent::ToolCallEnd { success, name, .. }) = tool_end {
        assert!(success, "exec tool should succeed");
        assert_eq!(name, "exec");
    }
}

/// Test that non-native providers can still execute tools via text parsing.
#[tokio::test]
async fn test_text_based_tool_calling() {
    let provider = Arc::new(TextToolCallingProvider {
        call_count: std::sync::atomic::AtomicUsize::new(0),
    });
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(TestExecTool));

    let events: Arc<std::sync::Mutex<Vec<RunnerEvent>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);
    let on_event: OnEvent = Box::new(move |event| {
        events_clone.lock().unwrap().push(event);
    });

    let uc = UserContent::text("Run echo hello");
    let result = run_agent_loop(
        provider,
        &tools,
        "You are a test bot.",
        &uc,
        Some(&on_event),
        None,
    )
    .await
    .unwrap();

    assert!(result.text.contains("hello"), "got: {}", result.text);
    assert_eq!(result.iterations, 2, "should take 2 iterations");
    assert_eq!(result.tool_calls_made, 1, "should execute 1 tool call");

    let evts = events.lock().unwrap();
    assert!(
        evts.iter()
            .any(|e| matches!(e, RunnerEvent::ToolCallStart { .. }))
    );
    assert!(
        evts.iter()
            .any(|e| matches!(e, RunnerEvent::ToolCallEnd { success: true, .. }))
    );
}

/// Native-tool provider that returns plain text (no structured tool call)
/// on the first turn for a command-like prompt.
struct DirectCommandNoToolProvider {
    call_count: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl LlmProvider for DirectCommandNoToolProvider {
    fn name(&self) -> &str {
        "mock-direct-command"
    }

    fn id(&self) -> &str {
        "mock-direct-command"
    }

    fn supports_tools(&self) -> bool {
        true
    }

    async fn complete(
        &self,
        messages: &[ChatMessage],
        _tools: &[serde_json::Value],
    ) -> Result<CompletionResponse> {
        let count = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if count == 0 {
            Ok(CompletionResponse {
                text: Some("I'll summarize the command output for you.".into()),
                tool_calls: vec![],
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 10,
                    ..Default::default()
                },
            })
        } else {
            let assistant_tool_text = messages
                .iter()
                .find_map(|m| {
                    if let ChatMessage::Assistant {
                        content,
                        tool_calls,
                    } = m
                    {
                        if tool_calls.is_empty() {
                            return None;
                        }
                        return content.as_deref();
                    }
                    None
                })
                .unwrap_or("");
            assert!(
                !assistant_tool_text.is_empty(),
                "forced exec should preserve assistant reasoning text"
            );
            let tool_content = messages
                .iter()
                .find_map(|m| {
                    if let ChatMessage::Tool { content, .. } = m {
                        Some(content.as_str())
                    } else {
                        None
                    }
                })
                .unwrap_or("");
            assert!(
                !tool_content.is_empty(),
                "forced exec should append a tool result message"
            );
            Ok(CompletionResponse {
                text: Some("done".into()),
                tool_calls: vec![],
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    ..Default::default()
                },
            })
        }
    }

    fn stream(
        &self,
        _messages: Vec<ChatMessage>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        Box::pin(tokio_stream::empty())
    }
}

#[tokio::test]
async fn test_explicit_sh_command_forces_exec_non_streaming() {
    let provider = Arc::new(DirectCommandNoToolProvider {
        call_count: std::sync::atomic::AtomicUsize::new(0),
    });
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(TestExecTool));

    let events: Arc<std::sync::Mutex<Vec<RunnerEvent>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);
    let on_event: OnEvent = Box::new(move |event| {
        events_clone.lock().unwrap().push(event);
    });

    let uc = UserContent::text("/sh pwd");
    let result = run_agent_loop(
        provider,
        &tools,
        "You are a test bot.",
        &uc,
        Some(&on_event),
        None,
    )
    .await
    .unwrap();

    assert_eq!(result.iterations, 2);
    assert_eq!(result.tool_calls_made, 1);
    assert_eq!(result.text, "done");

    let evts = events.lock().unwrap();
    let tool_start = evts.iter().find_map(|e| {
        if let RunnerEvent::ToolCallStart {
            name, arguments, ..
        } = e
        {
            Some((name.clone(), arguments.clone()))
        } else {
            None
        }
    });
    assert!(tool_start.is_some(), "should emit ToolCallStart");
    let (name, args) = tool_start.unwrap();
    assert_eq!(name, "exec");
    assert_eq!(args["command"], "pwd");
}

#[tokio::test]
async fn test_unprefixed_command_like_text_does_not_force_exec_non_streaming() {
    let provider = Arc::new(MockProvider {
        response_text: "plain response".to_string(),
    });
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(TestExecTool));

    let events: Arc<std::sync::Mutex<Vec<RunnerEvent>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);
    let on_event: OnEvent = Box::new(move |event| {
        events_clone.lock().unwrap().push(event);
    });

    let uc = UserContent::text("pwd");
    let result = run_agent_loop(
        provider,
        &tools,
        "You are a test bot.",
        &uc,
        Some(&on_event),
        None,
    )
    .await
    .unwrap();

    assert_eq!(result.iterations, 1);
    assert_eq!(result.tool_calls_made, 0);
    assert_eq!(result.text, "plain response");

    let evts = events.lock().unwrap();
    assert!(
        !evts
            .iter()
            .any(|e| matches!(e, RunnerEvent::ToolCallStart { .. })),
        "should not emit ToolCallStart for unprefixed command-like text"
    );
}

/// Native-tool provider that emits XML-like function text instead of
/// structured tool calls.
struct NativeTextFunctionProvider {
    call_count: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl LlmProvider for NativeTextFunctionProvider {
    fn name(&self) -> &str {
        "mock-native-function"
    }

    fn id(&self) -> &str {
        "mock-native-function"
    }

    fn supports_tools(&self) -> bool {
        true
    }

    async fn complete(
        &self,
        messages: &[ChatMessage],
        _tools: &[serde_json::Value],
    ) -> Result<CompletionResponse> {
        let count = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if count == 0 {
            Ok(CompletionResponse {
                text: Some(
                    "<function=process>\n<parameter=action>\nstart\n</parameter>\n<parameter=command>\npwd\n</parameter>\n</function>\n</tool_call>"
                        .into(),
                ),
                tool_calls: vec![],
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 20,
                    ..Default::default()
                },
            })
        } else {
            let tool_content = messages
                .iter()
                .find_map(|m| {
                    if let ChatMessage::Tool { content, .. } = m {
                        Some(content.as_str())
                    } else {
                        None
                    }
                })
                .unwrap_or("");
            assert!(
                tool_content.contains("\"action\":\"start\""),
                "tool result should include action=start, got: {tool_content}"
            );
            assert!(
                tool_content.contains("\"command\":\"pwd\""),
                "tool result should include command=pwd, got: {tool_content}"
            );
            Ok(CompletionResponse {
                text: Some("Process started for pwd".into()),
                tool_calls: vec![],
                usage: Usage {
                    input_tokens: 30,
                    output_tokens: 10,
                    ..Default::default()
                },
            })
        }
    }

    fn stream(
        &self,
        _messages: Vec<ChatMessage>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        Box::pin(tokio_stream::empty())
    }
}

#[tokio::test]
async fn test_native_text_function_tool_calling_non_streaming() {
    let provider = Arc::new(NativeTextFunctionProvider {
        call_count: std::sync::atomic::AtomicUsize::new(0),
    });
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(TestProcessTool));

    let events: Arc<std::sync::Mutex<Vec<RunnerEvent>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);
    let on_event: OnEvent = Box::new(move |event| {
        events_clone.lock().unwrap().push(event);
    });

    let uc = UserContent::text("execute pwd");
    let result = run_agent_loop(
        provider,
        &tools,
        "You are a test bot.",
        &uc,
        Some(&on_event),
        None,
    )
    .await
    .unwrap();

    assert!(result.text.contains("pwd"), "got: {}", result.text);
    assert_eq!(result.iterations, 2, "should take 2 iterations");
    assert_eq!(result.tool_calls_made, 1, "should execute 1 tool call");

    let evts = events.lock().unwrap();
    let tool_start = evts.iter().find_map(|e| {
        if let RunnerEvent::ToolCallStart {
            arguments, name, ..
        } = e
        {
            Some((name.clone(), arguments.clone()))
        } else {
            None
        }
    });
    assert!(tool_start.is_some(), "should emit ToolCallStart");
    let (name, args) = tool_start.unwrap();
    assert_eq!(name, "process");
    assert_eq!(args["action"], "start");
    assert_eq!(args["command"], "pwd");
}

// ── sanitize_tool_result tests ──────────────────────────────────

#[test]
fn test_sanitize_short_input_unchanged() {
    let input = "hello world";
    assert_eq!(sanitize_tool_result(input, 50_000), "hello world");
}

#[test]
fn test_sanitize_truncates_long_input() {
    let input = "x".repeat(1000);
    let result = sanitize_tool_result(&input, 100);
    assert!(result.starts_with("xxxx"));
    assert!(result.contains("[truncated"));
    assert!(result.contains("1000 bytes total"));
}

#[test]
fn test_sanitize_truncate_respects_char_boundary() {
    let input = "é".repeat(100);
    let result = sanitize_tool_result(&input, 51);
    assert!(result.contains("[truncated"));
    let prefix_end = result.find("\n\n[truncated").unwrap();
    assert!(prefix_end <= 51);
    assert_eq!(prefix_end % 2, 0);
}

#[test]
fn test_sanitize_strips_base64_data_uri() {
    let payload = "A".repeat(300);
    let input = format!("before data:image/png;base64,{payload} after");
    let result = sanitize_tool_result(&input, 50_000);
    assert!(!result.contains(&payload));
    assert!(result.contains("[screenshot captured and displayed in UI]"));
    assert!(result.contains("before"));
    assert!(result.contains("after"));
}

#[test]
fn test_sanitize_preserves_short_base64() {
    let payload = "QUFB";
    let input = format!("data:text/plain;base64,{payload}");
    let result = sanitize_tool_result(&input, 50_000);
    assert!(result.contains(payload));
}

#[test]
fn test_sanitize_strips_long_hex() {
    let hex = "a1b2c3d4".repeat(50);
    let input = format!("prefix {hex} suffix");
    let result = sanitize_tool_result(&input, 50_000);
    assert!(!result.contains(&hex));
    assert!(result.contains("[hex data removed"));
    assert!(result.contains("prefix"));
    assert!(result.contains("suffix"));
}

#[test]
fn test_sanitize_preserves_short_hex() {
    let hex = "deadbeef";
    let input = format!("code: {hex}");
    let result = sanitize_tool_result(&input, 50_000);
    assert!(result.contains(hex));
}

// ── extract_images_from_text tests ───────────────────────────────

#[test]
fn test_extract_images_basic() {
    let payload = "A".repeat(300);
    let input = format!("before data:image/png;base64,{payload} after");
    let (images, remaining) = extract_images_from_text(&input);
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].media_type, "image/png");
    assert_eq!(images[0].data, payload);
    assert!(remaining.contains("before"));
    assert!(remaining.contains("after"));
    assert!(!remaining.contains(&payload));
}

#[test]
fn test_extract_images_jpeg() {
    let payload = "B".repeat(300);
    let input = format!("data:image/jpeg;base64,{payload}");
    let (images, remaining) = extract_images_from_text(&input);
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].media_type, "image/jpeg");
    assert_eq!(images[0].data, payload);
    assert!(remaining.trim().is_empty());
}

#[test]
fn test_extract_images_multiple() {
    let payload1 = "A".repeat(300);
    let payload2 = "B".repeat(300);
    let input = format!(
        "first data:image/png;base64,{payload1} middle data:image/jpeg;base64,{payload2} end"
    );
    let (images, remaining) = extract_images_from_text(&input);
    assert_eq!(images.len(), 2);
    assert_eq!(images[0].media_type, "image/png");
    assert_eq!(images[1].media_type, "image/jpeg");
    assert!(remaining.contains("first"));
    assert!(remaining.contains("middle"));
    assert!(remaining.contains("end"));
}

#[test]
fn test_extract_images_ignores_non_image() {
    let payload = "A".repeat(300);
    let input = format!("data:text/plain;base64,{payload}");
    let (images, remaining) = extract_images_from_text(&input);
    assert!(images.is_empty());
    assert!(remaining.contains("data:text/plain"));
}

#[test]
fn test_extract_images_ignores_short_payload() {
    let payload = "QUFB";
    let input = format!("data:image/png;base64,{payload}");
    let (images, remaining) = extract_images_from_text(&input);
    assert!(images.is_empty());
    assert!(remaining.contains(payload));
}

// ── tool_result_to_content tests ─────────────────────────────────

#[test]
fn test_tool_result_to_content_no_vision() {
    let payload = "A".repeat(300);
    let input = format!(r#"{{"screenshot": "data:image/png;base64,{payload}"}}"#);
    let result = tool_result_to_content(&input, 50_000, false);
    assert!(result.is_string());
    let s = result.as_str().unwrap();
    assert!(s.contains("[screenshot captured and displayed in UI]"));
    assert!(!s.contains(&payload));
}

#[test]
fn test_tool_result_to_content_with_vision() {
    let payload = "A".repeat(300);
    let input = format!(r#"Result: data:image/png;base64,{payload} done"#);
    let result = tool_result_to_content(&input, 50_000, true);
    assert!(result.is_array());
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["type"], "text");
    assert!(arr[0]["text"].as_str().unwrap().contains("Result:"));
    assert!(arr[0]["text"].as_str().unwrap().contains("done"));
    assert_eq!(arr[1]["type"], "image_url");
    let url = arr[1]["image_url"]["url"].as_str().unwrap();
    assert!(url.starts_with("data:image/png;base64,"));
    assert!(url.contains(&payload));
}

#[test]
fn test_tool_result_to_content_vision_no_images() {
    let input = r#"{"result": "success", "message": "done"}"#;
    let result = tool_result_to_content(input, 50_000, true);
    assert!(result.is_string());
    assert!(result.as_str().unwrap().contains("success"));
}

#[test]
fn test_tool_result_to_content_vision_only_image() {
    let payload = "A".repeat(300);
    let input = format!("data:image/png;base64,{payload}");
    let result = tool_result_to_content(&input, 50_000, true);
    assert!(result.is_array());
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["type"], "image_url");
}

// ── Vision and image edge cases ─────────────────────────────────

#[tokio::test]
async fn test_vision_provider_tool_result_sanitized() {
    let provider = Arc::new(VisionEnabledProvider {
        call_count: std::sync::atomic::AtomicUsize::new(0),
    });
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(ScreenshotTool));
    let uc = UserContent::text("Take a screenshot");
    let result = run_agent_loop(provider, &tools, "You are a test bot.", &uc, None, None)
        .await
        .unwrap();
    assert_eq!(result.text, "Screenshot processed successfully");
    assert_eq!(result.tool_calls_made, 1);
}

#[tokio::test]
async fn test_tool_call_end_event_contains_raw_result() {
    let provider = Arc::new(VisionEnabledProvider {
        call_count: std::sync::atomic::AtomicUsize::new(0),
    });
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(ScreenshotTool));
    let events: Arc<std::sync::Mutex<Vec<RunnerEvent>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);
    let on_event: OnEvent = Box::new(move |event| {
        events_clone.lock().unwrap().push(event);
    });
    let uc = UserContent::text("Take a screenshot");
    let result = run_agent_loop(
        provider,
        &tools,
        "You are a test bot.",
        &uc,
        Some(&on_event),
        None,
    )
    .await
    .unwrap();
    assert_eq!(result.tool_calls_made, 1);
    let evts = events.lock().unwrap();
    let tool_end = evts
        .iter()
        .find(|e| matches!(e, RunnerEvent::ToolCallEnd { success: true, .. }));
    if let Some(RunnerEvent::ToolCallEnd {
        success,
        result: Some(result_json),
        ..
    }) = tool_end
    {
        assert!(success);
        let result_str = result_json.to_string();
        assert!(
            result_str.contains("screenshot"),
            "result should contain screenshot field"
        );
        assert!(
            result_str.contains("data:image/png;base64,"),
            "result should contain image data URI"
        );
    } else {
        panic!("expected ToolCallEnd event with success and result");
    }
}

#[test]
fn test_extract_images_webp() {
    let payload = "B".repeat(300);
    let input = format!("data:image/webp;base64,{payload}");
    let (images, _remaining) = extract_images_from_text(&input);
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].media_type, "image/webp");
}

#[test]
fn test_extract_images_gif() {
    let payload = "C".repeat(300);
    let input = format!("data:image/gif;base64,{payload}");
    let (images, _remaining) = extract_images_from_text(&input);
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].media_type, "image/gif");
}

#[test]
fn test_extract_images_with_special_base64_chars() {
    let payload = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/==";
    let padded = format!("{}{}", payload, "A".repeat(200));
    let input = format!("data:image/png;base64,{padded}");
    let (images, _remaining) = extract_images_from_text(&input);
    assert_eq!(images.len(), 1);
    assert!(images[0].data.contains("+"));
    assert!(images[0].data.contains("/"));
}

#[test]
fn test_extract_images_preserves_surrounding_text() {
    let payload = "A".repeat(300);
    let input = format!(
        "Before the image\n\ndata:image/png;base64,{payload}\n\nAfter the image with special chars: <>&"
    );
    let (images, remaining) = extract_images_from_text(&input);
    assert_eq!(images.len(), 1);
    assert!(remaining.contains("Before the image"));
    assert!(remaining.contains("After the image with special chars: <>&"));
    assert!(!remaining.contains(&payload));
}

#[test]
fn test_extract_images_in_json_context() {
    let payload = "A".repeat(300);
    let input = format!(r#"{{"screenshot": "data:image/png;base64,{payload}", "success": true}}"#);
    let (images, remaining) = extract_images_from_text(&input);
    assert_eq!(images.len(), 1);
    assert!(remaining.contains("screenshot"));
    assert!(remaining.contains("success"));
    assert!(!remaining.contains(&payload));
}

#[test]
fn test_tool_result_to_content_openai_format() {
    let payload = "A".repeat(300);
    let input = format!("Text: data:image/png;base64,{payload}");
    let result = tool_result_to_content(&input, 50_000, true);
    let arr = result.as_array().unwrap();
    assert_eq!(arr[0]["type"], "text");
    assert!(arr[0]["text"].is_string());
    assert_eq!(arr[1]["type"], "image_url");
    assert!(arr[1]["image_url"].is_object());
    assert!(arr[1]["image_url"]["url"].is_string());
    let url = arr[1]["image_url"]["url"].as_str().unwrap();
    assert!(url.starts_with("data:image/png;base64,"));
}

#[test]
fn test_tool_result_to_content_truncation() {
    let payload = "A".repeat(300);
    let long_text = "X".repeat(10_000);
    let input = format!("{long_text} data:image/png;base64,{payload}");
    let result = tool_result_to_content(&input, 500, true);
    let arr = result.as_array().unwrap();
    let text = arr[0]["text"].as_str().unwrap();
    assert!(
        text.contains("[truncated"),
        "text should be truncated: {text}"
    );
    assert_eq!(arr[1]["type"], "image_url");
}

// ── sanitize_tool_name ────────────────────────────────────────────

#[test]
fn sanitize_tool_name_clean_input() {
    assert_eq!(sanitize_tool_name("exec"), "exec");
}

#[test]
fn sanitize_tool_name_trims_whitespace() {
    assert_eq!(sanitize_tool_name("  exec  "), "exec");
    assert_eq!(sanitize_tool_name("\texec\n"), "exec");
}

#[test]
fn sanitize_tool_name_strips_quotes() {
    assert_eq!(sanitize_tool_name("\"exec\""), "exec");
    assert_eq!(sanitize_tool_name("  \"web_search\"  "), "web_search");
}

#[test]
fn sanitize_tool_name_partial_quotes_unchanged() {
    assert_eq!(sanitize_tool_name("\"exec"), "\"exec");
    assert_eq!(sanitize_tool_name("exec\""), "exec\"");
}

#[test]
fn sanitize_tool_name_noop_on_real_tool_names() {
    let real_names = [
        "exec",
        "web_search",
        "web_fetch",
        "memory_save",
        "memory_forget",
        "memory_delete",
        "memory_search",
        "file_read",
        "file_write",
        "calc",
        "mcp-server_tool-name",
    ];
    for name in real_names {
        assert_eq!(
            sanitize_tool_name(name),
            name,
            "sanitize_tool_name must be no-op on valid tool name '{name}'"
        );
    }
}

#[test]
fn sanitize_tool_name_empty_string() {
    assert_eq!(sanitize_tool_name(""), "");
    assert_eq!(sanitize_tool_name("  "), "");
}

#[test]
fn sanitize_tool_name_only_quotes() {
    assert_eq!(sanitize_tool_name("\"\""), "");
}

#[test]
fn sanitize_tool_name_preserves_internal_quotes() {
    assert_eq!(sanitize_tool_name("my\"tool"), "my\"tool");
}

#[test]
fn sanitize_tool_name_single_quotes_not_stripped() {
    assert_eq!(sanitize_tool_name("'exec'"), "'exec'");
}

#[test]
fn sanitize_tool_name_strips_numeric_suffix() {
    assert_eq!(sanitize_tool_name("exec_2"), "exec");
    assert_eq!(sanitize_tool_name("browser_4"), "browser");
    assert_eq!(sanitize_tool_name("exec_123"), "exec");
}

#[test]
fn sanitize_tool_name_strips_functions_prefix() {
    assert_eq!(sanitize_tool_name("functions_spawn_agent"), "spawn_agent");
    assert_eq!(sanitize_tool_name("functions_exec"), "exec");
}

#[test]
fn sanitize_tool_name_strips_prefix_and_suffix() {
    assert_eq!(sanitize_tool_name("functions_spawn_agent_6"), "spawn_agent");
    assert_eq!(sanitize_tool_name("functions_exec_2"), "exec");
}

#[test]
fn sanitize_tool_name_preserves_legitimate_underscores() {
    assert_eq!(sanitize_tool_name("web_search"), "web_search");
    assert_eq!(sanitize_tool_name("memory_save"), "memory_save");
    assert_eq!(sanitize_tool_name("memory_forget"), "memory_forget");
    assert_eq!(sanitize_tool_name("memory_delete"), "memory_delete");
    assert_eq!(sanitize_tool_name("spawn_agent"), "spawn_agent");
    assert_eq!(sanitize_tool_name("get_user_location"), "get_user_location");
}

#[test]
fn sanitize_tool_name_preserves_mcp_names() {
    assert_eq!(
        sanitize_tool_name("mcp__ai__find-tasks"),
        "mcp__ai__find-tasks"
    );
    assert_eq!(
        sanitize_tool_name("mcp__jmap-mcp-0-1-1__get_emails"),
        "mcp__jmap-mcp-0-1-1__get_emails"
    );
    assert_eq!(
        sanitize_tool_name("mcp-server_tool-name"),
        "mcp-server_tool-name"
    );
}

#[test]
fn sanitize_tool_name_functions_prefix_alone_yields_empty() {
    assert_eq!(sanitize_tool_name("functions_"), "");
}

#[test]
fn legacy_public_tool_alias_strips_wasm_suffix() {
    assert_eq!(
        legacy_public_tool_alias("web_search_wasm"),
        Some("web_search")
    );
    assert_eq!(legacy_public_tool_alias("calc_wasm"), Some("calc"));
    assert_eq!(legacy_public_tool_alias("web_search"), None);
}

#[test]
fn resolve_tool_lookup_prefers_public_alias_when_both_exist() {
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(LargeResultTool {
        tool_name: "web_search",
        payload: "public".into(),
    }));
    tools.register_wasm(
        Box::new(LargeResultTool {
            tool_name: "web_search_wasm",
            payload: "legacy".into(),
        }),
        [0x11; 32],
    );

    let (tool, resolved_name) = resolve_tool_lookup(&tools, "web_search_wasm");
    let tool = tool.expect("resolved tool should exist");
    assert_eq!(resolved_name, "web_search");
    assert_eq!(tool.name(), "web_search");
}

#[test]
fn resolve_tool_lookup_falls_back_to_legacy_name_when_no_public_tool_exists() {
    let mut tools = ToolRegistry::new();
    tools.register_wasm(
        Box::new(LargeResultTool {
            tool_name: "web_search_wasm",
            payload: "legacy".into(),
        }),
        [0x22; 32],
    );

    let (tool, resolved_name) = resolve_tool_lookup(&tools, "web_search_wasm");
    let tool = tool.expect("legacy tool should exist");
    assert_eq!(resolved_name, "web_search_wasm");
    assert_eq!(tool.name(), "web_search_wasm");
}
