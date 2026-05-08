//! Shared helpers for OpenAI-compatible streaming with tools.
//!
//! This module provides reusable functions for parsing OpenAI-style SSE streams
//! that include tool calls. Used by openai.rs, github_copilot.rs, and kimi_code.rs.

mod provider;
mod schema_normalization;
mod strict_mode;

#[cfg(test)]
mod tests;

pub use {
    provider::{
        ChatCompletionsFunction, ChatCompletionsTool, ResponsesApiTool, ResponsesStreamState,
        SseLineResult, StreamingToolState, finalize_responses_stream, finalize_stream,
        parse_openai_compat_usage, parse_openai_compat_usage_from_payload,
        parse_responses_completion, parse_tool_calls, process_openai_sse_line,
        process_responses_sse_line, responses_output_index, split_responses_instructions_and_input,
        strip_think_tags, to_openai_tools, to_responses_api_tools, to_responses_input,
    },
    schema_normalization::strip_null_from_typed_enums,
    strict_mode::patch_schema_for_strict_mode,
};

#[cfg(test)]
use schema_normalization::sanitize_schema_for_openai_compat;
