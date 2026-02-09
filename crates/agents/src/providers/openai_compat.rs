//! Shared helpers for OpenAI-compatible streaming with tools.
//!
//! This module provides reusable functions for parsing OpenAI-style SSE streams
//! that include tool calls. Used by openai.rs, github_copilot.rs, and kimi_code.rs.

use std::collections::HashMap;

use {serde::Serialize, tracing::trace};

use crate::model::{StreamEvent, ToolCall, Usage};

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

/// State for tracking streaming tool calls.
#[derive(Default)]
pub struct StreamingToolState {
    /// Map from index -> (id, name, arguments_buffer)
    pub tool_calls: HashMap<usize, (String, String, String)>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_write_tokens: u32,
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

/// Process a single SSE data line and return any events to yield.
///
/// This handles the common OpenAI streaming format used by:
/// - OpenAI API
/// - GitHub Copilot API
/// - Kimi Code API
/// - Any other OpenAI-compatible API
pub fn process_openai_sse_line(data: &str, state: &mut StreamingToolState) -> SseLineResult {
    if data == "[DONE]" {
        return SseLineResult::Done;
    }

    let Ok(evt) = serde_json::from_str::<serde_json::Value>(data) else {
        return SseLineResult::Skip;
    };

    let mut events = Vec::new();

    // Usage chunk (sent with stream_options.include_usage)
    if let Some(u) = evt.get("usage").filter(|u| !u.is_null()) {
        state.input_tokens = u["prompt_tokens"].as_u64().unwrap_or(0) as u32;
        state.output_tokens = u["completion_tokens"].as_u64().unwrap_or(0) as u32;
        // OpenAI reports cached tokens in prompt_tokens_details
        if let Some(cached) = u["prompt_tokens_details"]["cached_tokens"].as_u64() {
            state.cache_read_tokens = cached as u32;
        }
    }

    let delta = &evt["choices"][0]["delta"];

    // Handle text content
    if let Some(content) = delta["content"].as_str()
        && !content.is_empty()
    {
        events.push(StreamEvent::Delta(content.to_string()));
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

    if events.is_empty() {
        SseLineResult::Skip
    } else {
        SseLineResult::Events(events)
    }
}

/// Generate the final events when stream ends (tool call completions + done).
pub fn finalize_stream(state: &StreamingToolState) -> Vec<StreamEvent> {
    let mut events = Vec::new();

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
                assert_eq!(events.len(), 1);
                assert!(matches!(&events[0], StreamEvent::Delta(s) if s == "Hello"));
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
                assert_eq!(events.len(), 1);
                assert!(matches!(
                    &events[0],
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
                assert_eq!(events.len(), 1);
                assert!(matches!(
                    &events[0],
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

        let events = finalize_stream(&state);
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
}
