//! Shared helpers for OpenAI-compatible streaming with tools.
//!
//! This module provides reusable functions for parsing OpenAI-style SSE streams
//! that include tool calls. Used by openai.rs, github_copilot.rs, and kimi_code.rs.

use std::collections::HashMap;

use {serde::Serialize, tracing::trace};

use moltis_agents::model::{ChatMessage, StreamEvent, ToolCall, Usage, UserContent};

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

/// Recursively patch schema for OpenAI strict mode compliance.
///
/// OpenAI's strict mode requires:
/// 1. `additionalProperties: false` on every object in the schema tree
/// 2. All properties must be listed in the `required` array
///
/// This function recursively patches nested objects in `properties`, array
/// `items`, `anyOf`/`oneOf`/`allOf` variants, etc.
pub fn patch_schema_for_strict_mode(schema: &mut serde_json::Value) {
    let Some(obj) = schema.as_object_mut() else {
        return;
    };

    // If this is an object type, apply strict mode requirements
    if obj.get("type").and_then(|t| t.as_str()) == Some("object") {
        // Add additionalProperties: false
        obj.insert("additionalProperties".to_string(), serde_json::json!(false));

        // Ensure all properties are in required array
        // Objects without properties need empty properties + empty required
        if let Some(props) = obj.get("properties").and_then(|p| p.as_object()) {
            let all_prop_names: Vec<serde_json::Value> =
                props.keys().map(|k| serde_json::json!(k)).collect();
            obj.insert("required".to_string(), serde_json::json!(all_prop_names));
        } else {
            // Object without properties - add empty properties and required
            obj.insert("properties".to_string(), serde_json::json!({}));
            obj.insert("required".to_string(), serde_json::json!([]));
        }
    }

    // Recurse into properties
    if let Some(props) = obj.get_mut("properties").and_then(|p| p.as_object_mut()) {
        for (_, prop_schema) in props.iter_mut() {
            patch_schema_for_strict_mode(prop_schema);
        }
    }

    // Recurse into array items
    if let Some(items) = obj.get_mut("items") {
        patch_schema_for_strict_mode(items);
    }

    // Recurse into anyOf/oneOf/allOf
    for key in ["anyOf", "oneOf", "allOf"] {
        if let Some(variants) = obj.get_mut(key).and_then(|v| v.as_array_mut()) {
            for variant in variants {
                patch_schema_for_strict_mode(variant);
            }
        }
    }

    // Recurse into additionalProperties if it's a schema (not just true/false)
    if let Some(additional) = obj.get_mut("additionalProperties")
        && additional.is_object()
    {
        patch_schema_for_strict_mode(additional);
    }
}

/// Convert tool schemas to OpenAI Chat Completions function-calling format.
///
/// Uses the nested `function` object format required by Chat Completions API:
/// ```json
/// { "type": "function", "function": { "name": "...", ... } }
/// ```
///
/// Adds `strict: true` and patches schemas for strict mode compliance:
/// - `additionalProperties: false` on all object schemas
/// - All properties included in `required` array
///
/// This is required by some APIs (Claude via Copilot) to ensure the model
/// provides all required fields.
///
/// See: <https://platform.openai.com/docs/guides/function-calling>
pub fn to_openai_tools(tools: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let result: Vec<serde_json::Value> = tools
        .iter()
        .filter_map(|t| {
            // Clone parameters and patch for strict mode
            let mut params = t["parameters"].clone();
            patch_schema_for_strict_mode(&mut params);

            let name = t["name"].as_str()?.to_string();
            let description = t["description"].as_str().unwrap_or("").to_string();

            // Use typed struct to ensure all required fields are present
            let tool = ChatCompletionsTool {
                tool_type: "function",
                function: ChatCompletionsFunction {
                    name: name.clone(),
                    description,
                    parameters: params,
                    strict: true,
                },
            };

            trace!(tool_name = %name, "converted tool to Chat Completions format");

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
            ChatMessage::User { content } => {
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
                    let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
                    let arguments = serde_json::from_str(args_str).unwrap_or(serde_json::json!({}));
                    Some(ToolCall {
                        id,
                        name,
                        arguments,
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
    // `reasoning_content`. Surface it separately so UI can show it in the
    // thinking area without polluting final assistant text.
    if let Some(reasoning_content) = delta["reasoning_content"].as_str()
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
                events.push(StreamEvent::ToolCallStart {
                    id: id.to_string(),
                    name: name.to_string(),
                    index,
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

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_openai_tools() {
        let tools = vec![serde_json::json!({
            "name": "test_tool",
            "description": "A test tool",
            "parameters": {"type": "object", "properties": {"x": {"type": "string"}}}
        })];
        let converted = to_openai_tools(&tools);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["type"], "function");
        assert_eq!(converted[0]["function"]["name"], "test_tool");
        // Verify strict mode and additionalProperties
        assert_eq!(converted[0]["function"]["strict"], true);
        assert_eq!(
            converted[0]["function"]["parameters"]["additionalProperties"],
            false
        );
    }

    #[test]
    fn test_to_openai_tools_nested_objects() {
        // Test that nested objects get additionalProperties: false
        let tools = vec![serde_json::json!({
            "name": "nested_tool",
            "description": "Tool with nested objects",
            "parameters": {
                "type": "object",
                "properties": {
                    "outer": {
                        "type": "object",
                        "properties": {
                            "inner": {
                                "type": "object",
                                "properties": {
                                    "value": {"type": "string"}
                                }
                            }
                        }
                    }
                }
            }
        })];
        let converted = to_openai_tools(&tools);
        let params = &converted[0]["function"]["parameters"];

        // Top level should have additionalProperties: false
        assert_eq!(params["additionalProperties"], false);

        // Nested object should have additionalProperties: false
        let outer = &params["properties"]["outer"];
        assert_eq!(outer["additionalProperties"], false);

        // Deeply nested object should also have additionalProperties: false
        let inner = &outer["properties"]["inner"];
        assert_eq!(inner["additionalProperties"], false);
    }

    #[test]
    fn test_to_openai_tools_array_items() {
        // Test that array items with object type get additionalProperties: false
        // This is the case that was failing for mcp__memory__delete_observations
        let tools = vec![serde_json::json!({
            "name": "delete_observations",
            "description": "Delete observations",
            "parameters": {
                "type": "object",
                "properties": {
                    "deletions": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "observation": {"type": "string"}
                            },
                            "required": ["observation"]
                        }
                    }
                }
            }
        })];
        let converted = to_openai_tools(&tools);
        let params = &converted[0]["function"]["parameters"];

        // Top level should have additionalProperties: false
        assert_eq!(params["additionalProperties"], false);

        // Array items object should have additionalProperties: false
        let items = &params["properties"]["deletions"]["items"];
        assert_eq!(items["additionalProperties"], false);
    }

    #[test]
    fn test_to_openai_tools_anyof() {
        // Test that anyOf/oneOf/allOf variants get additionalProperties: false
        let tools = vec![serde_json::json!({
            "name": "union_tool",
            "description": "Tool with anyOf",
            "parameters": {
                "type": "object",
                "properties": {
                    "value": {
                        "anyOf": [
                            {"type": "string"},
                            {"type": "object", "properties": {"x": {"type": "number"}}}
                        ]
                    }
                }
            }
        })];
        let converted = to_openai_tools(&tools);
        let params = &converted[0]["function"]["parameters"];

        // The object variant in anyOf should have additionalProperties: false
        let any_of = params["properties"]["value"]["anyOf"].as_array().unwrap();
        // First variant is string, no additionalProperties needed
        // Second variant is object, should have additionalProperties: false
        assert_eq!(any_of[1]["additionalProperties"], false);
    }

    #[test]
    fn test_to_openai_tools_all_properties_required() {
        // Test that all properties are added to the required array
        // This is the case that was failing for web_fetch with extract_mode
        let tools = vec![serde_json::json!({
            "name": "web_fetch",
            "description": "Fetch a URL",
            "parameters": {
                "type": "object",
                "properties": {
                    "url": {"type": "string"},
                    "extract_mode": {"type": "string", "enum": ["markdown", "text"]},
                    "max_chars": {"type": "integer"}
                },
                "required": ["url"]  // Only url was originally required
            }
        })];
        let converted = to_openai_tools(&tools);
        let params = &converted[0]["function"]["parameters"];

        // All properties should be in required array
        let required = params["required"].as_array().unwrap();
        assert_eq!(required.len(), 3);
        assert!(required.contains(&serde_json::json!("url")));
        assert!(required.contains(&serde_json::json!("extract_mode")));
        assert!(required.contains(&serde_json::json!("max_chars")));
    }

    #[test]
    fn test_to_openai_tools_empty() {
        let converted = to_openai_tools(&[]);
        assert!(converted.is_empty());
    }

    #[test]
    fn test_parse_tool_calls_empty() {
        let msg = serde_json::json!({"content": "hello"});
        assert!(parse_tool_calls(&msg).is_empty());
    }

    #[test]
    fn test_parse_tool_calls_with_calls() {
        let msg = serde_json::json!({
            "tool_calls": [{
                "id": "call_1",
                "function": {
                    "name": "get_weather",
                    "arguments": "{\"city\":\"SF\"}"
                }
            }]
        });
        let calls = parse_tool_calls(&msg);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(calls[0].arguments["city"], "SF");
    }

    #[test]
    fn test_parse_openai_compat_usage_openai_fields() {
        let usage = serde_json::json!({
            "prompt_tokens": 42,
            "completion_tokens": 17,
            "prompt_tokens_details": {
                "cached_tokens": 9
            }
        });

        let parsed = parse_openai_compat_usage(&usage);
        assert_eq!(parsed.input_tokens, 42);
        assert_eq!(parsed.output_tokens, 17);
        assert_eq!(parsed.cache_read_tokens, 9);
        assert_eq!(parsed.cache_write_tokens, 0);
    }

    #[test]
    fn test_parse_openai_compat_usage_input_output_fields() {
        let usage = serde_json::json!({
            "input_tokens": 123,
            "output_tokens": 45,
            "cache_read_input_tokens": 7,
            "cache_creation_input_tokens": 11
        });

        let parsed = parse_openai_compat_usage(&usage);
        assert_eq!(parsed.input_tokens, 123);
        assert_eq!(parsed.output_tokens, 45);
        assert_eq!(parsed.cache_read_tokens, 7);
        assert_eq!(parsed.cache_write_tokens, 11);
    }

    #[test]
    fn test_parse_openai_compat_usage_camel_case_string_fields() {
        let usage = serde_json::json!({
            "promptTokens": "91",
            "completionTokens": "27",
            "promptTokensDetails": {
                "cachedTokens": "14"
            },
            "cacheCreationInputTokens": "6"
        });

        let parsed = parse_openai_compat_usage(&usage);
        assert_eq!(parsed.input_tokens, 91);
        assert_eq!(parsed.output_tokens, 27);
        assert_eq!(parsed.cache_read_tokens, 14);
        assert_eq!(parsed.cache_write_tokens, 6);
    }

    #[test]
    fn test_parse_openai_compat_usage_from_payload_choice_usage() {
        let payload = serde_json::json!({
            "id": "chatcmpl-123",
            "choices": [{
                "index": 0,
                "usage": {
                    "input_tokens": 32,
                    "output_tokens": 9
                }
            }]
        });

        let parsed = parse_openai_compat_usage_from_payload(&payload).expect("usage");
        assert_eq!(parsed.input_tokens, 32);
        assert_eq!(parsed.output_tokens, 9);
        assert_eq!(parsed.cache_read_tokens, 0);
        assert_eq!(parsed.cache_write_tokens, 0);
    }

    #[test]
    fn test_parse_openai_compat_usage_from_payload_delta_usage() {
        let payload = serde_json::json!({
            "choices": [{
                "delta": {
                    "usage": {
                        "prompt_tokens": 70,
                        "completion_tokens": 12
                    }
                }
            }]
        });

        let parsed = parse_openai_compat_usage_from_payload(&payload).expect("usage");
        assert_eq!(parsed.input_tokens, 70);
        assert_eq!(parsed.output_tokens, 12);
    }

    #[test]
    fn test_process_sse_usage_chunk_with_input_output_fields() {
        let mut state = StreamingToolState::default();
        let data = r#"{"choices":[],"usage":{"input_tokens":64,"output_tokens":19,"cache_read_input_tokens":5,"cache_creation_input_tokens":2}}"#;
        let result = process_openai_sse_line(data, &mut state);
        // Usage-only chunks emit only ProviderRaw, no content Delta
        match result {
            SseLineResult::Events(events) => {
                assert!(
                    events
                        .iter()
                        .all(|e| matches!(e, StreamEvent::ProviderRaw(_)))
                );
            },
            _ => panic!("Expected Events with ProviderRaw"),
        }
        assert_eq!(state.input_tokens, 64);
        assert_eq!(state.output_tokens, 19);
        assert_eq!(state.cache_read_tokens, 5);
        assert_eq!(state.cache_write_tokens, 2);
    }

    #[test]
    fn test_process_sse_usage_chunk_with_choice_nested_usage() {
        let mut state = StreamingToolState::default();
        let data = r#"{"choices":[{"usage":{"prompt_tokens":18,"completion_tokens":7,"prompt_tokens_details":{"cached_tokens":4}}}]}"#;
        let result = process_openai_sse_line(data, &mut state);
        // Usage-only chunks emit only ProviderRaw, no content Delta
        match result {
            SseLineResult::Events(events) => {
                assert!(
                    events
                        .iter()
                        .all(|e| matches!(e, StreamEvent::ProviderRaw(_)))
                );
            },
            _ => panic!("Expected Events with ProviderRaw"),
        }
        assert_eq!(state.input_tokens, 18);
        assert_eq!(state.output_tokens, 7);
        assert_eq!(state.cache_read_tokens, 4);
    }

    #[test]
    fn test_process_sse_done() {
        let mut state = StreamingToolState::default();
        matches!(
            process_openai_sse_line("[DONE]", &mut state),
            SseLineResult::Done
        );
    }

    #[test]
    fn test_process_sse_text_delta() {
        let mut state = StreamingToolState::default();
        let data = r#"{"choices":[{"delta":{"content":"Hello"}}]}"#;
        let result = process_openai_sse_line(data, &mut state);
        match result {
            SseLineResult::Events(events) => {
                // First event is always ProviderRaw, second is the Delta
                assert_eq!(events.len(), 2);
                assert!(matches!(&events[0], StreamEvent::ProviderRaw(_)));
                assert!(matches!(&events[1], StreamEvent::Delta(s) if s == "Hello"));
            },
            _ => panic!("Expected Events"),
        }
    }

    #[test]
    fn test_process_sse_reasoning_content_delta() {
        let mut state = StreamingToolState::default();
        let data = r#"{"choices":[{"delta":{"reasoning_content":"plan step"}}]}"#;
        let result = process_openai_sse_line(data, &mut state);
        match result {
            SseLineResult::Events(events) => {
                assert_eq!(events.len(), 2);
                assert!(matches!(&events[0], StreamEvent::ProviderRaw(_)));
                assert!(matches!(
                    &events[1],
                    StreamEvent::ReasoningDelta(s) if s == "plan step"
                ));
            },
            _ => panic!("Expected Events"),
        }
    }

    #[test]
    fn test_process_sse_tool_call_start() {
        let mut state = StreamingToolState::default();
        let data = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"test"}}]}}]}"#;
        let result = process_openai_sse_line(data, &mut state);
        match result {
            SseLineResult::Events(events) => {
                assert_eq!(events.len(), 2);
                assert!(matches!(&events[0], StreamEvent::ProviderRaw(_)));
                assert!(matches!(
                    &events[1],
                    StreamEvent::ToolCallStart { id, name, index }
                    if id == "call_1" && name == "test" && *index == 0
                ));
            },
            _ => panic!("Expected Events"),
        }
        assert!(state.tool_calls.contains_key(&0));
    }

    #[test]
    fn test_process_sse_tool_call_args_delta() {
        let mut state = StreamingToolState::default();
        // First, start the tool call
        let start_data = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"test"}}]}}]}"#;
        let _ = process_openai_sse_line(start_data, &mut state);

        // Then, send args delta
        let args_data = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"x\":"}}]}}]}"#;
        let result = process_openai_sse_line(args_data, &mut state);
        match result {
            SseLineResult::Events(events) => {
                assert_eq!(events.len(), 2);
                assert!(matches!(&events[0], StreamEvent::ProviderRaw(_)));
                assert!(matches!(
                    &events[1],
                    StreamEvent::ToolCallArgumentsDelta { index, delta }
                    if *index == 0 && delta == "{\"x\":"
                ));
            },
            _ => panic!("Expected Events"),
        }
    }

    #[test]
    fn test_finalize_stream() {
        let mut state = StreamingToolState::default();
        state
            .tool_calls
            .insert(0, ("call_1".into(), "test".into(), "{}".into()));
        state.input_tokens = 10;
        state.output_tokens = 5;

        let events = finalize_stream(&mut state);
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], StreamEvent::ToolCallComplete {
            index: 0
        }));
        assert!(matches!(
            &events[1],
            StreamEvent::Done(usage) if usage.input_tokens == 10 && usage.output_tokens == 5
        ));
    }

    // ============================================================
    // Tests for to_responses_api_tools (OpenAI Responses API format)
    // See: https://learn.microsoft.com/en-us/azure/ai-foundry/openai/how-to/responses
    // ============================================================

    #[test]
    fn test_to_responses_api_tools_format() {
        // Responses API uses flat format: name at top level, not nested under "function"
        let tools = vec![serde_json::json!({
            "name": "get_weather",
            "description": "Get the weather for a location",
            "parameters": {
                "type": "object",
                "properties": {
                    "location": {"type": "string"}
                },
                "required": ["location"]
            }
        })];
        let converted = to_responses_api_tools(&tools);
        assert_eq!(converted.len(), 1);

        // Verify flat format (name at top level, not nested under function)
        assert_eq!(converted[0]["type"], "function");
        assert_eq!(converted[0]["name"], "get_weather");
        assert_eq!(
            converted[0]["description"],
            "Get the weather for a location"
        );
        assert_eq!(converted[0]["strict"], true);

        // Should NOT have a "function" wrapper
        assert!(converted[0].get("function").is_none());

        // Parameters should be patched for strict mode
        assert_eq!(converted[0]["parameters"]["additionalProperties"], false);
    }

    #[test]
    fn test_to_responses_api_tools_nested_objects() {
        // Test that nested objects get additionalProperties: false and required
        let tools = vec![serde_json::json!({
            "name": "delete_observations",
            "description": "Delete observations",
            "parameters": {
                "type": "object",
                "properties": {
                    "deletions": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "observation": {"type": "string"},
                                "entity": {"type": "string"}
                            }
                        }
                    }
                }
            }
        })];
        let converted = to_responses_api_tools(&tools);
        let params = &converted[0]["parameters"];

        // Top level should have additionalProperties: false
        assert_eq!(params["additionalProperties"], false);

        // All properties at top level should be in required
        let required = params["required"].as_array().unwrap();
        assert!(required.contains(&serde_json::json!("deletions")));

        // Array items object should have additionalProperties: false
        let items = &params["properties"]["deletions"]["items"];
        assert_eq!(items["additionalProperties"], false);

        // Array items should have all properties in required
        let items_required = items["required"].as_array().unwrap();
        assert!(items_required.contains(&serde_json::json!("observation")));
        assert!(items_required.contains(&serde_json::json!("entity")));
    }

    #[test]
    fn test_object_without_properties() {
        // Objects without explicit properties need empty properties + empty required
        // This was failing for the cron tool's "patch" field
        let tools = vec![serde_json::json!({
            "name": "cron",
            "description": "Cron tool",
            "parameters": {
                "type": "object",
                "properties": {
                    "action": {"type": "string"},
                    "patch": {
                        "type": "object",
                        "description": "Fields to update (no properties defined)"
                    }
                },
                "required": ["action"]
            }
        })];
        let converted = to_responses_api_tools(&tools);
        let params = &converted[0]["parameters"];

        // Top level should have all properties in required
        let required = params["required"].as_array().unwrap();
        assert!(required.contains(&serde_json::json!("action")));
        assert!(required.contains(&serde_json::json!("patch")));

        // The "patch" object should have empty properties and empty required
        let patch = &params["properties"]["patch"];
        assert_eq!(patch["type"], "object");
        assert_eq!(patch["additionalProperties"], false);
        assert_eq!(patch["properties"], serde_json::json!({}));
        assert_eq!(patch["required"], serde_json::json!([]));
    }

    #[test]
    fn test_chat_completions_vs_responses_api_format() {
        // Verify the two formats are different
        let tools = vec![serde_json::json!({
            "name": "test_tool",
            "description": "A test tool",
            "parameters": {"type": "object", "properties": {"x": {"type": "string"}}}
        })];

        let chat_completions = to_openai_tools(&tools);
        let responses_api = to_responses_api_tools(&tools);

        // Chat Completions: nested under "function"
        assert!(chat_completions[0].get("function").is_some());
        assert_eq!(chat_completions[0]["function"]["name"], "test_tool");

        // Responses API: flat format
        assert!(responses_api[0].get("function").is_none());
        assert_eq!(responses_api[0]["name"], "test_tool");
    }

    // ============================================================
    // Tests for strip_think_tags (non-streaming)
    // ============================================================

    #[test]
    fn test_strip_think_tags_no_tags() {
        let (visible, thinking) = strip_think_tags("Hello, world!");
        assert_eq!(visible, "Hello, world!");
        assert_eq!(thinking, "");
    }

    #[test]
    fn test_strip_think_tags_single_block() {
        let input = "<think>reasoning here</think>The answer is 42.";
        let (visible, thinking) = strip_think_tags(input);
        assert_eq!(visible, "The answer is 42.");
        assert_eq!(thinking, "reasoning here");
    }

    #[test]
    fn test_strip_think_tags_multiple_blocks() {
        let input = "Start<think>thought 1</think> middle <think>thought 2</think> end";
        let (visible, thinking) = strip_think_tags(input);
        assert_eq!(visible, "Start middle  end");
        assert_eq!(thinking, "thought 1thought 2");
    }

    #[test]
    fn test_strip_think_tags_unclosed() {
        let input = "visible<think>unclosed reasoning";
        let (visible, thinking) = strip_think_tags(input);
        assert_eq!(visible, "visible");
        assert_eq!(thinking, "unclosed reasoning");
    }

    #[test]
    fn test_strip_think_tags_empty_block() {
        let input = "<think></think>just the answer";
        let (visible, thinking) = strip_think_tags(input);
        assert_eq!(visible, "just the answer");
        assert_eq!(thinking, "");
    }

    #[test]
    fn test_strip_think_tags_only_thinking() {
        let input = "<think>all reasoning no answer</think>";
        let (visible, thinking) = strip_think_tags(input);
        assert_eq!(visible, "");
        assert_eq!(thinking, "all reasoning no answer");
    }

    // ============================================================
    // Tests for longest_tag_suffix helper
    // ============================================================

    #[test]
    fn test_longest_tag_suffix_full_prefix() {
        assert_eq!(longest_tag_suffix("abc<think>", "<think>"), 7);
    }

    #[test]
    fn test_longest_tag_suffix_partial() {
        assert_eq!(longest_tag_suffix("abc<th", "<think>"), 3);
    }

    #[test]
    fn test_longest_tag_suffix_single_char() {
        assert_eq!(longest_tag_suffix("abc<", "<think>"), 1);
    }

    #[test]
    fn test_longest_tag_suffix_no_match() {
        assert_eq!(longest_tag_suffix("abcdef", "<think>"), 0);
    }

    // ============================================================
    // Tests for streaming think-tag handling
    // ============================================================

    #[test]
    fn test_stream_think_block_single_chunk() {
        let mut state = StreamingToolState::default();
        // Full think block + answer in one SSE chunk
        let data = r#"{"choices":[{"delta":{"content":"<think>reasoning</think>The answer"}}]}"#;
        let result = process_openai_sse_line(data, &mut state);
        match result {
            SseLineResult::Events(events) => {
                let reasoning: Vec<_> = events
                    .iter()
                    .filter_map(|e| match e {
                        StreamEvent::ReasoningDelta(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .collect();
                let visible: Vec<_> = events
                    .iter()
                    .filter_map(|e| match e {
                        StreamEvent::Delta(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(reasoning.join(""), "reasoning");
                assert_eq!(visible.join(""), "The answer");
            },
            _ => panic!("Expected Events"),
        }
    }

    #[test]
    fn test_stream_think_tag_split_across_chunks() {
        let mut state = StreamingToolState::default();

        // First chunk: starts with partial opening tag
        let data1 = r#"{"choices":[{"delta":{"content":"<thi"}}]}"#;
        let result1 = process_openai_sse_line(data1, &mut state);
        // Should hold back the partial tag — only ProviderRaw emitted, no Delta
        match result1 {
            SseLineResult::Events(events) => {
                let delta_count = events
                    .iter()
                    .filter(|e| matches!(e, StreamEvent::Delta(_)))
                    .count();
                assert_eq!(delta_count, 0, "partial tag should be buffered");
            },
            SseLineResult::Skip => {},
            _ => panic!("Expected Events or Skip"),
        }

        // Second chunk: completes the opening tag + reasoning
        let data2 = r#"{"choices":[{"delta":{"content":"nk>deep thought</think>42"}}]}"#;
        let result2 = process_openai_sse_line(data2, &mut state);
        match result2 {
            SseLineResult::Events(events) => {
                let reasoning: String = events
                    .iter()
                    .filter_map(|e| match e {
                        StreamEvent::ReasoningDelta(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .collect();
                let visible: String = events
                    .iter()
                    .filter_map(|e| match e {
                        StreamEvent::Delta(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(reasoning, "deep thought");
                assert_eq!(visible, "42");
            },
            _ => panic!("Expected Events"),
        }
    }

    #[test]
    fn test_stream_multiple_think_blocks() {
        let mut state = StreamingToolState::default();

        let data = r#"{"choices":[{"delta":{"content":"A<think>t1</think>B<think>t2</think>C"}}]}"#;
        let result = process_openai_sse_line(data, &mut state);
        match result {
            SseLineResult::Events(events) => {
                let reasoning: String = events
                    .iter()
                    .filter_map(|e| match e {
                        StreamEvent::ReasoningDelta(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .collect();
                let visible: String = events
                    .iter()
                    .filter_map(|e| match e {
                        StreamEvent::Delta(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(reasoning, "t1t2");
                assert_eq!(visible, "ABC");
            },
            _ => panic!("Expected Events"),
        }
    }

    #[test]
    fn test_stream_unclosed_think_emits_as_reasoning() {
        let mut state = StreamingToolState::default();

        // Start a think block that never closes
        let data = r#"{"choices":[{"delta":{"content":"<think>partial reasoning"}}]}"#;
        let result = process_openai_sse_line(data, &mut state);
        assert!(state.in_think_block);

        // Reasoning should be emitted during processing (not buffered)
        match result {
            SseLineResult::Events(events) => {
                let reasoning: String = events
                    .iter()
                    .filter_map(|e| match e {
                        StreamEvent::ReasoningDelta(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(reasoning, "partial reasoning");
            },
            _ => panic!("Expected Events"),
        }
    }

    #[test]
    fn test_stream_unclosed_think_partial_tag_flushed_on_finalize() {
        let mut state = StreamingToolState::default();

        // Enter think mode, then receive content ending with a partial </think>
        let data1 = r#"{"choices":[{"delta":{"content":"<think>start"}}]}"#;
        let _ = process_openai_sse_line(data1, &mut state);
        assert!(state.in_think_block);

        // Content ending with partial closing tag
        let data2 = r#"{"choices":[{"delta":{"content":" more</thi"}}]}"#;
        let _ = process_openai_sse_line(data2, &mut state);
        // The "</thi" should be buffered as a partial tag

        // Finalize should flush the buffered partial tag as reasoning
        let final_events = finalize_stream(&mut state);
        let has_reasoning = final_events
            .iter()
            .any(|e| matches!(e, StreamEvent::ReasoningDelta(s) if s.contains("</thi")));
        assert!(
            has_reasoning,
            "finalize should flush partial tag buffer as reasoning"
        );
    }

    #[test]
    fn test_stream_think_coexists_with_reasoning_content() {
        // Models that send both reasoning_content AND <think> tags
        let mut state = StreamingToolState::default();
        let data = r#"{"choices":[{"delta":{"content":"<think>inline thought</think>answer","reasoning_content":"separate reasoning"}}]}"#;
        let result = process_openai_sse_line(data, &mut state);
        match result {
            SseLineResult::Events(events) => {
                let reasoning_deltas: Vec<_> = events
                    .iter()
                    .filter_map(|e| match e {
                        StreamEvent::ReasoningDelta(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect();
                let visible: String = events
                    .iter()
                    .filter_map(|e| match e {
                        StreamEvent::Delta(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .collect();
                // Both reasoning sources should produce ReasoningDelta events
                assert!(reasoning_deltas.contains(&"inline thought".to_string()));
                assert!(reasoning_deltas.contains(&"separate reasoning".to_string()));
                assert_eq!(visible, "answer");
            },
            _ => panic!("Expected Events"),
        }
    }

    #[test]
    fn test_stream_no_think_tags_unchanged() {
        // Normal content without think tags should still produce Delta
        let mut state = StreamingToolState::default();
        let data = r#"{"choices":[{"delta":{"content":"Hello world"}}]}"#;
        let result = process_openai_sse_line(data, &mut state);
        match result {
            SseLineResult::Events(events) => {
                assert_eq!(events.len(), 2);
                assert!(matches!(&events[0], StreamEvent::ProviderRaw(_)));
                assert!(matches!(&events[1], StreamEvent::Delta(s) if s == "Hello world"));
            },
            _ => panic!("Expected Events"),
        }
    }

    #[test]
    fn test_stream_closing_tag_split_across_chunks() {
        let mut state = StreamingToolState::default();

        // Enter think mode
        let data1 = r#"{"choices":[{"delta":{"content":"<think>reasoning</thi"}}]}"#;
        let _ = process_openai_sse_line(data1, &mut state);
        assert!(state.in_think_block);

        // Complete the closing tag
        let data2 = r#"{"choices":[{"delta":{"content":"nk>visible"}}]}"#;
        let result = process_openai_sse_line(data2, &mut state);
        match result {
            SseLineResult::Events(events) => {
                let visible: String = events
                    .iter()
                    .filter_map(|e| match e {
                        StreamEvent::Delta(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(visible, "visible");
                assert!(!state.in_think_block);
            },
            _ => panic!("Expected Events"),
        }
    }

    // ============================================================
    // Tests for leading whitespace stripping in think blocks
    // ============================================================

    #[test]
    fn test_strip_think_tags_trims_leading_whitespace() {
        let input = "<think>\n  Let me think\n</think>Answer";
        let (visible, thinking) = strip_think_tags(input);
        assert_eq!(visible, "Answer");
        assert_eq!(thinking, "Let me think\n");
    }

    #[test]
    fn test_stream_think_strips_leading_newline() {
        let mut state = StreamingToolState::default();
        let data = r#"{"choices":[{"delta":{"content":"<think>\nreasoning</think>answer"}}]}"#;
        let result = process_openai_sse_line(data, &mut state);
        match result {
            SseLineResult::Events(events) => {
                let reasoning: String = events
                    .iter()
                    .filter_map(|e| match e {
                        StreamEvent::ReasoningDelta(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(reasoning, "reasoning");
            },
            _ => panic!("Expected Events"),
        }
    }

    #[test]
    fn test_stream_think_strips_leading_whitespace_across_chunks() {
        let mut state = StreamingToolState::default();

        // First chunk: open tag + whitespace only
        let data1 = r#"{"choices":[{"delta":{"content":"<think>\n  "}}]}"#;
        let result1 = process_openai_sse_line(data1, &mut state);
        // Whitespace-only reasoning should be swallowed
        match result1 {
            SseLineResult::Events(events) => {
                let reasoning_count = events
                    .iter()
                    .filter(|e| matches!(e, StreamEvent::ReasoningDelta(_)))
                    .count();
                assert_eq!(reasoning_count, 0, "leading whitespace should be stripped");
            },
            SseLineResult::Skip => {},
            _ => panic!("Expected Events or Skip"),
        }

        // Second chunk: actual reasoning
        let data2 = r#"{"choices":[{"delta":{"content":"actual thought</think>ok"}}]}"#;
        let result2 = process_openai_sse_line(data2, &mut state);
        match result2 {
            SseLineResult::Events(events) => {
                let reasoning: String = events
                    .iter()
                    .filter_map(|e| match e {
                        StreamEvent::ReasoningDelta(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(reasoning, "actual thought");
            },
            _ => panic!("Expected Events"),
        }
    }

    // ============================================================
    // Tests for visible content whitespace stripping after </think>
    // ============================================================

    #[test]
    fn test_strip_think_tags_trims_visible_after_think() {
        let input = "<think>reasoning</think>\n\nHere's the answer";
        let (visible, thinking) = strip_think_tags(input);
        assert_eq!(visible, "Here's the answer");
        assert_eq!(thinking, "reasoning");
    }

    #[test]
    fn test_stream_strips_visible_whitespace_after_think() {
        let mut state = StreamingToolState::default();
        let data = r#"{"choices":[{"delta":{"content":"<think>reasoning</think>\n\nHere's the answer"}}]}"#;
        let result = process_openai_sse_line(data, &mut state);
        match result {
            SseLineResult::Events(events) => {
                let visible: String = events
                    .iter()
                    .filter_map(|e| match e {
                        StreamEvent::Delta(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(visible, "Here's the answer");
            },
            _ => panic!("Expected Events"),
        }
    }

    #[test]
    fn test_stream_strips_visible_whitespace_across_chunks() {
        let mut state = StreamingToolState::default();

        // First chunk: think block + close tag + newlines
        let data1 = r#"{"choices":[{"delta":{"content":"<think>reasoning</think>\n\n"}}]}"#;
        let result1 = process_openai_sse_line(data1, &mut state);
        match result1 {
            SseLineResult::Events(events) => {
                let visible_count = events
                    .iter()
                    .filter(|e| matches!(e, StreamEvent::Delta(_)))
                    .count();
                assert_eq!(
                    visible_count, 0,
                    "leading whitespace after </think> should be stripped"
                );
            },
            SseLineResult::Skip => {},
            _ => panic!("Expected Events or Skip"),
        }

        // Second chunk: actual visible content
        let data2 = r#"{"choices":[{"delta":{"content":"The answer"}}]}"#;
        let result2 = process_openai_sse_line(data2, &mut state);
        match result2 {
            SseLineResult::Events(events) => {
                let visible: String = events
                    .iter()
                    .filter_map(|e| match e {
                        StreamEvent::Delta(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(visible, "The answer");
            },
            _ => panic!("Expected Events"),
        }
    }
}
