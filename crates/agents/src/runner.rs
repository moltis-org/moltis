use std::{fmt::Write, sync::Arc};

use {
    anyhow::{Result, bail},
    tracing::{debug, info, trace, warn},
};

#[cfg(feature = "metrics")]
use moltis_metrics::{counter, histogram, labels, llm as llm_metrics};

use moltis_common::hooks::{HookAction, HookPayload, HookRegistry};

use crate::{
    model::{ChatMessage, CompletionResponse, LlmProvider, StreamEvent, ToolCall, Usage},
    tool_registry::ToolRegistry,
};

use futures::StreamExt;

/// Maximum number of tool-call loop iterations before giving up.
const MAX_ITERATIONS: usize = 25;

/// Error patterns that indicate the context window has been exceeded.
const CONTEXT_WINDOW_PATTERNS: &[&str] = &[
    "context_length_exceeded",
    "max_tokens",
    "too many tokens",
    "request too large",
    "maximum context length",
    "context window",
    "token limit",
    "content_too_large",
    "request_too_large",
];

/// Check if an error message indicates a context window overflow.
fn is_context_window_error(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    CONTEXT_WINDOW_PATTERNS.iter().any(|p| lower.contains(p))
}

/// Typed errors from the agent loop.
#[derive(Debug, thiserror::Error)]
pub enum AgentRunError {
    /// The provider reported that the context window / token limit was exceeded.
    #[error("context window exceeded: {0}")]
    ContextWindowExceeded(String),
    /// Any other error.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Result of running the agent loop.
#[derive(Debug)]
pub struct AgentRunResult {
    pub text: String,
    pub iterations: usize,
    pub tool_calls_made: usize,
    pub usage: Usage,
}

/// Callback for streaming events out of the runner.
pub type OnEvent = Box<dyn Fn(RunnerEvent) + Send + Sync>;

/// Events emitted during the agent run.
#[derive(Debug, Clone)]
pub enum RunnerEvent {
    /// LLM is processing (show a "thinking" indicator).
    Thinking,
    /// LLM finished thinking (hide the indicator).
    ThinkingDone,
    ToolCallStart {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },
    ToolCallEnd {
        id: String,
        name: String,
        success: bool,
        error: Option<String>,
        result: Option<serde_json::Value>,
    },
    /// LLM returned reasoning/status text alongside tool calls.
    ThinkingText(String),
    TextDelta(String),
    Iteration(usize),
    SubAgentStart {
        task: String,
        model: String,
        depth: u64,
    },
    SubAgentEnd {
        task: String,
        model: String,
        depth: u64,
        iterations: usize,
        tool_calls_made: usize,
    },
}

/// Try to parse a tool call from the LLM's text response.
///
/// Providers without native tool-calling support are instructed (via the system
/// prompt) to emit a fenced block like:
///
/// ```tool_call
/// {"tool": "exec", "arguments": {"command": "ls"}}
/// ```
///
/// This function extracts that JSON and returns a synthetic `ToolCall` plus the
/// remaining text (if any) outside the fence.
fn parse_tool_call_from_text(text: &str) -> Option<(ToolCall, Option<String>)> {
    // Look for ```tool_call ... ``` blocks.
    let start_marker = "```tool_call";
    let start = text.find(start_marker)?;
    let after_marker = start + start_marker.len();
    let rest = &text[after_marker..];
    let end = rest.find("```")?;
    let json_str = rest[..end].trim();

    let parsed: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let tool_name = parsed["tool"].as_str()?.to_string();
    let arguments = parsed
        .get("arguments")
        .cloned()
        .unwrap_or(serde_json::json!({}));

    let id = format!("text-{}", uuid::Uuid::new_v4());

    // Collect any text outside the tool_call block.
    let before = text[..start].trim();
    let after_end = after_marker + end + 3; // skip closing ```
    let after = text.get(after_end..).unwrap_or("").trim();

    let remaining = if before.is_empty() && after.is_empty() {
        None
    } else if before.is_empty() {
        Some(after.to_string())
    } else if after.is_empty() {
        Some(before.to_string())
    } else {
        Some(format!("{before}\n{after}"))
    };

    Some((
        ToolCall {
            id,
            name: tool_name,
            arguments,
        },
        remaining,
    ))
}

// ── Tool result sanitization ────────────────────────────────────────────

/// Tag that starts a base64 data URI.
const BASE64_TAG: &str = "data:";
/// Marker between MIME type and base64 payload.
const BASE64_MARKER: &str = ";base64,";
/// Minimum length of a blob payload (base64 or hex) to be worth stripping.
const BLOB_MIN_LEN: usize = 200;

fn is_base64_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'='
}

/// Strip base64 data-URI blobs (e.g. `data:image/png;base64,AAAA...`) and
/// replace them with a short placeholder. Only targets payloads ≥ 200 chars.
fn strip_base64_blobs(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(start) = rest.find(BASE64_TAG) {
        result.push_str(&rest[..start]);
        let after_tag = &rest[start + BASE64_TAG.len()..];

        if let Some(marker_pos) = after_tag.find(BASE64_MARKER) {
            let mime_part = &after_tag[..marker_pos];
            let payload_start = marker_pos + BASE64_MARKER.len();
            let payload = &after_tag[payload_start..];
            let payload_len = payload.bytes().take_while(|b| is_base64_byte(*b)).count();

            if payload_len >= BLOB_MIN_LEN {
                let total_uri_len = BASE64_TAG.len() + payload_start + payload_len;
                // Provide a descriptive message based on MIME type
                if mime_part.starts_with("image/") {
                    result.push_str("[screenshot captured and displayed in UI]");
                } else {
                    write!(result, "[{mime_part} data removed — {total_uri_len} bytes]").unwrap();
                }
                rest = &rest[start + total_uri_len..];
                continue;
            }
        }

        result.push_str(BASE64_TAG);
        rest = after_tag;
    }
    result.push_str(rest);
    result
}

/// Strip long hex sequences (≥ 200 hex chars) that look like binary dumps.
fn strip_hex_blobs(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.char_indices().peekable();

    while let Some(&(start, ch)) = chars.peek() {
        if ch.is_ascii_hexdigit() {
            let mut end = start;
            while let Some(&(i, c)) = chars.peek() {
                if c.is_ascii_hexdigit() {
                    end = i + c.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            let run = end - start;
            if run >= BLOB_MIN_LEN {
                write!(result, "[hex data removed — {run} chars]").unwrap();
            } else {
                result.push_str(&input[start..end]);
            }
        } else {
            result.push(ch);
            chars.next();
        }
    }
    result
}

/// Sanitize a tool result string before feeding it to the LLM.
///
/// 1. Strips base64 data URIs (≥ 200 char payloads).
/// 2. Strips long hex sequences (≥ 200 hex chars).
/// 3. Truncates the result to `max_bytes` (at a char boundary), appending a
///    truncation marker.
pub fn sanitize_tool_result(input: &str, max_bytes: usize) -> String {
    let mut result = strip_base64_blobs(input);
    result = strip_hex_blobs(&result);

    if result.len() <= max_bytes {
        return result;
    }

    let original_len = result.len();
    let mut end = max_bytes;
    while end > 0 && !result.is_char_boundary(end) {
        end -= 1;
    }
    result.truncate(end);
    write!(result, "\n\n[truncated — {original_len} bytes total]").unwrap();
    result
}

// ── Multimodal tool result helpers ─────────────────────────────────────────

/// Image extracted from a tool result for multimodal handling.
#[derive(Debug)]
pub struct ExtractedImage {
    /// MIME type (e.g., "image/png", "image/jpeg")
    pub media_type: String,
    /// Base64-encoded image data
    pub data: String,
}

/// Extract image data URIs from text, returning the images and remaining text.
///
/// Searches for patterns like `data:image/png;base64,AAAA...` and extracts them.
/// Returns the list of images found and the text with images removed.
fn extract_images_from_text_impl(input: &str) -> (Vec<ExtractedImage>, String) {
    let mut images = Vec::new();
    let mut remaining = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(start) = rest.find(BASE64_TAG) {
        remaining.push_str(&rest[..start]);
        let after_tag = &rest[start + BASE64_TAG.len()..];

        // Check for image MIME type
        if let Some(marker_pos) = after_tag.find(BASE64_MARKER) {
            let mime_part = &after_tag[..marker_pos];

            // Only extract image/* MIME types
            if let Some(image_subtype) = mime_part.strip_prefix("image/") {
                let payload_start = marker_pos + BASE64_MARKER.len();
                let payload = &after_tag[payload_start..];
                let payload_len = payload.bytes().take_while(|b| is_base64_byte(*b)).count();

                if payload_len >= BLOB_MIN_LEN {
                    // Extract the image
                    let media_type = format!("image/{image_subtype}");
                    let data = payload[..payload_len].to_string();
                    images.push(ExtractedImage { media_type, data });

                    // Skip past the full data URI
                    let total_uri_len = BASE64_TAG.len() + payload_start + payload_len;
                    rest = &rest[start + total_uri_len..];
                    continue;
                }
            }
        }

        // Not an extractable image, keep the tag and continue
        remaining.push_str(BASE64_TAG);
        rest = after_tag;
    }
    remaining.push_str(rest);

    (images, remaining)
}

/// Test alias for extract_images_from_text_impl
#[cfg(test)]
fn extract_images_from_text(input: &str) -> (Vec<ExtractedImage>, String) {
    extract_images_from_text_impl(input)
}

/// Convert a tool result to multimodal content for vision-capable providers.
///
/// For providers with `supports_vision() == true`, this extracts images from
/// the tool result and returns them as OpenAI-style content blocks:
/// ```json
/// [
///   { "type": "text", "text": "..." },
///   { "type": "image_url", "image_url": { "url": "data:image/png;base64,..." } }
/// ]
/// ```
///
/// For non-vision providers, returns a simple string with images stripped.
///
/// Note: Browser screenshots are pre-stripped by the browser tool to avoid
/// the LLM outputting the raw base64 in its response (the UI already displays
/// screenshots via WebSocket events).
pub fn tool_result_to_content(
    result: &str,
    max_bytes: usize,
    supports_vision: bool,
) -> serde_json::Value {
    if !supports_vision {
        // Non-vision provider: strip images and return string
        return serde_json::Value::String(sanitize_tool_result(result, max_bytes));
    }

    // Vision provider: extract images and create multimodal content
    let (images, text) = extract_images_from_text_impl(result);

    if images.is_empty() {
        // No images found, just sanitize and return string
        return serde_json::Value::String(sanitize_tool_result(result, max_bytes));
    }

    // Build multimodal content array
    let mut content_blocks = Vec::new();

    // Sanitize remaining text (strips any remaining hex blobs, truncates if needed)
    let sanitized_text = sanitize_tool_result(&text, max_bytes);
    if !sanitized_text.trim().is_empty() {
        content_blocks.push(serde_json::json!({
            "type": "text",
            "text": sanitized_text
        }));
    }

    // Add image blocks
    for image in images {
        // Reconstruct data URI for OpenAI format
        let data_uri = format!("data:{};base64,{}", image.media_type, image.data);
        content_blocks.push(serde_json::json!({
            "type": "image_url",
            "image_url": { "url": data_uri }
        }));
    }

    serde_json::json!(content_blocks)
}

/// Run the agent loop: send messages to the LLM, execute tool calls, repeat.
///
/// If `history` is provided, those messages are inserted between the system
/// prompt and the current user message, giving the LLM conversational context.
pub async fn run_agent_loop(
    provider: Arc<dyn LlmProvider>,
    tools: &ToolRegistry,
    system_prompt: &str,
    user_message: &str,
    on_event: Option<&OnEvent>,
    history: Option<Vec<ChatMessage>>,
) -> Result<AgentRunResult, AgentRunError> {
    run_agent_loop_with_context(
        provider,
        tools,
        system_prompt,
        user_message,
        on_event,
        history,
        None,
        None,
    )
    .await
}

/// Like `run_agent_loop` but accepts optional context values that are injected
/// into every tool call's parameters (e.g. `_session_key`).
pub async fn run_agent_loop_with_context(
    provider: Arc<dyn LlmProvider>,
    tools: &ToolRegistry,
    system_prompt: &str,
    user_message: &str,
    on_event: Option<&OnEvent>,
    history: Option<Vec<ChatMessage>>,
    tool_context: Option<serde_json::Value>,
    hook_registry: Option<Arc<HookRegistry>>,
) -> Result<AgentRunResult, AgentRunError> {
    let native_tools = provider.supports_tools();
    let max_tool_result_bytes = moltis_config::discover_and_load()
        .tools
        .max_tool_result_bytes;
    let tool_schemas = tools.list_schemas();

    info!(
        provider = provider.name(),
        model = provider.id(),
        native_tools,
        tools_count = tool_schemas.len(),
        "starting agent loop"
    );

    let mut messages: Vec<ChatMessage> = vec![ChatMessage::system(system_prompt)];

    // Insert conversation history before the current user message.
    if let Some(hist) = history {
        messages.extend(hist);
    }

    messages.push(ChatMessage::user(user_message));

    // Only send tool schemas to providers that support them natively.
    let schemas_for_api = if native_tools {
        &tool_schemas
    } else {
        &vec![]
    };

    let mut iterations = 0;
    let mut total_tool_calls = 0;
    let mut total_input_tokens: u32 = 0;
    let mut total_output_tokens: u32 = 0;

    loop {
        iterations += 1;
        if iterations > MAX_ITERATIONS {
            warn!("agent loop exceeded max iterations ({})", MAX_ITERATIONS);
            return Err(AgentRunError::Other(anyhow::anyhow!(
                "agent loop exceeded max iterations"
            )));
        }

        if let Some(cb) = on_event {
            cb(RunnerEvent::Iteration(iterations));
        }

        info!(
            iteration = iterations,
            messages_count = messages.len(),
            "calling LLM"
        );
        trace!(iteration = iterations, messages = ?messages, "LLM request messages");

        if let Some(cb) = on_event {
            cb(RunnerEvent::Thinking);
        }

        let mut response: CompletionResponse = provider
            .complete(&messages, schemas_for_api)
            .await
            .map_err(|e| {
                if is_context_window_error(&e.to_string()) {
                    AgentRunError::ContextWindowExceeded(e.to_string())
                } else {
                    AgentRunError::Other(e)
                }
            })?;

        if let Some(cb) = on_event {
            cb(RunnerEvent::ThinkingDone);
        }

        total_input_tokens = total_input_tokens.saturating_add(response.usage.input_tokens);
        total_output_tokens = total_output_tokens.saturating_add(response.usage.output_tokens);

        info!(
            iteration = iterations,
            has_text = response.text.is_some(),
            tool_calls_count = response.tool_calls.len(),
            input_tokens = response.usage.input_tokens,
            output_tokens = response.usage.output_tokens,
            "LLM response received"
        );
        if let Some(ref text) = response.text {
            trace!(iteration = iterations, text = %text, "LLM response text");
        }

        // For providers without native tool calling, try parsing tool calls from text.
        if !native_tools
            && response.tool_calls.is_empty()
            && let Some(ref text) = response.text
            && let Some((tc, remaining_text)) = parse_tool_call_from_text(text)
        {
            info!(
                tool = %tc.name,
                "parsed tool call from text (non-native provider)"
            );
            response.text = remaining_text;
            response.tool_calls = vec![tc];
        }

        for tc in &response.tool_calls {
            info!(
                iteration = iterations,
                tool_name = %tc.name,
                arguments = %tc.arguments,
                "LLM requested tool call"
            );
        }

        // If no tool calls, return the text response.
        if response.tool_calls.is_empty() {
            let text = response.text.unwrap_or_default();

            info!(
                iterations,
                tool_calls = total_tool_calls,
                "agent loop complete — returning text"
            );
            return Ok(AgentRunResult {
                text,
                iterations,
                tool_calls_made: total_tool_calls,
                usage: Usage {
                    input_tokens: total_input_tokens,
                    output_tokens: total_output_tokens,
                    ..Default::default()
                },
            });
        }

        // Append assistant message with tool calls.
        if let Some(ref text) = response.text
            && let Some(cb) = on_event
        {
            cb(RunnerEvent::ThinkingText(text.clone()));
        }
        messages.push(ChatMessage::assistant_with_tools(
            response.text.clone(),
            response.tool_calls.clone(),
        ));

        // Extract session key from tool_context for hook payloads.
        let session_key = tool_context
            .as_ref()
            .and_then(|ctx| ctx.get("_session_key"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Execute tool calls concurrently.
        total_tool_calls += response.tool_calls.len();

        // Emit all ToolCallStart events first (preserves notification order).
        for tc in &response.tool_calls {
            if let Some(cb) = on_event {
                cb(RunnerEvent::ToolCallStart {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                });
            }
            info!(tool = %tc.name, id = %tc.id, args = %tc.arguments, "executing tool");
        }

        // Build futures for all tool calls (executed concurrently).
        let tool_futures: Vec<_> = response
            .tool_calls
            .iter()
            .map(|tc| {
                let tool = tools.get(&tc.name);
                let mut args = tc.arguments.clone();

                // Dispatch BeforeToolCall hook — may block or modify arguments.
                let hook_registry = hook_registry.clone();
                let session_key = session_key.clone();
                let tc_name = tc.name.clone();
                let _tc_id = tc.id.clone();

                if let Some(ref ctx) = tool_context
                    && let (Some(args_obj), Some(ctx_obj)) = (args.as_object_mut(), ctx.as_object())
                {
                    for (k, v) in ctx_obj {
                        args_obj.insert(k.clone(), v.clone());
                    }
                }
                async move {
                    // Run BeforeToolCall hook.
                    if let Some(ref hooks) = hook_registry {
                        let payload = HookPayload::BeforeToolCall {
                            session_key: session_key.clone(),
                            tool_name: tc_name.clone(),
                            arguments: args.clone(),
                        };
                        match hooks.dispatch(&payload).await {
                            Ok(HookAction::Block(reason)) => {
                                warn!(tool = %tc_name, reason = %reason, "tool call blocked by hook");
                                let err_str = format!("blocked by hook: {reason}");
                                return (
                                    false,
                                    serde_json::json!({ "error": err_str }),
                                    Some(err_str),
                                );
                            },
                            Ok(HookAction::ModifyPayload(v)) => {
                                args = v;
                            },
                            Ok(HookAction::Continue) => {},
                            Err(e) => {
                                warn!(tool = %tc_name, error = %e, "BeforeToolCall hook dispatch failed");
                            },
                        }
                    }

                    if let Some(tool) = tool {
                        match tool.execute(args).await {
                            Ok(val) => {
                                // Check if the result indicates a logical failure
                                // (e.g., BrowserResponse with success: false)
                                let has_error = val.get("error").is_some()
                                    || val.get("success") == Some(&serde_json::json!(false));
                                let error_msg = if has_error {
                                    val.get("error")
                                        .and_then(|e| e.as_str())
                                        .map(String::from)
                                } else {
                                    None
                                };

                                // Dispatch AfterToolCall hook.
                                if let Some(ref hooks) = hook_registry {
                                    let payload = HookPayload::AfterToolCall {
                                        session_key: session_key.clone(),
                                        tool_name: tc_name.clone(),
                                        success: !has_error,
                                        result: Some(val.clone()),
                                    };
                                    if let Err(e) = hooks.dispatch(&payload).await {
                                        warn!(tool = %tc_name, error = %e, "AfterToolCall hook dispatch failed");
                                    }
                                }

                                if has_error {
                                    // Tool executed but returned an error in the result
                                    (false, serde_json::json!({ "result": val }), error_msg)
                                } else {
                                    (true, serde_json::json!({ "result": val }), None)
                                }
                            },
                            Err(e) => {
                                let err_str = e.to_string();
                                // Dispatch AfterToolCall hook on failure.
                                if let Some(ref hooks) = hook_registry {
                                    let payload = HookPayload::AfterToolCall {
                                        session_key: session_key.clone(),
                                        tool_name: tc_name.clone(),
                                        success: false,
                                        result: None,
                                    };
                                    if let Err(e) = hooks.dispatch(&payload).await {
                                        warn!(tool = %tc_name, error = %e, "AfterToolCall hook dispatch failed");
                                    }
                                }
                                (
                                    false,
                                    serde_json::json!({ "error": err_str }),
                                    Some(err_str),
                                )
                            },
                        }
                    } else {
                        let err_str = format!("unknown tool: {tc_name}");
                        (
                            false,
                            serde_json::json!({ "error": err_str }),
                            Some(err_str),
                        )
                    }
                }
            })
            .collect();

        // Execute all tools concurrently and collect results in order.
        let results = futures::future::join_all(tool_futures).await;

        // Process results in original order: emit events, append messages.
        for (tc, (success, result, error)) in response.tool_calls.iter().zip(results) {
            if success {
                info!(tool = %tc.name, id = %tc.id, "tool execution succeeded");
                trace!(tool = %tc.name, result = %result, "tool result");
            } else {
                warn!(tool = %tc.name, id = %tc.id, error = %error.as_deref().unwrap_or(""), "tool execution failed");
            }

            if let Some(cb) = on_event {
                cb(RunnerEvent::ToolCallEnd {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    success,
                    error,
                    result: if success {
                        result.get("result").cloned()
                    } else {
                        None
                    },
                });
            }

            // Always sanitize tool results as strings - most LLM APIs don't support
            // multimodal content in tool results. Images are stripped but the UI
            // still receives them via ToolCallEnd event.
            let tool_result_str = sanitize_tool_result(&result.to_string(), max_tool_result_bytes);
            debug!(
                tool = %tc.name,
                id = %tc.id,
                result_len = tool_result_str.len(),
                "appending tool result to messages"
            );
            trace!(tool = %tc.name, content = %tool_result_str, "tool result message content");

            messages.push(ChatMessage::tool(&tc.id, &tool_result_str));
        }
    }
}

/// Convenience wrapper matching the old stub signature.
pub async fn run_agent(_agent_id: &str, _session_key: &str, _message: &str) -> Result<String> {
    bail!("run_agent requires a configured provider and tool registry; use run_agent_loop instead")
}

/// Streaming variant of the agent loop.
///
/// Unlike `run_agent_loop_with_context`, this function uses streaming to send
/// text deltas to the UI as they arrive, providing a much better UX.
///
/// Tool calls are accumulated from the stream and executed after the stream
/// completes, then the loop continues with the next iteration.
pub async fn run_agent_loop_streaming(
    provider: Arc<dyn LlmProvider>,
    tools: &ToolRegistry,
    system_prompt: &str,
    user_message: &str,
    on_event: Option<&OnEvent>,
    history: Option<Vec<ChatMessage>>,
    tool_context: Option<serde_json::Value>,
    hook_registry: Option<Arc<HookRegistry>>,
) -> Result<AgentRunResult, AgentRunError> {
    let native_tools = provider.supports_tools();
    let max_tool_result_bytes = moltis_config::discover_and_load()
        .tools
        .max_tool_result_bytes;
    let tool_schemas = tools.list_schemas();

    info!(
        provider = provider.name(),
        model = provider.id(),
        native_tools,
        tools_count = tool_schemas.len(),
        "starting streaming agent loop"
    );

    let mut messages: Vec<ChatMessage> = vec![ChatMessage::system(system_prompt)];

    // Insert conversation history before the current user message.
    if let Some(hist) = history {
        messages.extend(hist);
    }

    messages.push(ChatMessage::user(user_message));

    // Only send tool schemas to providers that support them natively.
    let schemas_for_api = if native_tools {
        tool_schemas.clone()
    } else {
        vec![]
    };

    info!(
        native_tools,
        schemas_for_api_count = schemas_for_api.len(),
        tool_schemas_count = tool_schemas.len(),
        "schemas_for_api prepared for streaming"
    );

    let mut iterations = 0;
    let mut total_tool_calls = 0;
    let mut total_input_tokens: u32 = 0;
    let mut total_output_tokens: u32 = 0;

    loop {
        iterations += 1;
        if iterations > MAX_ITERATIONS {
            warn!(
                "streaming agent loop exceeded max iterations ({})",
                MAX_ITERATIONS
            );
            return Err(AgentRunError::Other(anyhow::anyhow!(
                "agent loop exceeded max iterations"
            )));
        }

        if let Some(cb) = on_event {
            cb(RunnerEvent::Iteration(iterations));
        }

        info!(
            iteration = iterations,
            messages_count = messages.len(),
            "calling LLM (streaming)"
        );
        trace!(iteration = iterations, messages = ?messages, "LLM request messages");

        if let Some(cb) = on_event {
            cb(RunnerEvent::Thinking);
        }

        // Use streaming API.
        #[cfg(feature = "metrics")]
        let iter_start = std::time::Instant::now();
        let mut stream = provider.stream_with_tools(messages.clone(), schemas_for_api.clone());

        // Accumulate text and tool calls from the stream.
        let mut accumulated_text = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        // Map streaming index → accumulated JSON args string.
        let mut tool_call_args: std::collections::HashMap<usize, String> =
            std::collections::HashMap::new();
        // Map streaming index → position in the `tool_calls` vec.
        // The streaming index may not start at 0 (e.g. Copilot proxying
        // Anthropic uses the content-block index, so a text block at index 0
        // pushes the tool_use to index 1).
        let mut stream_idx_to_vec_pos: std::collections::HashMap<usize, usize> =
            std::collections::HashMap::new();
        let mut input_tokens: u32 = 0;
        let mut output_tokens: u32 = 0;
        let mut stream_error: Option<String> = None;

        while let Some(event) = stream.next().await {
            match event {
                StreamEvent::Delta(text) => {
                    accumulated_text.push_str(&text);
                    if let Some(cb) = on_event {
                        cb(RunnerEvent::TextDelta(text));
                    }
                },
                StreamEvent::ToolCallStart { id, name, index } => {
                    let vec_pos = tool_calls.len();
                    debug!(tool = %name, id = %id, stream_index = index, vec_pos, "tool call started in stream");
                    tool_calls.push(ToolCall {
                        id,
                        name,
                        arguments: serde_json::json!({}),
                    });
                    stream_idx_to_vec_pos.insert(index, vec_pos);
                    tool_call_args.insert(index, String::new());
                },
                StreamEvent::ToolCallArgumentsDelta { index, delta } => {
                    if let Some(args) = tool_call_args.get_mut(&index) {
                        args.push_str(&delta);
                    }
                },
                StreamEvent::ToolCallComplete { index } => {
                    // Arguments are finalized after stream completes.
                    // Just log for now - we'll parse accumulated args later.
                    debug!(index, "tool call arguments complete");
                },
                StreamEvent::Done(usage) => {
                    input_tokens = usage.input_tokens;
                    output_tokens = usage.output_tokens;
                    debug!(input_tokens, output_tokens, "stream done");

                    #[cfg(feature = "metrics")]
                    {
                        let provider_name = provider.name().to_string();
                        let model_id = provider.id().to_string();
                        let duration = iter_start.elapsed().as_secs_f64();
                        counter!(
                            llm_metrics::COMPLETIONS_TOTAL,
                            labels::PROVIDER => provider_name.clone(),
                            labels::MODEL => model_id.clone()
                        )
                        .increment(1);
                        counter!(
                            llm_metrics::INPUT_TOKENS_TOTAL,
                            labels::PROVIDER => provider_name.clone(),
                            labels::MODEL => model_id.clone()
                        )
                        .increment(u64::from(usage.input_tokens));
                        counter!(
                            llm_metrics::OUTPUT_TOKENS_TOTAL,
                            labels::PROVIDER => provider_name.clone(),
                            labels::MODEL => model_id.clone()
                        )
                        .increment(u64::from(usage.output_tokens));
                        counter!(
                            llm_metrics::CACHE_READ_TOKENS_TOTAL,
                            labels::PROVIDER => provider_name.clone(),
                            labels::MODEL => model_id.clone()
                        )
                        .increment(u64::from(usage.cache_read_tokens));
                        counter!(
                            llm_metrics::CACHE_WRITE_TOKENS_TOTAL,
                            labels::PROVIDER => provider_name.clone(),
                            labels::MODEL => model_id.clone()
                        )
                        .increment(u64::from(usage.cache_write_tokens));
                        histogram!(
                            llm_metrics::COMPLETION_DURATION_SECONDS,
                            labels::PROVIDER => provider_name,
                            labels::MODEL => model_id
                        )
                        .record(duration);
                    }
                },
                StreamEvent::Error(msg) => {
                    stream_error = Some(msg);
                    break;
                },
            }
        }

        if let Some(cb) = on_event {
            cb(RunnerEvent::ThinkingDone);
        }

        // Handle stream error.
        if let Some(err) = stream_error {
            if is_context_window_error(&err) {
                return Err(AgentRunError::ContextWindowExceeded(err));
            }
            return Err(AgentRunError::Other(anyhow::anyhow!(err)));
        }

        total_input_tokens = total_input_tokens.saturating_add(input_tokens);
        total_output_tokens = total_output_tokens.saturating_add(output_tokens);

        // Finalize tool call arguments from accumulated strings.
        // Use stream_idx_to_vec_pos to map streaming indices (which may not
        // start at 0) to the actual position in the tool_calls vec.
        for (stream_idx, args_str) in &tool_call_args {
            if let Some(&vec_pos) = stream_idx_to_vec_pos.get(stream_idx)
                && vec_pos < tool_calls.len()
                && !args_str.is_empty()
                && let Ok(args) = serde_json::from_str::<serde_json::Value>(args_str)
            {
                tool_calls[vec_pos].arguments = args;
            }
        }

        info!(
            iteration = iterations,
            has_text = !accumulated_text.is_empty(),
            tool_calls_count = tool_calls.len(),
            input_tokens,
            output_tokens,
            "streaming LLM response complete"
        );

        // For providers without native tool calling, try parsing tool calls from text.
        if !native_tools
            && tool_calls.is_empty()
            && !accumulated_text.is_empty()
            && let Some((tc, remaining_text)) = parse_tool_call_from_text(&accumulated_text)
        {
            info!(
                tool = %tc.name,
                "parsed tool call from text (non-native provider)"
            );
            accumulated_text = remaining_text.unwrap_or_default();
            tool_calls = vec![tc];
        }

        // If no tool calls, return the text response.
        if tool_calls.is_empty() {
            info!(
                iterations,
                tool_calls = total_tool_calls,
                "streaming agent loop complete — returning text"
            );
            return Ok(AgentRunResult {
                text: accumulated_text,
                iterations,
                tool_calls_made: total_tool_calls,
                usage: Usage {
                    input_tokens: total_input_tokens,
                    output_tokens: total_output_tokens,
                    ..Default::default()
                },
            });
        }

        // Append assistant message with tool calls.
        let text_for_msg = if accumulated_text.is_empty() {
            None
        } else {
            if let Some(cb) = on_event {
                cb(RunnerEvent::ThinkingText(accumulated_text.clone()));
            }
            Some(accumulated_text)
        };
        messages.push(ChatMessage::assistant_with_tools(
            text_for_msg,
            tool_calls.clone(),
        ));

        // Extract session key from tool_context for hook payloads.
        let session_key = tool_context
            .as_ref()
            .and_then(|ctx| ctx.get("_session_key"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Execute tool calls concurrently.
        total_tool_calls += tool_calls.len();

        // Emit all ToolCallStart events first (preserves notification order).
        for tc in &tool_calls {
            if let Some(cb) = on_event {
                cb(RunnerEvent::ToolCallStart {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                });
            }
            info!(tool = %tc.name, id = %tc.id, args = %tc.arguments, "executing tool");
        }

        // Build futures for all tool calls (executed concurrently).
        let tool_futures: Vec<_> = tool_calls
            .iter()
            .map(|tc| {
                let tool = tools.get(&tc.name);
                let mut args = tc.arguments.clone();

                let hook_registry = hook_registry.clone();
                let session_key = session_key.clone();
                let tc_name = tc.name.clone();

                if let Some(ref ctx) = tool_context
                    && let (Some(args_obj), Some(ctx_obj)) = (args.as_object_mut(), ctx.as_object())
                {
                    for (k, v) in ctx_obj {
                        args_obj.insert(k.clone(), v.clone());
                    }
                }
                async move {
                    // Run BeforeToolCall hook.
                    if let Some(ref hooks) = hook_registry {
                        let payload = HookPayload::BeforeToolCall {
                            session_key: session_key.clone(),
                            tool_name: tc_name.clone(),
                            arguments: args.clone(),
                        };
                        match hooks.dispatch(&payload).await {
                            Ok(HookAction::Block(reason)) => {
                                warn!(tool = %tc_name, reason = %reason, "tool call blocked by hook");
                                let err_str = format!("blocked by hook: {reason}");
                                return (
                                    false,
                                    serde_json::json!({ "error": err_str }),
                                    Some(err_str),
                                );
                            }
                            Ok(HookAction::ModifyPayload(v)) => {
                                args = v;
                            }
                            Ok(HookAction::Continue) => {}
                            Err(e) => {
                                warn!(tool = %tc_name, error = %e, "BeforeToolCall hook dispatch failed");
                            }
                        }
                    }

                    if let Some(tool) = tool {
                        match tool.execute(args).await {
                            Ok(val) => {
                                // Check if the result indicates a logical failure
                                // (e.g., BrowserResponse with success: false)
                                let has_error = val.get("error").is_some()
                                    || val.get("success") == Some(&serde_json::json!(false));
                                let error_msg = if has_error {
                                    val.get("error")
                                        .and_then(|e| e.as_str())
                                        .map(String::from)
                                } else {
                                    None
                                };

                                if let Some(ref hooks) = hook_registry {
                                    let payload = HookPayload::AfterToolCall {
                                        session_key: session_key.clone(),
                                        tool_name: tc_name.clone(),
                                        success: !has_error,
                                        result: Some(val.clone()),
                                    };
                                    if let Err(e) = hooks.dispatch(&payload).await {
                                        warn!(tool = %tc_name, error = %e, "AfterToolCall hook dispatch failed");
                                    }
                                }

                                if has_error {
                                    (false, serde_json::json!({ "result": val }), error_msg)
                                } else {
                                    (true, serde_json::json!({ "result": val }), None)
                                }
                            }
                            Err(e) => {
                                let err_str = e.to_string();
                                if let Some(ref hooks) = hook_registry {
                                    let payload = HookPayload::AfterToolCall {
                                        session_key: session_key.clone(),
                                        tool_name: tc_name.clone(),
                                        success: false,
                                        result: None,
                                    };
                                    if let Err(e) = hooks.dispatch(&payload).await {
                                        warn!(tool = %tc_name, error = %e, "AfterToolCall hook dispatch failed");
                                    }
                                }
                                (
                                    false,
                                    serde_json::json!({ "error": err_str }),
                                    Some(err_str),
                                )
                            }
                        }
                    } else {
                        let err_str = format!("unknown tool: {tc_name}");
                        (
                            false,
                            serde_json::json!({ "error": err_str }),
                            Some(err_str),
                        )
                    }
                }
            })
            .collect();

        // Execute all tools concurrently and collect results in order.
        let results = futures::future::join_all(tool_futures).await;

        // Process results in original order: emit events, append messages.
        for (tc, (success, result, error)) in tool_calls.iter().zip(results) {
            if success {
                info!(tool = %tc.name, id = %tc.id, "tool execution succeeded");
                trace!(tool = %tc.name, result = %result, "tool result");
            } else {
                warn!(tool = %tc.name, id = %tc.id, error = %error.as_deref().unwrap_or(""), "tool execution failed");
            }

            if let Some(cb) = on_event {
                cb(RunnerEvent::ToolCallEnd {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    success,
                    error,
                    result: if success {
                        result.get("result").cloned()
                    } else {
                        None
                    },
                });
            }

            // Always sanitize tool results as strings - most LLM APIs don't support
            // multimodal content in tool results. Images are stripped but the UI
            // still receives them via ToolCallEnd event.
            let tool_result_str = sanitize_tool_result(&result.to_string(), max_tool_result_bytes);
            debug!(
                tool = %tc.name,
                id = %tc.id,
                result_len = tool_result_str.len(),
                "appending tool result to messages"
            );
            trace!(tool = %tc.name, content = %tool_result_str, "tool result message content");

            messages.push(ChatMessage::tool(&tc.id, &tool_result_str));
        }
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::model::{
            ChatMessage, CompletionResponse, LlmProvider, StreamEvent, ToolCall, Usage,
        },
        async_trait::async_trait,
        std::pin::Pin,
        tokio_stream::Stream,
    };

    // ── parse_tool_call_from_text tests ──────────────────────────────

    #[test]
    fn test_parse_tool_call_basic() {
        let text = "```tool_call\n{\"tool\": \"exec\", \"arguments\": {\"command\": \"ls\"}}\n```";
        let (tc, remaining) = parse_tool_call_from_text(text).unwrap();
        assert_eq!(tc.name, "exec");
        assert_eq!(tc.arguments["command"], "ls");
        assert!(remaining.is_none());
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

    // ── Mock helpers ─────────────────────────────────────────────────

    /// A mock provider that returns text on the first call.
    struct MockProvider {
        response_text: String,
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
    struct ToolCallingProvider {
        call_count: std::sync::atomic::AtomicUsize,
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
    struct TextToolCallingProvider {
        call_count: std::sync::atomic::AtomicUsize,
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
                // Simulate an LLM emitting a tool_call block in text.
                Ok(CompletionResponse {
                    text: Some("```tool_call\n{\"tool\": \"exec\", \"arguments\": {\"command\": \"echo hello\"}}\n```".into()),
                    tool_calls: vec![],
                    usage: Usage { input_tokens: 10, output_tokens: 20, ..Default::default() },
                })
            } else {
                // Verify tool result was fed back.
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

    /// Simple echo tool for testing.
    struct EchoTool;

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

    /// A tool that actually runs shell commands (test-only, mirrors ExecTool).
    struct TestExecTool;

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

    // ── Tests ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_simple_text_response() {
        let provider = Arc::new(MockProvider {
            response_text: "Hello!".into(),
        });
        let tools = ToolRegistry::new();
        let result = run_agent_loop(provider, &tools, "You are a test bot.", "Hi", None, None)
            .await
            .unwrap();
        assert_eq!(result.text, "Hello!");
        assert_eq!(result.iterations, 1);
        assert_eq!(result.tool_calls_made, 0);
    }

    #[tokio::test]
    async fn test_tool_call_loop() {
        let provider = Arc::new(ToolCallingProvider {
            call_count: std::sync::atomic::AtomicUsize::new(0),
        });
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(EchoTool));

        let result = run_agent_loop(
            provider,
            &tools,
            "You are a test bot.",
            "Use the tool",
            None,
            None,
        )
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

        let result = run_agent_loop(
            provider,
            &tools,
            "You are a test bot.",
            "Run echo hello",
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

        let result = run_agent_loop(
            provider,
            &tools,
            "You are a test bot.",
            "Run echo hello",
            Some(&on_event),
            None,
        )
        .await
        .unwrap();

        assert!(result.text.contains("hello"), "got: {}", result.text);
        assert_eq!(result.iterations, 2, "should take 2 iterations");
        assert_eq!(result.tool_calls_made, 1, "should execute 1 tool call");

        // Verify tool events were emitted even for text-parsed calls.
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

    // ── Parallel tool execution tests ────────────────────────────────

    /// A tool that sleeps then returns its name.
    struct SlowTool {
        tool_name: String,
        delay_ms: u64,
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
    struct FailTool;

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

    /// Mock provider returning N tool calls on the first call, then text.
    struct MultiToolProvider {
        call_count: std::sync::atomic::AtomicUsize,
        tool_calls: Vec<ToolCall>,
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

    #[tokio::test]
    async fn test_parallel_tool_execution() {
        let provider = Arc::new(MultiToolProvider {
            call_count: std::sync::atomic::AtomicUsize::new(0),
            tool_calls: vec![
                ToolCall {
                    id: "c1".into(),
                    name: "tool_a".into(),
                    arguments: serde_json::json!({}),
                },
                ToolCall {
                    id: "c2".into(),
                    name: "tool_b".into(),
                    arguments: serde_json::json!({}),
                },
                ToolCall {
                    id: "c3".into(),
                    name: "tool_c".into(),
                    arguments: serde_json::json!({}),
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

        let result = run_agent_loop(
            provider,
            &tools,
            "Test bot",
            "Use all tools",
            Some(&on_event),
            None,
        )
        .await
        .unwrap();

        assert_eq!(result.text, "All done");
        assert_eq!(result.tool_calls_made, 3);

        // Verify all 3 ToolCallStart events come before any ToolCallEnd events.
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
                },
                ToolCall {
                    id: "c2".into(),
                    name: "fail_tool".into(),
                    arguments: serde_json::json!({}),
                },
                ToolCall {
                    id: "c3".into(),
                    name: "tool_c".into(),
                    arguments: serde_json::json!({}),
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

        let result = run_agent_loop(
            provider,
            &tools,
            "Test bot",
            "Use all tools",
            Some(&on_event),
            None,
        )
        .await
        .unwrap();

        assert_eq!(result.text, "All done");
        assert_eq!(result.tool_calls_made, 3);

        // Verify: 2 successes, 1 failure.
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
                },
                ToolCall {
                    id: "c2".into(),
                    name: "slow_b".into(),
                    arguments: serde_json::json!({}),
                },
                ToolCall {
                    id: "c3".into(),
                    name: "slow_c".into(),
                    arguments: serde_json::json!({}),
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
        let result = run_agent_loop(provider, &tools, "Test bot", "Use all tools", None, None)
            .await
            .unwrap();
        let elapsed = start.elapsed();

        assert_eq!(result.text, "All done");
        assert_eq!(result.tool_calls_made, 3);
        // If sequential, would take ≥300ms. Parallel should be ~100ms.
        assert!(
            elapsed < std::time::Duration::from_millis(250),
            "parallel execution took {:?}, expected < 250ms",
            elapsed
        );
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
        let input = "é".repeat(100); // 200 bytes
        let result = sanitize_tool_result(&input, 51); // mid-char
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
        // Image data URIs get a user-friendly message
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
        let hex = "a1b2c3d4".repeat(50); // 400 hex chars
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
        // Non-image data URIs are kept as-is
        assert!(remaining.contains("data:text/plain"));
    }

    #[test]
    fn test_extract_images_ignores_short_payload() {
        let payload = "QUFB"; // Short base64
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

        // Should strip the image for non-vision providers with user-friendly message
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

        // Should return array with text and image blocks
        assert!(result.is_array());
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 2);

        // Check text block
        assert_eq!(arr[0]["type"], "text");
        assert!(arr[0]["text"].as_str().unwrap().contains("Result:"));
        assert!(arr[0]["text"].as_str().unwrap().contains("done"));

        // Check image block
        assert_eq!(arr[1]["type"], "image_url");
        let url = arr[1]["image_url"]["url"].as_str().unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
        assert!(url.contains(&payload));
    }

    #[test]
    fn test_tool_result_to_content_vision_no_images() {
        let input = r#"{"result": "success", "message": "done"}"#;
        let result = tool_result_to_content(input, 50_000, true);

        // No images, should return plain string
        assert!(result.is_string());
        assert!(result.as_str().unwrap().contains("success"));
    }

    #[test]
    fn test_tool_result_to_content_vision_only_image() {
        let payload = "A".repeat(300);
        let input = format!("data:image/png;base64,{payload}");
        let result = tool_result_to_content(&input, 50_000, true);

        // Only image, no text - should return array with just image
        assert!(result.is_array());
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["type"], "image_url");
    }

    // ── Vision Provider Integration Tests ─────────────────────────────

    /// Mock provider that supports vision.
    struct VisionEnabledProvider {
        call_count: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl LlmProvider for VisionEnabledProvider {
        fn name(&self) -> &str {
            "mock-vision"
        }

        fn id(&self) -> &str {
            "gpt-4o" // Vision-capable model
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
                // First call: request a tool that returns an image
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
                // Second call: verify tool result was sanitized (image stripped)
                // because tool results don't support multimodal content
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

                // Tool result should be sanitized (image data replaced with user-friendly message)
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

    /// Tool that returns a result with an embedded screenshot
    struct ScreenshotTool;

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
            // Return a result with a base64 image data URI
            let fake_image_data = "A".repeat(500); // Long enough to be detected
            Ok(serde_json::json!({
                "success": true,
                "screenshot": format!("data:image/png;base64,{fake_image_data}"),
                "message": "Screenshot captured"
            }))
        }
    }

    #[tokio::test]
    async fn test_vision_provider_tool_result_sanitized() {
        // Even for vision-capable providers, tool results are sanitized
        // because most LLM APIs don't support multimodal content in tool results
        let provider = Arc::new(VisionEnabledProvider {
            call_count: std::sync::atomic::AtomicUsize::new(0),
        });
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(ScreenshotTool));

        let result = run_agent_loop(
            provider,
            &tools,
            "You are a test bot.",
            "Take a screenshot",
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(result.text, "Screenshot processed successfully");
        assert_eq!(result.tool_calls_made, 1);
    }

    #[tokio::test]
    async fn test_tool_call_end_event_contains_raw_result() {
        // Verify that ToolCallEnd events contain the raw (unsanitized) result
        // so the UI can display images even though they're stripped from LLM context
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

        let result = run_agent_loop(
            provider,
            &tools,
            "You are a test bot.",
            "Take a screenshot",
            Some(&on_event),
            None,
        )
        .await
        .unwrap();

        assert_eq!(result.tool_calls_made, 1);

        // Find the ToolCallEnd event
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
            // The raw result should contain the screenshot data
            let result_str = result_json.to_string();
            assert!(
                result_str.contains("screenshot"),
                "result should contain screenshot field"
            );
            // Note: the result contains the base64 data because it's raw
            assert!(
                result_str.contains("data:image/png;base64,"),
                "result should contain image data URI"
            );
        } else {
            panic!("expected ToolCallEnd event with success and result");
        }
    }

    // ── Extract Images Edge Cases ─────────────────────────────────────

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
        // Base64 can contain +, /, and = characters
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
        // Images often appear in JSON tool results
        let payload = "A".repeat(300);
        let input =
            format!(r#"{{"screenshot": "data:image/png;base64,{payload}", "success": true}}"#);
        let (images, remaining) = extract_images_from_text(&input);

        assert_eq!(images.len(), 1);
        assert!(remaining.contains("screenshot"));
        assert!(remaining.contains("success"));
        assert!(!remaining.contains(&payload));
    }

    // ── Tool Result Content Format Tests ──────────────────────────────

    #[test]
    fn test_tool_result_to_content_openai_format() {
        // Verify the OpenAI multimodal format is correct
        let payload = "A".repeat(300);
        let input = format!("Text: data:image/png;base64,{payload}");
        let result = tool_result_to_content(&input, 50_000, true);

        let arr = result.as_array().unwrap();
        // Check text block format
        assert_eq!(arr[0]["type"], "text");
        assert!(arr[0]["text"].is_string());

        // Check image block format matches OpenAI spec
        assert_eq!(arr[1]["type"], "image_url");
        assert!(arr[1]["image_url"].is_object());
        assert!(arr[1]["image_url"]["url"].is_string());
        let url = arr[1]["image_url"]["url"].as_str().unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn test_tool_result_to_content_truncation() {
        // Test that truncation works correctly with vision enabled
        let payload = "A".repeat(300);
        let long_text = "X".repeat(10_000);
        let input = format!("{long_text} data:image/png;base64,{payload}");

        // With small max_bytes, text should be truncated but image preserved
        let result = tool_result_to_content(&input, 500, true);

        let arr = result.as_array().unwrap();
        // Text should be truncated
        let text = arr[0]["text"].as_str().unwrap();
        assert!(
            text.contains("[truncated"),
            "text should be truncated: {text}"
        );

        // Image should still be present
        assert_eq!(arr[1]["type"], "image_url");
    }

    // ── Streaming tool-call index mapping tests ─────────────────────

    /// Mock streaming provider that emits text + a tool call at a non-zero
    /// streaming index. This simulates GitHub Copilot proxying Anthropic
    /// where the text content block is at index 0 and the tool_use block
    /// is at index 1.
    ///
    /// On the first call it streams text + tool call (index 1).
    /// On the second call it streams a final text response.
    struct NonZeroIndexStreamProvider {
        call_count: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl LlmProvider for NonZeroIndexStreamProvider {
        fn name(&self) -> &str {
            "mock-nonzero-idx"
        }

        fn id(&self) -> &str {
            "mock-nonzero-idx"
        }

        fn supports_tools(&self) -> bool {
            true
        }

        async fn complete(
            &self,
            _messages: &[ChatMessage],
            _tools: &[serde_json::Value],
        ) -> Result<CompletionResponse> {
            Ok(CompletionResponse {
                text: Some("fallback".into()),
                tool_calls: vec![],
                usage: Usage::default(),
            })
        }

        fn stream(
            &self,
            _messages: Vec<ChatMessage>,
        ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
            self.stream_with_tools(_messages, vec![])
        }

        fn stream_with_tools(
            &self,
            _messages: Vec<ChatMessage>,
            _tools: Vec<serde_json::Value>,
        ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
            let count = self
                .call_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if count == 0 {
                // First call: text block (implicit index 0) then tool call at index 1.
                Box::pin(tokio_stream::iter(vec![
                    StreamEvent::Delta("I'll create that for you.".into()),
                    StreamEvent::ToolCallStart {
                        id: "call_abc".into(),
                        name: "echo_tool".into(),
                        index: 1, // non-zero — the bug trigger
                    },
                    StreamEvent::ToolCallArgumentsDelta {
                        index: 1,
                        delta: r#"{"text""#.into(),
                    },
                    StreamEvent::ToolCallArgumentsDelta {
                        index: 1,
                        delta: r#": "hello"}"#.into(),
                    },
                    StreamEvent::ToolCallComplete { index: 1 },
                    StreamEvent::Done(Usage {
                        input_tokens: 10,
                        output_tokens: 5,
                        ..Default::default()
                    }),
                ]))
            } else {
                // Second call: just text.
                Box::pin(tokio_stream::iter(vec![
                    StreamEvent::Delta("Done!".into()),
                    StreamEvent::Done(Usage {
                        input_tokens: 5,
                        output_tokens: 3,
                        ..Default::default()
                    }),
                ]))
            }
        }
    }

    /// Regression test: when a streaming provider emits a tool call with a
    /// non-zero index (e.g. index 1 because index 0 is a text block), the
    /// runner must still correctly assemble the tool call arguments.
    ///
    /// Before the fix, the finalization code used the streaming index as the
    /// vec position directly: `tool_calls[1]` when `tool_calls.len() == 1`,
    /// silently dropping the arguments.
    #[tokio::test]
    async fn test_streaming_nonzero_tool_call_index_preserves_args() {
        let provider = Arc::new(NonZeroIndexStreamProvider {
            call_count: std::sync::atomic::AtomicUsize::new(0),
        });
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(EchoTool));

        let events: Arc<std::sync::Mutex<Vec<RunnerEvent>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);
        let on_event: OnEvent = Box::new(move |event| {
            events_clone.lock().unwrap().push(event);
        });

        let result = run_agent_loop_streaming(
            provider,
            &tools,
            "You are a test bot.",
            "Create something",
            Some(&on_event),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        // The tool should have been called with the correct arguments.
        assert_eq!(result.tool_calls_made, 1, "should execute 1 tool call");
        assert_eq!(result.iterations, 2, "tool call + final text");
        assert!(
            result.text.contains("Done!"),
            "final text should be 'Done!'"
        );

        // Verify the tool was actually invoked with the correct args, not {}.
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
        assert!(tool_start.is_some(), "should have a ToolCallStart event");
        let (name, args) = tool_start.unwrap();
        assert_eq!(name, "echo_tool");
        // The args in RunnerEvent::ToolCallStart should contain the parsed arguments.
        assert_eq!(
            args["text"].as_str(),
            Some("hello"),
            "tool call arguments must not be empty — got: {args}"
        );
    }

    /// Similar to the above, but with TWO tool calls at non-zero indices
    /// (e.g. index 2 and 4 with text blocks in between) to ensure the
    /// mapping handles multiple non-contiguous indices.
    struct MultiNonZeroIndexStreamProvider {
        call_count: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl LlmProvider for MultiNonZeroIndexStreamProvider {
        fn name(&self) -> &str {
            "mock-multi-nonzero"
        }

        fn id(&self) -> &str {
            "mock-multi-nonzero"
        }

        fn supports_tools(&self) -> bool {
            true
        }

        async fn complete(
            &self,
            _messages: &[ChatMessage],
            _tools: &[serde_json::Value],
        ) -> Result<CompletionResponse> {
            Ok(CompletionResponse {
                text: Some("fallback".into()),
                tool_calls: vec![],
                usage: Usage::default(),
            })
        }

        fn stream(
            &self,
            _messages: Vec<ChatMessage>,
        ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
            self.stream_with_tools(_messages, vec![])
        }

        fn stream_with_tools(
            &self,
            _messages: Vec<ChatMessage>,
            _tools: Vec<serde_json::Value>,
        ) -> Pin<Box<dyn Stream<Item = StreamEvent> + Send + '_>> {
            let count = self
                .call_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if count == 0 {
                // Two tool calls with gaps: indices 1 and 3 (text at 0 and 2).
                Box::pin(tokio_stream::iter(vec![
                    StreamEvent::Delta("Starting...".into()),
                    StreamEvent::ToolCallStart {
                        id: "call_1".into(),
                        name: "echo_tool".into(),
                        index: 1,
                    },
                    StreamEvent::ToolCallArgumentsDelta {
                        index: 1,
                        delta: r#"{"text": "first"}"#.into(),
                    },
                    StreamEvent::ToolCallComplete { index: 1 },
                    StreamEvent::ToolCallStart {
                        id: "call_2".into(),
                        name: "echo_tool".into(),
                        index: 3,
                    },
                    StreamEvent::ToolCallArgumentsDelta {
                        index: 3,
                        delta: r#"{"text": "second"}"#.into(),
                    },
                    StreamEvent::ToolCallComplete { index: 3 },
                    StreamEvent::Done(Usage {
                        input_tokens: 15,
                        output_tokens: 10,
                        ..Default::default()
                    }),
                ]))
            } else {
                Box::pin(tokio_stream::iter(vec![
                    StreamEvent::Delta("All done!".into()),
                    StreamEvent::Done(Usage {
                        input_tokens: 5,
                        output_tokens: 3,
                        ..Default::default()
                    }),
                ]))
            }
        }
    }

    #[tokio::test]
    async fn test_streaming_multiple_nonzero_indices() {
        let provider = Arc::new(MultiNonZeroIndexStreamProvider {
            call_count: std::sync::atomic::AtomicUsize::new(0),
        });
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(EchoTool));

        let events: Arc<std::sync::Mutex<Vec<RunnerEvent>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);
        let on_event: OnEvent = Box::new(move |event| {
            events_clone.lock().unwrap().push(event);
        });

        let result = run_agent_loop_streaming(
            provider,
            &tools,
            "You are a test bot.",
            "Do two things",
            Some(&on_event),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(result.tool_calls_made, 2, "should execute 2 tool calls");
        assert!(result.text.contains("All done!"));

        // Verify both tool calls had correct arguments.
        let evts = events.lock().unwrap();
        let tool_starts: Vec<_> = evts
            .iter()
            .filter_map(|e| {
                if let RunnerEvent::ToolCallStart { arguments, .. } = e {
                    Some(arguments.clone())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(tool_starts.len(), 2, "should have 2 ToolCallStart events");
        assert_eq!(
            tool_starts[0]["text"].as_str(),
            Some("first"),
            "first tool call args — got: {}",
            tool_starts[0]
        );
        assert_eq!(
            tool_starts[1]["text"].as_str(),
            Some("second"),
            "second tool call args — got: {}",
            tool_starts[1]
        );
    }
}
