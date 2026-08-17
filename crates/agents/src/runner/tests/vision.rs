//! Vision and tool-result image handling tests.

use std::{pin::Pin, sync::Arc};

use {
    super::helpers::*,
    crate::model::{ChatMessage, CompletionResponse, LlmProvider, StreamEvent, Usage},
    anyhow::Result,
    async_trait::async_trait,
    tokio_stream::Stream,
};

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
fn test_extract_images_in_json_context() {
    let payload = "A".repeat(300);
    let input = format!(r#"{{"screenshot": "data:image/png;base64,{payload}", "success": true}}"#);
    let (images, remaining) = extract_images_from_text(&input);
    assert_eq!(images.len(), 1);
    assert!(remaining.contains("screenshot"));
    assert!(remaining.contains("success"));
    assert!(!remaining.contains(&payload));
}
