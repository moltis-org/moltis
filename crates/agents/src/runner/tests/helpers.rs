#![allow(dead_code, unused_imports)]

//! Shared mock providers, tools, and test helpers for runner tests.

use {
    super::super::*,
    crate::model::{ChatMessage, CompletionResponse, LlmProvider, StreamEvent, ToolCall, Usage},
    anyhow::{Result, bail},
    async_trait::async_trait,
    moltis_common::hooks::{HookAction, HookEvent, HookHandler, HookPayload, HookRegistry},
    std::{pin::Pin, sync::Arc},
    tokio_stream::Stream,
};

// Re-export commonly used items for test submodules.
pub(super) use {
    super::super::{
        AgentRunError, AgentRunResult, OnEvent, RunnerEvent, TOOL_RESULT_COMPACTION_PLACEHOLDER,
        compact_tool_results_oldest_first_in_place, enforce_tool_result_context_budget,
        explicit_shell_command_from_user_content,
        is_substantive_answer_text, legacy_public_tool_alias, resolve_tool_lookup,
        retry::*,
        run_agent_loop, run_agent_loop_with_context, sanitize_tool_name, sanitize_tool_result,
        streaming::run_agent_loop_streaming,
        tool_result::{ExtractedImage, extract_images_from_text},
        tool_result_to_content,
    },
    crate::{model::UserContent, tool_registry::ToolRegistry},
};

pub(super) use crate::tool_parsing::parse_tool_call_from_text;

// ── Recording hook ──────────────────────────────────────────────────────

pub(super) struct RecordingHook {
    pub payloads: Arc<std::sync::Mutex<Vec<HookPayload>>>,
}

#[async_trait]
impl HookHandler for RecordingHook {
    fn name(&self) -> &str {
        "recording-hook"
    }

    fn events(&self) -> &[HookEvent] {
        static EVENTS: [HookEvent; 2] = [HookEvent::BeforeToolCall, HookEvent::AfterToolCall];
        &EVENTS
    }

    async fn handle(
        &self,
        _event: HookEvent,
        payload: &HookPayload,
    ) -> moltis_common::error::Result<HookAction> {
        self.payloads.lock().unwrap().push(payload.clone());
        Ok(HookAction::Continue)
    }
}

// ── Mock providers ──────────────────────────────────────────────────────

/// A mock provider that returns text on the first call.
pub(super) struct MockProvider {
    pub response_text: String,
}

#[async_trait]
impl LlmProvider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }

    fn id(&self) -> &str {
        "mock-model"
    }

    async fn complete(
        &self,
        _messages: &[ChatMessage],
        _tools: &[serde_json::Value],
    ) -> Result<CompletionResponse> {
        Ok(CompletionResponse {
            text: Some(self.response_text.clone()),
            tool_calls: vec![],
            usage: Usage {
                input_tokens: 10,
                output_tokens: 5,
                ..Default::default()
            },
        })
    }

    fn stream(
        &self,
        _messages: Vec<ChatMessage>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        Box::pin(tokio_stream::empty())
    }
}

/// Mock provider that makes one tool call then returns text (native tool support).
pub(super) struct ToolCallingProvider {
    pub call_count: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl LlmProvider for ToolCallingProvider {
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
                    id: "call_1".into(),
                    name: "echo_tool".into(),
                    arguments: serde_json::json!({"text": "hi"}),
                }],
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    ..Default::default()
                },
            })
        } else {
            Ok(CompletionResponse {
                text: Some("Done!".into()),
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

/// Non-native provider that returns tool calls as text blocks.
pub(super) struct TextToolCallingProvider {
    pub call_count: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl LlmProvider for TextToolCallingProvider {
    fn name(&self) -> &str {
        "mock-no-native"
    }

    fn id(&self) -> &str {
        "mock-no-native"
    }

    fn supports_tools(&self) -> bool {
        false
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
                text: Some("```tool_call\n{\"tool\": \"exec\", \"arguments\": {\"command\": \"echo hello\"}}\n```".into()),
                tool_calls: vec![],
                usage: Usage { input_tokens: 10, output_tokens: 20, ..Default::default() },
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
                tool_content.contains("hello"),
                "tool result should contain 'hello', got: {tool_content}"
            );
            Ok(CompletionResponse {
                text: Some("The command output: hello".into()),
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

// ── Mock tools ──────────────────────────────────────────────────────────

/// Simple echo tool for testing.
pub(super) struct EchoTool;

#[async_trait]
impl crate::tool_registry::AgentTool for EchoTool {
    fn name(&self) -> &str {
        "echo_tool"
    }

    fn description(&self) -> &str {
        "Echoes input"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {"text": {"type": "string"}}})
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        Ok(params)
    }
}

pub(super) struct LargeResultTool {
    pub tool_name: &'static str,
    pub payload: String,
}

#[async_trait]
impl crate::tool_registry::AgentTool for LargeResultTool {
    fn name(&self) -> &str {
        self.tool_name
    }

    fn description(&self) -> &str {
        "Returns a large payload"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
        })
    }

    async fn execute(&self, _params: serde_json::Value) -> Result<serde_json::Value> {
        Ok(serde_json::json!({
            "stdout": self.payload,
        }))
    }
}

/// A tool that actually runs shell commands (test-only, mirrors ExecTool).
pub(super) struct TestExecTool;

#[async_trait]
impl crate::tool_registry::AgentTool for TestExecTool {
    fn name(&self) -> &str {
        "exec"
    }

    fn description(&self) -> &str {
        "Execute a shell command"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command to execute" }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        let command = params["command"].as_str().unwrap_or("echo noop");
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
            .await?;
        Ok(serde_json::json!({
            "stdout": String::from_utf8_lossy(&output.stdout).to_string(),
            "stderr": String::from_utf8_lossy(&output.stderr).to_string(),
            "exit_code": output.status.code().unwrap_or(-1),
        }))
    }
}

/// Minimal process tool for testing `<function=process>` parsing.
pub(super) struct TestProcessTool;

#[async_trait]
impl crate::tool_registry::AgentTool for TestProcessTool {
    fn name(&self) -> &str {
        "process"
    }

    fn description(&self) -> &str {
        "Process tool for tests"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": { "type": "string" },
                "command": { "type": "string" }
            }
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        Ok(serde_json::json!({
            "received": params,
        }))
    }
}

/// A tool that sleeps then returns its name.
pub(super) struct SlowTool {
    pub tool_name: String,
    pub delay_ms: u64,
}

#[async_trait]
impl crate::tool_registry::AgentTool for SlowTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        "Slow tool for testing"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {}})
    }

    async fn execute(&self, _params: serde_json::Value) -> Result<serde_json::Value> {
        tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        Ok(serde_json::json!({ "tool": self.tool_name }))
    }
}

/// A tool that always fails.
pub(super) struct FailTool;

#[async_trait]
impl crate::tool_registry::AgentTool for FailTool {
    fn name(&self) -> &str {
        "fail_tool"
    }

    fn description(&self) -> &str {
        "Always fails"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {}})
    }

    async fn execute(&self, _params: serde_json::Value) -> Result<serde_json::Value> {
        anyhow::bail!("intentional failure")
    }
}

/// Tool that returns a result with an embedded screenshot
pub(super) struct ScreenshotTool;

#[async_trait]
impl crate::tool_registry::AgentTool for ScreenshotTool {
    fn name(&self) -> &str {
        "screenshot_tool"
    }

    fn description(&self) -> &str {
        "Takes a screenshot and returns it as base64"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {}})
    }

    async fn execute(&self, _params: serde_json::Value) -> Result<serde_json::Value> {
        let fake_image_data = "A".repeat(500);
        Ok(serde_json::json!({
            "success": true,
            "screenshot": format!("data:image/png;base64,{fake_image_data}"),
            "message": "Screenshot captured"
        }))
    }
}

// ── More mock providers ─────────────────────────────────────────────────

/// Mock provider returning N tool calls on the first call, then text.
pub(super) struct MultiToolProvider {
    pub call_count: std::sync::atomic::AtomicUsize,
    pub tool_calls: Vec<ToolCall>,
}

#[async_trait]
impl LlmProvider for MultiToolProvider {
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
        _messages: &[ChatMessage],
        _tools: &[serde_json::Value],
    ) -> Result<CompletionResponse> {
        let count = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if count == 0 {
            Ok(CompletionResponse {
                text: None,
                tool_calls: self.tool_calls.clone(),
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    ..Default::default()
                },
            })
        } else {
            Ok(CompletionResponse {
                text: Some("All done".into()),
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

pub(super) struct PreemptiveOverflowProvider {
    pub call_count: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl LlmProvider for PreemptiveOverflowProvider {
    fn name(&self) -> &str {
        "mock-overflow"
    }

    fn id(&self) -> &str {
        "mock-overflow-model"
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn context_window(&self) -> u32 {
        120
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
                text: Some("reasoning ".repeat(80)),
                tool_calls: vec![ToolCall {
                    id: "overflow_call".into(),
                    name: "overflow_tool".into(),
                    arguments: serde_json::json!({}),
                }],
                usage: Usage::default(),
            })
        } else {
            bail!("second provider call should not happen")
        }
    }

    fn stream(
        &self,
        _messages: Vec<ChatMessage>,
    ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
        Box::pin(tokio_stream::empty())
    }
}

/// Mock provider that supports vision.
pub(super) struct VisionEnabledProvider {
    pub call_count: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl LlmProvider for VisionEnabledProvider {
    fn name(&self) -> &str {
        "mock-vision"
    }

    fn id(&self) -> &str {
        "gpt-4o"
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn supports_vision(&self) -> bool {
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
                    id: "call_screenshot".into(),
                    name: "screenshot_tool".into(),
                    arguments: serde_json::json!({}),
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
            assert!(
                tool_content.contains("[screenshot captured and displayed in UI]"),
                "tool result should have image stripped: {tool_content}"
            );
            assert!(
                !tool_content.contains("AAAA"),
                "tool result should not contain raw base64: {tool_content}"
            );
            Ok(CompletionResponse {
                text: Some("Screenshot processed successfully".into()),
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

// ── Shared test utility functions ───────────────────────────────────────

pub(super) fn last_user_text(messages: &[ChatMessage]) -> &str {
    messages
        .iter()
        .rev()
        .find_map(|message| match message {
            ChatMessage::User {
                content: UserContent::Text(text),
                ..
            } => Some(text.as_str()),
            _ => None,
        })
        .unwrap_or("")
}

pub(super) fn last_tool_text(messages: &[ChatMessage]) -> &str {
    messages
        .iter()
        .rev()
        .find_map(|message| match message {
            ChatMessage::Tool { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .unwrap_or("")
}

pub(super) fn has_tool_message_containing(messages: &[ChatMessage], needle: &str) -> bool {
    messages.iter().any(|message| match message {
        ChatMessage::Tool { content, .. } => content.contains(needle),
        _ => false,
    })
}

/// Long text emitted after a few tool calls should be returned verbatim
/// without auto-continue firing (GH #628).
pub(super) const GH_628_LONG_ANSWER: &str = "Your volume is still down compared to last week. \
    Squat volume dropped from 12 sets to 8 sets, bench press held steady at 10 sets, \
    and deadlift volume fell from 6 to 4 sets. Consider adding an accessory day to \
    recover weekly tonnage before the next overload block.";

pub(super) fn history_contains_intervention(messages: &[ChatMessage]) -> bool {
    messages.iter().any(|m| match m {
        ChatMessage::User {
            content: UserContent::Text(text),
            ..
        } => text.contains("LOOP DETECTED") || text.contains("TOOLS DISABLED"),
        _ => false,
    })
}
