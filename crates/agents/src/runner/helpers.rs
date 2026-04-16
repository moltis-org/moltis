//! Shared types, constants, and utility functions for the agent runner.

use std::borrow::Cow;

use tracing::{info, warn};

use moltis_common::hooks::{ChannelBinding, HookAction, HookPayload, HookRegistry};

use crate::{
    model::{ChatMessage, ToolCall, Usage, UserContent},
    response_sanitizer::clean_response,
    tool_loop_detector::{
        LoopDetectorAction, ToolLoopDetector, format_intervention_message,
        format_strip_tools_message,
    },
};

// ── Constants ───────────────────────────────────────────────────────────

pub(crate) const TOOL_RESULT_COMPACTION_PLACEHOLDER: &str =
    "[tool result compacted to preserve context budget]";
pub(crate) const TOOL_RESULT_COMPACTION_MIN_BYTES: usize = 200;

pub(crate) const MALFORMED_TOOL_RETRY_PROMPT: &str = "Your tool call was malformed. Retry with exact format:\n\
     ```tool_call\n{\"tool\": \"name\", \"arguments\": {...}}\n```";
pub(crate) const EMPTY_TOOL_NAME_RETRY_PROMPT: &str = "Your structured tool call had an empty tool name. Retry the same tool call using the intended tool's exact name and the same arguments.";

/// Nudge sent to the model when auto-continue fires after it stopped mid-task
/// without emitting a substantive final answer.
///
/// Deliberately avoids phrasing like "provide a brief final answer" because
/// that invites the model to overwrite an already-emitted long response with
/// a terse summary (see GH #628).
pub(crate) const AUTO_CONTINUE_NUDGE: &str = "Your previous response ended without tool calls and without a final answer. \
     If there are still steps to run, continue executing them. \
     Otherwise reply with exactly: done";

/// Minimum character count (after trimming) that qualifies an assistant text
/// response as a "substantive final answer" — at or above this length the
/// auto-continue nudge is suppressed because the model has clearly finished
/// talking and nudging it risks losing the answer (GH #628).
pub(crate) const AUTO_CONTINUE_SUBSTANTIVE_TEXT_THRESHOLD: usize = 40;

// ── Typed errors and result ─────────────────────────────────────────────

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
    /// Sum of usage across all LLM requests in this run.
    pub usage: Usage,
    /// Usage for the final LLM request in this run.
    pub request_usage: Usage,
    pub raw_llm_responses: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct UsageAccumulator {
    total: Usage,
    request: Usage,
}

impl UsageAccumulator {
    pub(crate) fn record_request(&mut self, usage: Usage) {
        self.total.saturating_add_assign(&usage);
        self.request = usage;
    }

    pub(crate) fn total(&self) -> Usage {
        self.total.clone()
    }

    pub(crate) fn request(&self) -> Usage {
        self.request.clone()
    }
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
    /// A transient LLM error occurred and the runner will retry.
    RetryingAfterError {
        error: String,
        delay_ms: u64,
    },
    /// The model stopped without tool calls but iteration budget remains;
    /// the runner is automatically re-prompting.
    AutoContinue {
        iteration: usize,
        max_iterations: usize,
    },
    /// A tool call was rejected by pre-dispatch schema validation before the
    /// tool's `execute` method ran. Used in place of the usual
    /// `ToolCallStart`/`ToolCallEnd` pair for rejected calls so the UI does
    /// not render a misleading "executing" status for a call that never
    /// actually executed.
    ToolCallRejected {
        id: String,
        name: String,
        arguments: serde_json::Value,
        error: String,
    },
    /// The loop detector fired after observing repeated identical tool-call
    /// failures. `stage` is 1 for the nudge/directive intervention and 2 for
    /// the stronger tool-stripping escalation (see issue #658).
    LoopInterventionFired {
        stage: u8,
        tool_name: String,
    },
}

// ── Shared helper functions ─────────────────────────────────────────────

/// Sanitize a tool name from model output.
///
/// Handles quirks from various LLM providers:
/// 1. Trims whitespace
/// 2. Strips surrounding double quotes (some models quote tool names)
/// 3. Strips `functions_` prefix (OpenAI legacy artifact from some models)
/// 4. Strips trailing `_\d+` suffix (parallel-call indexing from some models,
///    e.g. Kimi K2.5 via OpenRouter sends `exec_2`, `browser_4`)
pub(crate) fn sanitize_tool_name(name: &str) -> Cow<'_, str> {
    let trimmed = name.trim();
    let unquoted = trimmed
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(trimmed);

    // Strip `functions_` prefix (OpenAI legacy artifact from some models).
    // INVARIANT: no registered tool name starts with "functions_".
    let without_prefix = unquoted.strip_prefix("functions_").unwrap_or(unquoted);

    // Strip trailing `_\d+` suffix (parallel-call indexing from some models).
    // INVARIANT: no registered tool name ends with `_\d+` (a purely numeric segment after the last underscore).
    let cleaned = without_prefix
        .rfind('_')
        .and_then(|pos| {
            let suffix = &without_prefix[pos + 1..];
            if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) && pos > 0 {
                Some(&without_prefix[..pos])
            } else {
                None
            }
        })
        .unwrap_or(without_prefix);

    if cleaned == name {
        Cow::Borrowed(name)
    } else {
        Cow::Owned(cleaned.to_string())
    }
}

fn build_after_llm_call_payload(
    session_key: &str,
    provider: &str,
    model: &str,
    text: Option<String>,
    tool_calls: &[ToolCall],
    usage: &Usage,
    iteration: usize,
) -> HookPayload {
    let tool_calls = tool_calls
        .iter()
        .map(|tc| {
            serde_json::json!({
                "id": tc.id,
                "name": tc.name,
                "arguments": tc.arguments,
            })
        })
        .collect();

    HookPayload::AfterLLMCall {
        session_key: session_key.to_string(),
        provider: provider.to_string(),
        model: model.to_string(),
        text,
        tool_calls,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        iteration,
    }
}

pub(crate) async fn dispatch_after_llm_call_hook(
    hook_registry: Option<&std::sync::Arc<HookRegistry>>,
    session_key: &str,
    provider: &str,
    model: &str,
    text: Option<String>,
    tool_calls: &[ToolCall],
    usage: &Usage,
    iteration: usize,
) -> Result<(), AgentRunError> {
    let Some(hooks) = hook_registry else {
        return Ok(());
    };

    let payload = build_after_llm_call_payload(
        session_key,
        provider,
        model,
        text,
        tool_calls,
        usage,
        iteration,
    );

    match hooks.dispatch(&payload).await {
        Ok(HookAction::Block(reason)) => {
            warn!(reason = %reason, "LLM response blocked by AfterLLMCall hook");
            Err(AgentRunError::Other(anyhow::anyhow!(
                "blocked by AfterLLMCall hook: {reason}"
            )))
        },
        Ok(HookAction::ModifyPayload(_)) => {
            tracing::debug!("AfterLLMCall ModifyPayload ignored (response is typed)");
            Ok(())
        },
        Ok(HookAction::Continue) => Ok(()),
        Err(e) => {
            warn!(error = %e, "AfterLLMCall hook dispatch failed");
            Ok(())
        },
    }
}

pub(crate) fn finish_agent_run(
    final_text: String,
    iterations: usize,
    tool_calls_made: usize,
    usage_accumulator: &UsageAccumulator,
    raw_llm_responses: Vec<serde_json::Value>,
) -> AgentRunResult {
    AgentRunResult {
        text: clean_response(&final_text),
        iterations,
        tool_calls_made,
        usage: usage_accumulator.total(),
        request_usage: usage_accumulator.request(),
        raw_llm_responses,
    }
}

pub(crate) fn legacy_public_tool_alias(name: &str) -> Option<&str> {
    name.strip_suffix("_wasm").filter(|base| !base.is_empty())
}

pub(crate) fn resolve_tool_lookup<'a>(
    tools: &crate::tool_registry::ToolRegistry,
    name: &'a str,
) -> (
    Option<std::sync::Arc<dyn crate::tool_registry::AgentTool>>,
    Cow<'a, str>,
) {
    if let Some(alias) = legacy_public_tool_alias(name)
        && let Some(tool) = tools.get(alias)
    {
        return (Some(tool), Cow::Owned(alias.to_string()));
    }

    (tools.get(name), Cow::Borrowed(name))
}

/// Detect an explicit shell command in the latest user turn.
///
/// Only `/sh ...` commands are treated as explicit shell execution requests.
/// This keeps normal chat turns (`hey`, `hello`, etc.) out of the forced-exec path.
///
/// Supported forms:
/// - `/sh pwd`
/// - `/sh@mybot uname -a`
pub(crate) fn explicit_shell_command_from_user_content(
    user_content: &UserContent,
) -> Option<String> {
    let text = match user_content {
        UserContent::Text(text) => text.trim(),
        UserContent::Multimodal(_) => return None,
    };

    if text.is_empty() || text.len() > 4096 || text.contains('\n') || text.contains('\r') {
        return None;
    }

    let rest = text.strip_prefix('/')?;
    let split_idx = rest.find(char::is_whitespace)?;
    let head = &rest[..split_idx];
    let command = rest[split_idx..].trim_start();
    if command.is_empty() {
        return None;
    }

    let head_lower = head.to_ascii_lowercase();
    let is_sh_prefix = if head_lower == "sh" {
        true
    } else {
        head_lower
            .strip_prefix("sh@")
            .is_some_and(|mention| !mention.is_empty())
    };

    if !is_sh_prefix {
        return None;
    }

    Some(command.to_string())
}

/// Returns `true` if `text` (trimmed) is long enough to be considered a real
/// final answer rather than an empty/terse pause.
#[must_use]
pub(crate) fn is_substantive_answer_text(text: &str) -> bool {
    text.trim().chars().count() >= AUTO_CONTINUE_SUBSTANTIVE_TEXT_THRESHOLD
}

pub(crate) fn find_empty_tool_name_call(tool_calls: &[ToolCall]) -> Option<&ToolCall> {
    tool_calls
        .iter()
        .find(|tc| sanitize_tool_name(&tc.name).is_empty())
}

pub(crate) fn has_named_tool_call(tool_calls: &[ToolCall]) -> bool {
    tool_calls
        .iter()
        .any(|tc| !sanitize_tool_name(&tc.name).is_empty())
}

pub(crate) fn empty_tool_name_retry_prompt(tool_call: &ToolCall) -> String {
    format!(
        "{EMPTY_TOOL_NAME_RETRY_PROMPT}\nExact arguments JSON:\n{}",
        tool_call.arguments
    )
}

pub(crate) fn record_answer_text(last_answer_text: &mut String, text: &Option<String>) {
    if let Some(text) = text.as_ref()
        && !text.is_empty()
    {
        last_answer_text.clone_from(text);
    }
}

pub(crate) fn streaming_tool_call_message_content(
    last_answer_text: &mut String,
    accumulated_text: &str,
    accumulated_reasoning: &str,
) -> Option<String> {
    if !accumulated_reasoning.is_empty() {
        Some(accumulated_reasoning.to_string())
    } else if !accumulated_text.is_empty() {
        last_answer_text.clear();
        last_answer_text.push_str(accumulated_text);
        Some(accumulated_text.to_string())
    } else {
        None
    }
}

#[must_use]
pub(crate) fn estimate_prompt_text_tokens(text: &str) -> usize {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return 0;
    }
    trimmed.len().div_ceil(4).max(1)
}

#[must_use]
pub(crate) fn estimate_message_tokens(message: &ChatMessage) -> usize {
    estimate_prompt_text_tokens(&message.to_openai_value().to_string())
}

#[must_use]
pub(crate) fn estimate_prompt_tokens(
    messages: &[ChatMessage],
    tool_schemas: &[serde_json::Value],
) -> usize {
    let message_tokens: usize = messages.iter().map(estimate_message_tokens).sum();
    let tool_tokens: usize = tool_schemas
        .iter()
        .map(|schema| estimate_prompt_text_tokens(&schema.to_string()))
        .sum();
    message_tokens.saturating_add(tool_tokens)
}

#[must_use]
pub(crate) fn has_tool_result_messages(messages: &[ChatMessage]) -> bool {
    messages
        .iter()
        .any(|message| matches!(message, ChatMessage::Tool { .. }))
}

pub(crate) fn compact_tool_results_oldest_first_in_place(
    messages: &mut [ChatMessage],
    tokens_needed: usize,
) -> usize {
    if tokens_needed == 0 {
        return 0;
    }

    let mut reduced = 0;
    for message in messages.iter_mut() {
        if reduced >= tokens_needed {
            break;
        }

        let ChatMessage::Tool {
            tool_call_id,
            content,
        } = message
        else {
            continue;
        };
        if content == TOOL_RESULT_COMPACTION_PLACEHOLDER
            || content.len() < TOOL_RESULT_COMPACTION_MIN_BYTES
        {
            continue;
        }

        let tool_call_id = tool_call_id.clone();
        let original = content.clone();
        let before = estimate_message_tokens(&ChatMessage::tool(&tool_call_id, &original));
        *content = TOOL_RESULT_COMPACTION_PLACEHOLDER.to_string();
        let after = estimate_message_tokens(&ChatMessage::tool(
            &tool_call_id,
            TOOL_RESULT_COMPACTION_PLACEHOLDER,
        ));
        let saved = before.saturating_sub(after);
        if saved == 0 {
            *content = original;
            continue;
        }

        reduced = reduced.saturating_add(saved);
    }

    reduced
}

pub(crate) fn enforce_tool_result_context_budget(
    messages: &mut [ChatMessage],
    tool_schemas: &[serde_json::Value],
    context_window: u32,
    compaction_ratio: usize,
    overflow_ratio: usize,
) -> Result<(), AgentRunError> {
    let context_window = context_window as usize;
    if context_window == 0 || !has_tool_result_messages(messages) {
        return Ok(());
    }

    // Allow disabling per-iteration compaction entirely via config (ratio = 0).
    if compaction_ratio == 0 {
        let overflow_budget = context_window.saturating_mul(overflow_ratio) / 100;
        let current_tokens = estimate_prompt_tokens(messages, tool_schemas);
        if current_tokens > overflow_budget {
            return Err(AgentRunError::ContextWindowExceeded(format!(
                "preemptive context overflow: estimated prompt size {current_tokens} tokens \
                 exceeds {overflow_budget} token budget (compaction disabled)"
            )));
        }
        return Ok(());
    }
    let compaction_budget = context_window.saturating_mul(compaction_ratio) / 100;
    let overflow_budget = context_window.saturating_mul(overflow_ratio) / 100;
    let current_tokens = estimate_prompt_tokens(messages, tool_schemas);

    if current_tokens > compaction_budget {
        let needed = current_tokens.saturating_sub(compaction_budget);
        let reduced = compact_tool_results_oldest_first_in_place(messages, needed);
        tracing::debug!(
            current_tokens,
            compaction_budget,
            overflow_budget,
            needed,
            reduced,
            "compacted oldest tool results to preserve prompt budget"
        );
    }

    let post_compaction_tokens = estimate_prompt_tokens(messages, tool_schemas);
    if post_compaction_tokens > overflow_budget {
        return Err(AgentRunError::ContextWindowExceeded(format!(
            "preemptive context overflow: estimated prompt size {post_compaction_tokens} tokens exceeds {overflow_budget} token budget after tool-result compaction"
        )));
    }

    Ok(())
}

pub(crate) fn channel_binding_from_tool_context(
    session_key: &str,
    tool_context: Option<&serde_json::Value>,
) -> Option<ChannelBinding> {
    let channel_value = tool_context.and_then(|ctx| ctx.get("_channel"))?;
    match serde_json::from_value(channel_value.clone()) {
        Ok(binding) => Some(binding),
        Err(error) => {
            warn!(
                error = %error,
                session = %session_key,
                "failed to parse _channel tool context for hooks; ignoring channel provenance"
            );
            None
        },
    }
}

/// Consume the detector's post-batch action (if any) and apply it to the
/// runner state: push the directive user message into `messages`, emit the
/// `LoopInterventionFired` UI event, and set `strip_tools_next_iter` when
/// stage 2 fires. Shared by the streaming and non-streaming loops (issue
/// #658).
pub(crate) fn apply_loop_detector_intervention(
    loop_detector: &mut ToolLoopDetector,
    messages: &mut Vec<ChatMessage>,
    strip_tools_next_iter: &mut bool,
    on_event: Option<&OnEvent>,
) {
    if !loop_detector.is_enabled() {
        return;
    }
    match loop_detector.consume_pending_action() {
        LoopDetectorAction::None => {},
        LoopDetectorAction::InjectNudge => {
            let window = loop_detector.window_snapshot();
            let stuck_tool = window
                .first()
                .map(|fp| fp.tool_name.clone())
                .unwrap_or_default();
            let intervention = format_intervention_message(&window);
            info!(
                tool = %stuck_tool,
                "loop detector fired (stage 1): injecting directive intervention"
            );
            if let Some(cb) = on_event {
                cb(RunnerEvent::LoopInterventionFired {
                    stage: 1,
                    tool_name: stuck_tool,
                });
            }
            messages.push(ChatMessage::user(intervention));
        },
        LoopDetectorAction::StripTools => {
            let stuck_tool = loop_detector
                .window_snapshot()
                .first()
                .map(|fp| fp.tool_name.clone())
                .unwrap_or_default();
            info!(
                tool = %stuck_tool,
                "loop detector fired (stage 2): stripping tools for next iteration"
            );
            if let Some(cb) = on_event {
                cb(RunnerEvent::LoopInterventionFired {
                    stage: 2,
                    tool_name: stuck_tool,
                });
            }
            messages.push(ChatMessage::user(format_strip_tools_message()));
            *strip_tools_next_iter = true;
        },
    }
}
