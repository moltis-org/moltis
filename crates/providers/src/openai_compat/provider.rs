//! Shared helpers for OpenAI-compatible streaming with tools.
//!
//! This module provides reusable functions for parsing OpenAI-style SSE streams
//! that include tool calls. Used by openai.rs, github_copilot.rs, and kimi_code.rs.

use std::collections::{HashMap, HashSet};

use {
    moltis_agents::model::{
        ChatMessage, CompletionResponse, StreamEvent, ToolCall, Usage, UserContent,
        decode_tool_call_arguments, extract_tool_call_metadata,
    },
    serde::Serialize,
    tracing::trace,
};

use super::{
    schema_normalization::sanitize_schema_for_openai_compat,
    strict_mode::patch_schema_for_strict_mode,
};

// ============================================================================
// OpenAI Tool Schema Types
// ============================================================================
// These types enforce the correct structure for OpenAI-compatible APIs.
// Using typed structs instead of manual JSON prevents missing fields at compile time.
//
// References:
// - Chat Completions: https://platform.openai.com/docs/guides/function-calling
// - Responses API: https://learn.microsoft.com/en-us/azure/ai-foundry/openai/how-to/responses
// ============================================================================

/// Chat Completions API tool format (nested under "function").
///
/// ```json
/// { "type": "function", "function": { "name": "...", ... } }
/// ```
#[derive(Debug, Serialize)]
pub struct ChatCompletionsTool {
    #[serde(rename = "type")]
    pub tool_type: &'static str,
    pub function: ChatCompletionsFunction,
}

/// The function definition nested inside ChatCompletionsTool.
#[derive(Debug, Serialize)]
pub struct ChatCompletionsFunction {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub strict: bool,
}

/// Responses API tool format (flat, name at top level).
///
/// ```json
/// { "type": "function", "name": "...", "parameters": {...}, "strict": true }
/// ```
#[derive(Debug, Serialize)]
pub struct ResponsesApiTool {
    #[serde(rename = "type")]
    pub tool_type: &'static str,
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub strict: bool,
}

/// Convert tool schemas to OpenAI Chat Completions function-calling format.
///
/// Uses the nested `function` object format required by Chat Completions API:
/// ```json
/// { "type": "function", "function": { "name": "...", ... } }
/// ```
///
/// When `strict` is `true`, patches schemas for strict mode compliance:
/// - `additionalProperties: false` on all object schemas
/// - All properties included in `required` array
/// - Originally-optional properties made nullable via array-form types
///
/// When `strict` is `false`, schemas are sanitized but not patched for strict
/// mode. This avoids array-form types like `["boolean", "null"]` that
/// non-OpenAI backends (e.g. Google via OpenRouter) reject.
///
/// See: <https://platform.openai.com/docs/guides/function-calling>
pub fn to_openai_tools(tools: &[serde_json::Value], strict: bool) -> Vec<serde_json::Value> {
    let result: Vec<serde_json::Value> = tools
        .iter()
        .filter_map(|t| {
            let mut params = t["parameters"].clone();
            sanitize_schema_for_openai_compat(&mut params);
            if strict {
                patch_schema_for_strict_mode(&mut params);
            }

            let name = t["name"].as_str()?.to_string();
            let description = t["description"].as_str().unwrap_or("").to_string();

            // Use typed struct to ensure all required fields are present
            let tool = ChatCompletionsTool {
                tool_type: "function",
                function: ChatCompletionsFunction {
                    name: name.clone(),
                    description,
                    parameters: params,
                    strict,
                },
            };

            trace!(tool_name = %name, strict, "converted tool to Chat Completions format");

            // Serialize to Value for compatibility with existing API
            serde_json::to_value(tool).ok()
        })
        .collect();

    trace!(tools_count = result.len(), "to_openai_tools complete");
    result
}

/// Convert tool schemas to OpenAI Responses API function-calling format.
///
/// Uses the flat format required by the Responses API where `name` is at top level:
/// ```json
/// { "type": "function", "name": "...", "parameters": {...}, "strict": true }
/// ```
///
/// This is the format used by OpenAI Codex and the Responses API.
///
/// Patches schemas for strict mode compliance:
/// - `additionalProperties: false` on all object schemas
/// - All properties included in `required` array
///
/// See: <https://learn.microsoft.com/en-us/azure/ai-foundry/openai/how-to/responses>
pub fn to_responses_api_tools(tools: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let result: Vec<serde_json::Value> = tools
        .iter()
        .filter_map(|t| {
            // Clone parameters and patch for strict mode
            let mut params = t["parameters"].clone();
            sanitize_schema_for_openai_compat(&mut params);
            patch_schema_for_strict_mode(&mut params);

            let name = t["name"].as_str()?.to_string();
            let description = t["description"].as_str().unwrap_or("").to_string();

            // Use typed struct to ensure all required fields are present
            let tool = ResponsesApiTool {
                tool_type: "function",
                name: name.clone(),
                description,
                parameters: params,
                strict: true,
            };

            trace!(tool_name = %name, "converted tool to Responses API format");

            // Serialize to Value for compatibility with existing API
            serde_json::to_value(tool).ok()
        })
        .collect();

    trace!(
        tools_count = result.len(),
        "to_responses_api_tools complete"
    );
    result
}

/// Convert typed chat messages to Responses API input items.
///
/// Responses API accepts a heterogeneous input array (messages, tool calls, and
/// tool outputs). This keeps one canonical conversion for providers that use
/// Responses transport (SSE or WebSocket).
#[must_use]
pub fn to_responses_input(messages: &[ChatMessage]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .flat_map(|msg| match msg {
            ChatMessage::System { .. } => {
                // System messages are extracted into `instructions`.
                vec![]
            },
            ChatMessage::User { content, .. } => {
                let content_blocks = match content {
                    UserContent::Text(t) => {
                        vec![serde_json::json!({"type": "input_text", "text": t})]
                    },
                    UserContent::Multimodal(parts) => parts
                        .iter()
                        .map(|p| match p {
                            moltis_agents::model::ContentPart::Text(t) => {
                                serde_json::json!({"type": "input_text", "text": t})
                            },
                            moltis_agents::model::ContentPart::Image { media_type, data } => {
                                let data_uri = format!("data:{media_type};base64,{data}");
                                serde_json::json!({
                                    "type": "input_image",
                                    "image_url": data_uri,
                                })
                            },
                        })
                        .collect(),
                };
                vec![serde_json::json!({
                    "role": "user",
                    "content": content_blocks,
                })]
            },
            ChatMessage::Assistant {
                content,
                tool_calls,
            } => {
                if !tool_calls.is_empty() {
                    let mut items: Vec<serde_json::Value> = tool_calls
                        .iter()
                        .map(|tc| {
                            serde_json::json!({
                                "type": "function_call",
                                "call_id": tc.id,
                                "name": tc.name,
                                "arguments": tc.arguments.to_string(),
                            })
                        })
                        .collect();
                    if let Some(text) = content
                        && !text.is_empty()
                    {
                        items.insert(
                            0,
                            serde_json::json!({
                                "type": "message",
                                "role": "assistant",
                                "content": [{"type": "output_text", "text": text}]
                            }),
                        );
                    }
                    items
                } else {
                    let text = content.as_deref().unwrap_or("");
                    vec![serde_json::json!({
                        "type": "message",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": text}]
                    })]
                }
            },
            ChatMessage::Tool {
                tool_call_id,
                content,
            } => {
                vec![serde_json::json!({
                    "type": "function_call_output",
                    "call_id": tool_call_id,
                    "output": content,
                })]
            },
        })
        .collect()
}

/// Parse tool_calls from an OpenAI response message (non-streaming).
pub fn parse_tool_calls(message: &serde_json::Value) -> Vec<ToolCall> {
    message["tool_calls"]
        .as_array()
        .map(|tcs| {
            tcs.iter()
                .filter_map(|tc| {
                    let id = tc["id"].as_str()?.to_string();
                    let name = tc["function"]["name"].as_str()?.to_string();
                    let arguments = decode_tool_call_arguments(tc["function"].get("arguments"));
                    let metadata = extract_tool_call_metadata(tc);
                    Some(ToolCall {
                        id,
                        name,
                        arguments,
                        metadata,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn usage_value_at_path(usage: &serde_json::Value, path: &[&str]) -> Option<u64> {
    let mut cursor = usage;
    for key in path {
        cursor = cursor.get(*key)?;
    }
    cursor
        .as_u64()
        .or_else(|| cursor.as_str().and_then(|raw| raw.parse::<u64>().ok()))
}

fn usage_field_u32(usage: &serde_json::Value, paths: &[&[&str]]) -> u32 {
    paths
        .iter()
        .find_map(|path| usage_value_at_path(usage, path))
        .unwrap_or(0) as u32
}

fn usage_object_from_payload(payload: &serde_json::Value) -> Option<&serde_json::Value> {
    if let Some(usage) = payload.get("usage").filter(|usage| usage.is_object()) {
        return Some(usage);
    }

    if let Some(usage) = payload
        .get("choices")
        .and_then(serde_json::Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("usage"))
        .filter(|usage| usage.is_object())
    {
        return Some(usage);
    }

    if let Some(usage) = payload
        .get("choices")
        .and_then(serde_json::Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("delta"))
        .and_then(|delta| delta.get("usage"))
        .filter(|usage| usage.is_object())
    {
        return Some(usage);
    }

    if let Some(usage) = payload
        .get("choices")
        .and_then(serde_json::Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("usage"))
        .filter(|usage| usage.is_object())
    {
        return Some(usage);
    }

    payload
        .get("x_groq")
        .and_then(|x_groq| x_groq.get("usage"))
        .filter(|usage| usage.is_object())
}

/// Parse usage payloads from OpenAI-compatible backends.
///
/// Different providers use different field names:
/// - OpenAI-style: `prompt_tokens`, `completion_tokens`
/// - Anthropic/MiniMax-style: `input_tokens`, `output_tokens`
/// - Cache fields may be top-level or nested in `*_tokens_details`.
#[must_use]
pub fn parse_openai_compat_usage(usage: &serde_json::Value) -> Usage {
    Usage {
        input_tokens: usage_field_u32(usage, &[
            &["prompt_tokens"],
            &["promptTokens"],
            &["input_tokens"],
            &["inputTokens"],
        ]),
        output_tokens: usage_field_u32(usage, &[
            &["completion_tokens"],
            &["completionTokens"],
            &["output_tokens"],
            &["outputTokens"],
        ]),
        cache_read_tokens: usage_field_u32(usage, &[
            &["prompt_tokens_details", "cached_tokens"],
            &["promptTokensDetails", "cachedTokens"],
            &["input_tokens_details", "cached_tokens"],
            &["inputTokensDetails", "cachedTokens"],
            &["cache_read_input_tokens"],
            &["cacheReadInputTokens"],
            &["input_tokens_details", "cache_read_input_tokens"],
            &["inputTokensDetails", "cacheReadInputTokens"],
        ]),
        cache_write_tokens: usage_field_u32(usage, &[
            &["cache_creation_input_tokens"],
            &["cacheCreationInputTokens"],
            &["input_tokens_details", "cache_creation_input_tokens"],
            &["inputTokensDetails", "cacheCreationInputTokens"],
        ]),
    }
}

/// Parse usage from an OpenAI-compatible payload, checking common nesting variants.
///
/// Providers differ on where they place usage metadata:
/// - top-level `usage`
/// - `choices[0].usage`
/// - `choices[0].delta.usage`
/// - `choices[0].message.usage`
/// - provider extension blocks (for example `x_groq.usage`)
#[must_use]
pub fn parse_openai_compat_usage_from_payload(payload: &serde_json::Value) -> Option<Usage> {
    usage_object_from_payload(payload).map(parse_openai_compat_usage)
}

/// Strip `<think>...</think>` tags from content, returning `(visible, thinking)`.
///
/// Models like DeepSeek R1, QwQ, and MiniMax embed chain-of-thought reasoning
/// inside `<think>` tags in the `content` field rather than using a separate
/// `reasoning_content` field.  This helper splits content into the visible
/// answer text and the thinking text so callers can handle them appropriately.
///
/// Edge cases handled:
/// - Multiple `<think>` blocks interspersed with answer text
/// - Unclosed `<think>` tag (remainder treated as reasoning)
/// - Empty `<think></think>` blocks
/// - Nested angle brackets inside thinking text
pub fn strip_think_tags(content: &str) -> (String, String) {
    let mut visible = String::new();
    let mut thinking = String::new();
    let mut remaining = content;

    loop {
        match remaining.find("<think>") {
            Some(start) => {
                // Text before <think> is visible
                visible.push_str(&remaining[..start]);
                let after_open = &remaining[start + "<think>".len()..];
                match after_open.find("</think>") {
                    Some(end) => {
                        thinking.push_str(&after_open[..end]);
                        remaining = &after_open[end + "</think>".len()..];
                    },
                    None => {
                        // Unclosed <think> — treat rest as reasoning
                        thinking.push_str(after_open);
                        break;
                    },
                }
            },
            None => {
                visible.push_str(remaining);
                break;
            },
        }
    }

    (
        visible.trim_start().to_string(),
        thinking.trim_start().to_string(),
    )
}

/// State for tracking streaming tool calls.
#[derive(Default)]
pub struct StreamingToolState {
    /// Map from index -> (id, name, arguments_buffer)
    pub tool_calls: HashMap<usize, (String, String, String)>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_write_tokens: u32,
    /// Whether we are currently inside a `<think>` block in streamed content.
    in_think_block: bool,
    /// Whether we are still stripping leading whitespace at the start of a
    /// think block. Set to `true` when entering `<think>`, cleared once
    /// non-whitespace reasoning content is emitted.
    think_strip_leading_ws: bool,
    /// Whether we are still stripping leading whitespace from visible content
    /// after exiting a `</think>` block. Models often emit `\n\n` between
    /// `</think>` and the actual answer.
    visible_strip_leading_ws: bool,
    /// Buffer for detecting `<think>` / `</think>` tags that may be split
    /// across SSE chunk boundaries.
    tag_buffer: String,
}

/// Result of processing a single SSE line.
#[derive(Debug)]
pub enum SseLineResult {
    /// No actionable event (empty line, non-data prefix)
    Skip,
    /// Stream is done
    Done,
    /// Events to yield
    Events(Vec<StreamEvent>),
}

/// Emit a `ReasoningDelta`, stripping leading whitespace at the start of a
/// think block so the UI doesn't show a blank prefix.
fn emit_reasoning(text: String, strip_leading_ws: &mut bool, events: &mut Vec<StreamEvent>) {
    if text.is_empty() {
        return;
    }
    let emitted = if *strip_leading_ws {
        let trimmed = text.trim_start();
        if trimmed.is_empty() {
            // Entire chunk was whitespace — keep stripping
            return;
        }
        *strip_leading_ws = false;
        trimmed.to_string()
    } else {
        text
    };
    events.push(StreamEvent::ReasoningDelta(emitted));
}

/// Emit a visible `Delta`, stripping leading whitespace after a `</think>`
/// block so the UI doesn't show blank lines before the answer.
fn emit_visible(text: String, strip_leading_ws: &mut bool, events: &mut Vec<StreamEvent>) {
    if text.is_empty() {
        return;
    }
    let emitted = if *strip_leading_ws {
        let trimmed = text.trim_start();
        if trimmed.is_empty() {
            // Entire chunk was whitespace — keep stripping
            return;
        }
        *strip_leading_ws = false;
        trimmed.to_string()
    } else {
        text
    };
    events.push(StreamEvent::Delta(emitted));
}

/// Process streamed content through the `<think>` tag state machine.
///
/// Content arriving inside `<think>...</think>` is emitted as
/// `ReasoningDelta`; content outside is emitted as `Delta`.
/// Tags may be split across SSE chunks — `tag_buffer` accumulates
/// partial tag fragments until they can be resolved.
/// Leading whitespace at the start of each think block is stripped.
fn process_content_think_tags(
    content: &str,
    state: &mut StreamingToolState,
    events: &mut Vec<StreamEvent>,
) {
    state.tag_buffer.push_str(content);

    loop {
        if state.in_think_block {
            // Look for </think> to exit think mode
            match state.tag_buffer.find("</think>") {
                Some(pos) => {
                    let thinking = state.tag_buffer[..pos].to_string();
                    emit_reasoning(thinking, &mut state.think_strip_leading_ws, events);
                    state.in_think_block = false;
                    state.visible_strip_leading_ws = true;
                    let rest = state.tag_buffer[pos + "</think>".len()..].to_string();
                    state.tag_buffer = rest;
                    // Continue loop to process remaining content
                },
                None => {
                    // Check if buffer ends with a prefix of "</think>"
                    // to avoid emitting partial tag as reasoning text.
                    let suffix_match = longest_tag_suffix(&state.tag_buffer, "</think>");
                    if suffix_match > 0 {
                        let safe = state.tag_buffer.len() - suffix_match;
                        let emit = state.tag_buffer[..safe].to_string();
                        emit_reasoning(emit, &mut state.think_strip_leading_ws, events);
                        let kept = state.tag_buffer[safe..].to_string();
                        state.tag_buffer = kept;
                    } else {
                        // No partial tag — emit everything as reasoning
                        let buf = std::mem::take(&mut state.tag_buffer);
                        emit_reasoning(buf, &mut state.think_strip_leading_ws, events);
                    }
                    break;
                },
            }
        } else {
            // Look for <think> to enter think mode
            match state.tag_buffer.find("<think>") {
                Some(pos) => {
                    let visible = state.tag_buffer[..pos].to_string();
                    emit_visible(visible, &mut state.visible_strip_leading_ws, events);
                    state.in_think_block = true;
                    state.think_strip_leading_ws = true;
                    let rest = state.tag_buffer[pos + "<think>".len()..].to_string();
                    state.tag_buffer = rest;
                    // Continue loop to process remaining content
                },
                None => {
                    // Check if buffer ends with a prefix of "<think>"
                    let suffix_match = longest_tag_suffix(&state.tag_buffer, "<think>");
                    if suffix_match > 0 {
                        let safe = state.tag_buffer.len() - suffix_match;
                        let emit = state.tag_buffer[..safe].to_string();
                        emit_visible(emit, &mut state.visible_strip_leading_ws, events);
                        let kept = state.tag_buffer[safe..].to_string();
                        state.tag_buffer = kept;
                    } else {
                        // No partial tag — emit everything as visible
                        let buf = std::mem::take(&mut state.tag_buffer);
                        emit_visible(buf, &mut state.visible_strip_leading_ws, events);
                    }
                    break;
                },
            }
        }
    }
}

/// Return the length of the longest suffix of `text` that is a prefix of `tag`.
///
/// For example, `longest_tag_suffix("abc<th", "<think>")` returns 3 because
/// `"<th"` is a 3-character prefix of `"<think>"`.
fn longest_tag_suffix(text: &str, tag: &str) -> usize {
    let text_bytes = text.as_bytes();
    let tag_bytes = tag.as_bytes();
    let max_check = text_bytes.len().min(tag_bytes.len());
    for len in (1..=max_check).rev() {
        if text_bytes[text_bytes.len() - len..] == tag_bytes[..len] {
            return len;
        }
    }
    0
}

/// Process a single SSE data line and return any events to yield.
///
/// This handles the common OpenAI streaming format used by:
/// - OpenAI API
/// - GitHub Copilot API
/// - Kimi Code API
/// - Any other OpenAI-compatible API
///
/// Content inside `<think>...</think>` tags is emitted as `ReasoningDelta`
/// events rather than `Delta`, allowing the UI to show reasoning text
/// separately. This handles models (DeepSeek R1, QwQ, MiniMax) that embed
/// chain-of-thought in `content` rather than using `reasoning_content`.
pub fn process_openai_sse_line(data: &str, state: &mut StreamingToolState) -> SseLineResult {
    if data == "[DONE]" {
        return SseLineResult::Done;
    }

    let Ok(evt) = serde_json::from_str::<serde_json::Value>(data) else {
        return SseLineResult::Skip;
    };

    let mut events = vec![StreamEvent::ProviderRaw(evt.clone())];

    if let Some(usage) = parse_openai_compat_usage_from_payload(&evt) {
        state.input_tokens = usage.input_tokens;
        state.output_tokens = usage.output_tokens;
        state.cache_read_tokens = usage.cache_read_tokens;
        state.cache_write_tokens = usage.cache_write_tokens;
    }

    let delta = &evt["choices"][0]["delta"];

    // Handle user-visible text content, stripping <think> tags.
    if let Some(content) = delta["content"].as_str()
        && !content.is_empty()
    {
        process_content_think_tags(content, state, &mut events);
    }

    // Some OpenAI-compatible backends stream planning text in
    // `reasoning_content` or `reasoning`. Surface it separately so UI can
    //  show it in the thinking area without polluting final assistant text.
    let reasoning_text = delta["reasoning_content"]
        .as_str()
        .or_else(|| delta["reasoning"].as_str());
    if let Some(reasoning_content) = reasoning_text
        && !reasoning_content.is_empty()
    {
        events.push(StreamEvent::ReasoningDelta(reasoning_content.to_string()));
    }

    // Handle tool calls
    if let Some(tcs) = delta["tool_calls"].as_array() {
        for tc in tcs {
            let index = tc["index"].as_u64().unwrap_or(0) as usize;

            // Check if this is a new tool call (has id and function.name)
            if let (Some(id), Some(name)) = (tc["id"].as_str(), tc["function"]["name"].as_str()) {
                state
                    .tool_calls
                    .insert(index, (id.to_string(), name.to_string(), String::new()));
                let metadata = extract_tool_call_metadata(tc);
                events.push(StreamEvent::ToolCallStart {
                    id: id.to_string(),
                    name: name.to_string(),
                    index,
                    metadata,
                });
            }

            // Handle arguments delta
            if let Some(args_delta) = tc["function"]["arguments"].as_str()
                && !args_delta.is_empty()
            {
                if let Some((_, _, args_buf)) = state.tool_calls.get_mut(&index) {
                    args_buf.push_str(args_delta);
                }
                events.push(StreamEvent::ToolCallArgumentsDelta {
                    index,
                    delta: args_delta.to_string(),
                });
            }
        }
    }

    // Detect error finish reasons (e.g. "network_error", "content_filter").
    // Normal reasons (null, "stop", "tool_calls", "length") are not errors.
    if let Some(reason) = evt["choices"][0]["finish_reason"].as_str() {
        match reason {
            "stop" | "tool_calls" | "length" | "function_call" => {},
            error_reason => {
                events.push(StreamEvent::Error(format!(
                    "Provider stream ended with finish_reason: {error_reason}"
                )));
            },
        }
    }

    SseLineResult::Events(events)
}

/// Generate the final events when stream ends (tool call completions + done).
///
/// Any residual content in the think-tag buffer is flushed as the appropriate
/// event type (reasoning if we were inside a think block, visible otherwise).
pub fn finalize_stream(state: &mut StreamingToolState) -> Vec<StreamEvent> {
    let mut events = Vec::new();

    // Flush any remaining think-tag buffer content
    if !state.tag_buffer.is_empty() {
        let remaining = std::mem::take(&mut state.tag_buffer);
        if state.in_think_block {
            events.push(StreamEvent::ReasoningDelta(remaining));
        } else {
            events.push(StreamEvent::Delta(remaining));
        }
    }

    // Emit completion for any pending tool calls
    for index in state.tool_calls.keys() {
        events.push(StreamEvent::ToolCallComplete { index: *index });
    }

    events.push(StreamEvent::Done(Usage {
        input_tokens: state.input_tokens,
        output_tokens: state.output_tokens,
        cache_read_tokens: state.cache_read_tokens,
        cache_write_tokens: state.cache_write_tokens,
    }));

    events
}

// ============================================================================
// Responses API helpers (shared by openai.rs and github_copilot.rs)
// ============================================================================

/// Split system messages into `instructions` and convert the rest to Responses
/// API `input` items.
///
/// The Responses API uses a top-level `instructions` field instead of a system
/// message role.  This function extracts all system messages, joins them with
/// `\n\n`, and converts the remaining messages via [`to_responses_input`].
#[must_use]
pub fn split_responses_instructions_and_input(
    messages: Vec<ChatMessage>,
) -> (Option<String>, Vec<serde_json::Value>) {
    let mut instruction_parts: Vec<String> = Vec::new();
    let mut non_system: Vec<ChatMessage> = Vec::new();

    for message in messages {
        match message {
            ChatMessage::System { content } => {
                if !content.trim().is_empty() {
                    instruction_parts.push(content);
                }
            },
            other => non_system.push(other),
        }
    }

    let instructions = if instruction_parts.is_empty() {
        None
    } else {
        Some(instruction_parts.join("\n\n"))
    };

    (instructions, to_responses_input(&non_system))
}

/// Resolve the output index from a Responses API event.
///
/// The Responses API uses `output_index` for items and `index` for
/// sub-item fields.  WebSocket events may also use `item_index`.
/// Falls back to `fallback` if none of these keys are present.
pub fn responses_output_index(event: &serde_json::Value, fallback: usize) -> usize {
    event
        .get("output_index")
        .or_else(|| event.get("item_index"))
        .or_else(|| event.get("index"))
        .and_then(serde_json::Value::as_u64)
        .map(|i| i as usize)
        .unwrap_or(fallback)
}

/// State for tracking Responses API SSE streaming.
#[derive(Default)]
pub struct ResponsesStreamState {
    /// Map from index -> (call_id, name)
    pub tool_calls: HashMap<usize, (String, String)>,
    /// Set of tool call indices that have already emitted `ToolCallComplete`.
    pub completed_tool_calls: HashSet<usize>,
    /// The next tool call index to assign.
    pub current_tool_index: usize,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_write_tokens: u32,
}

/// Process a single SSE data line from a Responses API stream.
///
/// Returns [`SseLineResult`] indicating whether to skip, yield events, or stop.
///
/// Handles the event types emitted by the Responses API:
/// - `response.output_text.delta` → text delta + `ProviderRaw`
/// - `response.output_item.added` (type=function_call) → tool call start + `ProviderRaw`
/// - `response.function_call_arguments.delta` → tool call arguments delta + `ProviderRaw`
/// - `response.function_call_arguments.done` → tool call complete + `ProviderRaw`
/// - `response.completed` → parse usage, done
/// - `error` / `response.failed` → error + `ProviderRaw`
pub fn process_responses_sse_line(data: &str, state: &mut ResponsesStreamState) -> SseLineResult {
    if data == "[DONE]" {
        return SseLineResult::Done;
    }

    let Ok(evt) = serde_json::from_str::<serde_json::Value>(data) else {
        return SseLineResult::Skip;
    };

    // Emit ProviderRaw for every parsed event, mirroring the Chat Completions path.
    let raw = StreamEvent::ProviderRaw(evt.clone());

    match evt["type"].as_str().unwrap_or("") {
        "response.output_text.delta" => {
            if let Some(delta) = evt["delta"].as_str()
                && !delta.is_empty()
            {
                SseLineResult::Events(vec![raw, StreamEvent::Delta(delta.to_string())])
            } else {
                SseLineResult::Events(vec![raw])
            }
        },
        "response.output_item.added" => {
            if evt["item"]["type"].as_str() == Some("function_call") {
                let id = evt["item"]["call_id"].as_str().unwrap_or("").to_string();
                let name = evt["item"]["name"].as_str().unwrap_or("").to_string();
                let index = responses_output_index(&evt, state.current_tool_index);
                state.current_tool_index = state.current_tool_index.max(index + 1);
                state.tool_calls.insert(index, (id.clone(), name.clone()));
                SseLineResult::Events(vec![raw, StreamEvent::ToolCallStart {
                    id,
                    name,
                    index,
                    metadata: None,
                }])
            } else {
                SseLineResult::Events(vec![raw])
            }
        },
        "response.function_call_arguments.delta" => {
            if let Some(delta) = evt["delta"].as_str()
                && !delta.is_empty()
            {
                let index =
                    responses_output_index(&evt, state.current_tool_index.saturating_sub(1));
                SseLineResult::Events(vec![raw, StreamEvent::ToolCallArgumentsDelta {
                    index,
                    delta: delta.to_string(),
                }])
            } else {
                SseLineResult::Events(vec![raw])
            }
        },
        "response.function_call_arguments.done" => {
            let index = responses_output_index(&evt, state.current_tool_index.saturating_sub(1));
            if state.completed_tool_calls.insert(index) {
                SseLineResult::Events(vec![raw, StreamEvent::ToolCallComplete { index }])
            } else {
                SseLineResult::Events(vec![raw])
            }
        },
        "response.completed" => {
            if let Some(usage) = evt
                .get("response")
                .and_then(|response| response.get("usage"))
            {
                let parsed = parse_openai_compat_usage(usage);
                state.input_tokens = parsed.input_tokens;
                state.output_tokens = parsed.output_tokens;
                state.cache_read_tokens = parsed.cache_read_tokens;
                state.cache_write_tokens = parsed.cache_write_tokens;
            }
            SseLineResult::Done
        },
        "error" | "response.failed" => {
            let msg = evt["error"]["message"]
                .as_str()
                .or_else(|| evt["response"]["error"]["message"].as_str())
                .or_else(|| evt["message"].as_str())
                .unwrap_or("unknown error");
            SseLineResult::Events(vec![raw, StreamEvent::Error(msg.to_string())])
        },
        _ => SseLineResult::Events(vec![raw]),
    }
}

/// Generate the final events when a Responses API stream ends.
///
/// Emits `ToolCallComplete` for any pending tool calls and a final `Done` with
/// accumulated usage.
pub fn finalize_responses_stream(state: &mut ResponsesStreamState) -> Vec<StreamEvent> {
    let mut events = Vec::new();

    let mut pending: Vec<usize> = state.tool_calls.keys().copied().collect();
    pending.sort_unstable();
    for index in pending {
        if state.completed_tool_calls.insert(index) {
            events.push(StreamEvent::ToolCallComplete { index });
        }
    }

    events.push(StreamEvent::Done(Usage {
        input_tokens: state.input_tokens,
        output_tokens: state.output_tokens,
        cache_read_tokens: state.cache_read_tokens,
        cache_write_tokens: state.cache_write_tokens,
    }));

    events
}

/// Parse a non-streaming Responses API JSON response into [`CompletionResponse`].
///
/// The Responses API returns an `output` array containing `message` items
/// (with `content[].text`) and `function_call` items (with `call_id`, `name`,
/// `arguments`).
pub fn parse_responses_completion(resp: &serde_json::Value) -> CompletionResponse {
    let mut text: Option<String> = None;
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    if let Some(output) = resp.get("output").and_then(|o| o.as_array()) {
        for item in output {
            match item["type"].as_str().unwrap_or("") {
                "message" => {
                    if let Some(content) = item.get("content").and_then(|c| c.as_array()) {
                        for part in content {
                            if part["type"].as_str() == Some("output_text")
                                && let Some(t) = part["text"].as_str()
                            {
                                text = Some(text.map_or_else(|| t.to_string(), |prev| prev + t));
                            }
                        }
                    }
                },
                "function_call" => {
                    let id = item["call_id"].as_str().unwrap_or("").to_string();
                    let name = item["name"].as_str().unwrap_or("").to_string();
                    let arguments = decode_tool_call_arguments(item.get("arguments"));
                    tool_calls.push(ToolCall {
                        id,
                        name,
                        arguments,
                        metadata: None,
                    });
                },
                _ => {},
            }
        }
    }

    let usage = resp
        .get("usage")
        .map(parse_openai_compat_usage)
        .unwrap_or_default();

    CompletionResponse {
        text,
        tool_calls,
        usage,
    }
}
