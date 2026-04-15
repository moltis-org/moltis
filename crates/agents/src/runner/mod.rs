//! Agent runner: LLM call loop with tool execution, retry, and streaming support.

mod helpers;
mod non_streaming;
pub mod retry;
mod streaming;
pub mod tool_result;

#[cfg(test)]
mod tests;

#[cfg(test)]
#[allow(dead_code, clippy::all)]
mod tests_legacy;

// ── Re-exports (preserve public API) ────────────────────────────────────

pub use {
    helpers::{AgentRunError, AgentRunResult, OnEvent, RunnerEvent},
    non_streaming::{run_agent, run_agent_loop, run_agent_loop_with_context},
    streaming::run_agent_loop_streaming,
    tool_result::{ExtractedImage, sanitize_tool_result, tool_result_to_content},
};

// Re-export helpers at the module level so that sibling submodules
// (`non_streaming`, `streaming`) can continue to import via `super::item_name`.
pub(crate) use helpers::{
    AUTO_CONTINUE_NUDGE, MALFORMED_TOOL_RETRY_PROMPT, UsageAccumulator,
    apply_loop_detector_intervention, channel_binding_from_tool_context,
    dispatch_after_llm_call_hook, empty_tool_name_retry_prompt, enforce_tool_result_context_budget,
    explicit_shell_command_from_user_content, find_empty_tool_name_call, finish_agent_run,
    has_named_tool_call, is_substantive_answer_text, record_answer_text, resolve_tool_lookup,
    sanitize_tool_name, streaming_tool_call_message_content,
};

// Items only consumed by test submodules (`tests`, `tests_legacy`).
#[cfg(test)]
pub(crate) use helpers::{
    TOOL_RESULT_COMPACTION_PLACEHOLDER, compact_tool_results_oldest_first_in_place,
    legacy_public_tool_alias,
};
