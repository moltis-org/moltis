use super::*;

/// Fallback loop limit when config is missing or invalid.
pub(crate) const DEFAULT_AGENT_MAX_ITERATIONS: usize = 25;
pub(crate) const TOOL_RESULT_COMPACTION_RATIO_PERCENT: usize = 75;
pub(crate) const PREEMPTIVE_OVERFLOW_RATIO_PERCENT: usize = 90;
pub(crate) const TOOL_RESULT_COMPACTION_PLACEHOLDER: &str =
    "[tool result compacted to preserve context budget]";
pub(crate) const TOOL_RESULT_COMPACTION_MIN_BYTES: usize = 200;

pub(crate) fn resolve_agent_max_iterations(configured: usize) -> usize {
    if configured == 0 {
        warn!(
            default = DEFAULT_AGENT_MAX_ITERATIONS,
            "tools.agent_max_iterations was 0; falling back to default"
        );
        return DEFAULT_AGENT_MAX_ITERATIONS;
    }
    configured
}

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

pub(super) fn streaming_tool_call_message_content(
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
pub(super) fn estimate_prompt_text_tokens(text: &str) -> usize {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return 0;
    }
    trimmed.len().div_ceil(4).max(1)
}

#[must_use]
pub(super) fn estimate_message_tokens(message: &ChatMessage) -> usize {
    estimate_prompt_text_tokens(&message.to_openai_value().to_string())
}

#[must_use]
pub(super) fn estimate_prompt_tokens(
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
pub(super) fn has_tool_result_messages(messages: &[ChatMessage]) -> bool {
    messages
        .iter()
        .any(|message| matches!(message, ChatMessage::Tool { .. }))
}

pub(super) fn compact_tool_results_newest_first_in_place(
    messages: &mut [ChatMessage],
    tokens_needed: usize,
) -> usize {
    if tokens_needed == 0 {
        return 0;
    }

    let mut reduced = 0;
    for message in messages.iter_mut().rev() {
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

pub(super) fn enforce_tool_result_context_budget(
    messages: &mut [ChatMessage],
    tool_schemas: &[serde_json::Value],
    context_window: u32,
) -> Result<(), AgentRunError> {
    let context_window = context_window as usize;
    if context_window == 0 || !has_tool_result_messages(messages) {
        return Ok(());
    }

    let compaction_budget =
        context_window.saturating_mul(TOOL_RESULT_COMPACTION_RATIO_PERCENT) / 100;
    let overflow_budget = context_window.saturating_mul(PREEMPTIVE_OVERFLOW_RATIO_PERCENT) / 100;
    let current_tokens = estimate_prompt_tokens(messages, tool_schemas);

    if current_tokens > compaction_budget {
        let needed = current_tokens.saturating_sub(compaction_budget);
        let reduced = compact_tool_results_newest_first_in_place(messages, needed);
        debug!(
            current_tokens,
            compaction_budget,
            overflow_budget,
            needed,
            reduced,
            "compacted newest tool results to preserve prompt budget"
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

/// Error patterns that indicate the context window has been exceeded.
pub(super) const CONTEXT_WINDOW_PATTERNS: &[&str] = &[
    "context_length_exceeded",
    "context_window_exceeded",
    "context_window_exceeded",
    "max_tokens",
    "too many tokens",
    "request too large",
    "maximum context length",
    "context window",
    "token limit",
    "input too long",
    "input_too_long",
    "content_too_large",
    "request_too_large",
];

/// Check if an error message indicates a context window overflow.
pub(super) fn is_context_window_error(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    CONTEXT_WINDOW_PATTERNS.iter().any(|p| lower.contains(p))
}

/// Error patterns that indicate a transient server error worth retrying.
pub(super) const RETRYABLE_SERVER_PATTERNS: &[&str] = &[
    "http 500",
    "http 502",
    "http 503",
    "http 529",
    "server_error",
    "internal server error",
    "overloaded",
    "bad gateway",
    "service unavailable",
    "the server had an error processing your request",
];

/// Check if an error looks like a transient provider failure that may
/// succeed on retry (5xx, overloaded, etc.).
pub(super) fn is_retryable_server_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    RETRYABLE_SERVER_PATTERNS.iter().any(|p| lower.contains(p))
}

/// Error patterns that indicate provider-side rate limiting.
pub(super) const RATE_LIMIT_PATTERNS: &[&str] = &[
    "http 429",
    "status=429",
    "status 429",
    "status: 429",
    "too many requests",
    "rate limit",
    "rate_limit",
];

pub(super) fn is_rate_limit_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    RATE_LIMIT_PATTERNS.iter().any(|p| lower.contains(p))
}

/// Error patterns that indicate the account is out of credits/quota.
/// These are not retryable in the short term and should surface directly.
pub(super) const BILLING_QUOTA_PATTERNS: &[&str] = &[
    "insufficient_quota",
    "quota exceeded",
    "current quota",
    "billing details",
    "billing limit",
    "credit balance",
];

pub(super) fn is_billing_quota_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    BILLING_QUOTA_PATTERNS.iter().any(|p| lower.contains(p))
}

/// Base delay for non-rate-limit transient retries.
pub(super) const SERVER_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(2);

/// Rate-limit retries use exponential backoff with a cap.
pub(super) const RATE_LIMIT_INITIAL_RETRY_MS: u64 = 2_000;
pub(super) const RATE_LIMIT_MAX_RETRY_MS: u64 = 60_000;
pub(super) const RATE_LIMIT_MAX_RETRIES: u8 = 10;

pub(super) fn next_rate_limit_retry_ms(previous_ms: Option<u64>) -> u64 {
    previous_ms
        .map(|ms| ms.saturating_mul(2))
        .unwrap_or(RATE_LIMIT_INITIAL_RETRY_MS)
        .clamp(RATE_LIMIT_INITIAL_RETRY_MS, RATE_LIMIT_MAX_RETRY_MS)
}

pub(super) fn parse_retry_delay_ms_from_fragment(
    fragment: &str,
    unit_default_ms: bool,
    max_ms: u64,
) -> Option<u64> {
    let start = fragment.find(|c: char| c.is_ascii_digit())?;
    let tail = &fragment[start..];
    let digits_len = tail.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits_len == 0 {
        return None;
    }
    let amount = tail[..digits_len].parse::<u64>().ok()?;
    let unit = tail[digits_len..].trim_start();

    let ms = if unit.starts_with("ms") || unit.starts_with("millisecond") {
        amount
    } else if unit.starts_with("sec") || unit.starts_with("second") || unit.starts_with('s') {
        amount.saturating_mul(1_000)
    } else if unit.starts_with("min") || unit.starts_with("minute") || unit.starts_with('m') {
        amount.saturating_mul(60_000)
    } else if unit_default_ms {
        amount
    } else {
        amount.saturating_mul(1_000)
    };

    Some(ms.clamp(1, max_ms))
}

/// Extract retry delay hints embedded in provider error messages.
///
/// Supports patterns like:
/// - `retry_after_ms=1234`
/// - `Retry-After: 30`
/// - `retry after 30s`
/// - `retry in 45 seconds`
pub(super) fn extract_retry_after_ms(msg: &str, max_ms: u64) -> Option<u64> {
    let lower = msg.to_ascii_lowercase();
    for (needle, default_ms) in [
        ("retry_after_ms=", true),
        ("retry-after-ms=", true),
        ("retry_after=", false),
        ("retry-after:", false),
        ("retry after ", false),
        ("retry in ", false),
    ] {
        if let Some(idx) = lower.find(needle) {
            let fragment = &lower[idx + needle.len()..];
            if let Some(ms) = parse_retry_delay_ms_from_fragment(fragment, default_ms, max_ms) {
                return Some(ms);
            }
        }
    }
    None
}

pub(super) fn next_retry_delay_ms(
    msg: &str,
    server_retries_remaining: &mut u8,
    rate_limit_retries_remaining: &mut u8,
    rate_limit_backoff_ms: &mut Option<u64>,
) -> Option<u64> {
    // Account/billing quota exhaustion is not transient; don't auto-retry.
    if is_billing_quota_error(msg) {
        return None;
    }

    if is_rate_limit_error(msg) {
        if *rate_limit_retries_remaining == 0 {
            return None;
        }
        *rate_limit_retries_remaining -= 1;

        // Keep exponential state advancing even when the provider gives a
        // Retry-After hint, so future retries remain bounded and predictable.
        let current_backoff = *rate_limit_backoff_ms;
        *rate_limit_backoff_ms = Some(next_rate_limit_retry_ms(current_backoff));

        let hinted_ms = extract_retry_after_ms(msg, RATE_LIMIT_MAX_RETRY_MS);
        let delay_ms = hinted_ms
            .or(*rate_limit_backoff_ms)
            .unwrap_or(RATE_LIMIT_INITIAL_RETRY_MS);
        return Some(delay_ms.clamp(1, RATE_LIMIT_MAX_RETRY_MS));
    }

    if is_retryable_server_error(msg) {
        if *server_retries_remaining == 0 {
            return None;
        }
        *server_retries_remaining -= 1;
        return Some(SERVER_RETRY_DELAY.as_millis() as u64);
    }

    None
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
    /// Sum of usage across all LLM requests in this run.
    pub usage: Usage,
    /// Usage for the final LLM request in this run.
    pub request_usage: Usage,
    pub raw_llm_responses: Vec<serde_json::Value>,
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

/// Detect an explicit shell command in the latest user turn.
///
/// Only `/sh ...` commands are treated as explicit shell execution requests.
/// This keeps normal chat turns (`hey`, `hello`, etc.) out of the forced-exec path.
///
/// Supported forms:
/// - `/sh pwd`
/// - `/sh@mybot uname -a`
pub(super) fn explicit_shell_command_from_user_content(
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
