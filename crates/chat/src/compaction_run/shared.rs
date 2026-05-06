//! Boundary computation, pruning, and tool-pair repair helpers.
//!
//! All the primitives that every compaction strategy in this module needs.
//! Strategies import from here rather than duplicating logic, and the
//! shared code gets its own focused tests.

use {
    moltis_config::CompactionConfig,
    moltis_sessions::{MessageContent, PersistedMessage},
    serde_json::Value,
};

use crate::types::now_ms;

use super::CompactionRunError;

/// Placeholder text injected when a bulky tool-result is pruned in place.
pub(super) const PRUNED_TOOL_PLACEHOLDER: &str = "[Old tool output cleared to save context space]";

// ── Boundary computation ──────────────────────────────────────────────

/// Head/tail boundaries computed by [`compute_boundaries`].
///
/// `head_end` is the exclusive index marking the end of the verbatim head
/// region; `tail_start` is the inclusive index marking the beginning of the
/// verbatim tail. The middle slice is `history[head_end..tail_start]` and
/// may be empty when the session is already small enough to fit in the
/// retained budget.
pub(super) struct HeadTailBounds {
    pub(super) head_end: usize,
    pub(super) tail_start: usize,
    pub(super) protect_head: usize,
    pub(super) protect_tail_min: usize,
}

/// Compute the head / middle / tail boundaries for a recency-aware strategy.
///
/// Never splits a tool_use / tool_result group — the tail boundary slides
/// forward past any consecutive tool-result run so the kept slice always
/// starts on a non-tool message (or the end of the history).
pub(super) fn compute_boundaries(
    history: &[Value],
    config: &CompactionConfig,
    context_window: u32,
) -> HeadTailBounds {
    let n = history.len();
    let protect_head = (config.protect_head as usize).min(n);
    let protect_tail_min = (config.protect_tail_min as usize).min(n.saturating_sub(protect_head));

    // Convert fractional budgets to a concrete token count. Clamp everything
    // into a sane range so wildly misconfigured ratios don't divide by zero.
    let threshold = config.threshold_percent.clamp(0.1, 0.95);
    let tail_ratio = config.tail_budget_ratio.clamp(0.05, 0.80);
    let tail_budget_tokens =
        ((f64::from(context_window) * f64::from(threshold) * f64::from(tail_ratio)).round() as u64)
            .max(1);

    // Walk backward from the end until either the budget is consumed or
    // the floor (protect_tail_min) is satisfied. Whichever covers more
    // messages wins, so small sessions still keep the floor even when
    // their tail is tiny and large sessions honour the token budget
    // when the floor is too small.
    //
    // This is a **contiguous** walk: the tail is always a suffix of the
    // history, never a cherry-picked set. When a message breaks the
    // budget AND the floor is satisfied, the walk stops immediately —
    // we deliberately don't skip over a large message to pack smaller
    // earlier ones into the tail, because the resulting "tail" would
    // have a gap and no longer be contiguous. Reviewers have asked
    // whether this greedy break loses tokens; it does, but only
    // relative to a non-contiguous "maximum packing" strategy, which
    // isn't what we want here.
    let head_end = protect_head;
    let mut accumulated: u64 = 0;
    let mut tail_start = n;
    for idx in (head_end..n).rev() {
        let msg_tokens = message_tokens(&history[idx]);
        let keep_for_budget = accumulated + msg_tokens <= tail_budget_tokens;
        let keep_for_floor = (n - idx) <= protect_tail_min;
        if keep_for_budget || keep_for_floor {
            accumulated += msg_tokens;
            tail_start = idx;
        } else {
            break;
        }
    }

    // Never split a tool_use / tool_result group: if the boundary falls on a
    // tool-result message, walk forward past the whole group (the parent
    // assistant message and any siblings).
    let tail_start = align_boundary_forward_past_tool_group(history, tail_start);

    HeadTailBounds {
        head_end,
        tail_start,
        protect_head,
        protect_tail_min,
    }
}

/// Prune bulky tool-result content in anything older than the last
/// `protect_tail_min * 3` messages, then repair orphaned tool_call /
/// tool_result pairs so strict providers accept the retry.
pub(super) fn finalize_kept(
    mut kept: Vec<Value>,
    config: &CompactionConfig,
    protect_tail_min: usize,
) -> Result<Vec<Value>, CompactionRunError> {
    let tool_prune_frontier = kept.len().saturating_sub(protect_tail_min * 3);
    prune_tool_results_before(
        &mut kept,
        tool_prune_frontier,
        config.tool_prune_char_threshold,
    );
    sanitize_tool_pairs(kept)
}

/// Handle the "head and tail already cover everything" fallback.
///
/// Prune bulky tool-result content in place; if nothing actually changed,
/// return [`CompactionRunError::TooSmallToCompact`] so the caller can fall
/// back to a different mode instead of retrying forever.
pub(super) fn in_place_prune_or_err(
    history: &[Value],
    config: &CompactionConfig,
    bounds: &HeadTailBounds,
) -> Result<Vec<Value>, CompactionRunError> {
    let mut kept: Vec<Value> = history.to_vec();
    let kept_len = kept.len();
    let pruned = prune_tool_results_before(&mut kept, kept_len, config.tool_prune_char_threshold);
    if pruned == 0 {
        return Err(CompactionRunError::TooSmallToCompact {
            messages: history.len(),
            head: bounds.protect_head,
            tail: bounds.protect_tail_min,
        });
    }
    sanitize_tool_pairs(kept)
}

// ── Token estimation + message shape helpers ─────────────────────────

/// Rough token count for a persisted message.
///
/// Uses the same bytes/4 heuristic as the chat crate's existing estimator
/// plus a 10-token overhead for role/metadata framing. Covers the common
/// shapes without pulling in a tokenizer dependency.
pub(super) fn message_tokens(message: &Value) -> u64 {
    const META_OVERHEAD: u64 = 10;
    let mut bytes: usize = 0;

    // Top-level content: string or array of content blocks.
    if let Some(text) = message.get("content").and_then(Value::as_str) {
        bytes += text.len();
    } else if let Some(blocks) = message.get("content").and_then(Value::as_array) {
        for block in blocks {
            if let Some(text) = block.get("text").and_then(Value::as_str) {
                bytes += text.len();
            } else if let Some(url) = block
                .get("image_url")
                .and_then(|iu| iu.get("url"))
                .and_then(Value::as_str)
            {
                bytes += url.len();
            }
        }
    }

    // Tool call arguments on assistant messages.
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            if let Some(args) = call
                .get("function")
                .and_then(|f| f.get("arguments"))
                .and_then(Value::as_str)
            {
                bytes += args.len();
            }
        }
    }

    // Tool-result structured fields.
    if let Some(result) = message.get("result") {
        bytes += serde_json::to_string(result).map(|s| s.len()).unwrap_or(0);
    }
    if let Some(error) = message.get("error").and_then(Value::as_str) {
        bytes += error.len();
    }

    ((bytes as u64) / 4) + META_OVERHEAD
}

/// True if the message is a tool/tool_result shape.
pub(super) fn is_tool_role_value(message: &Value) -> bool {
    matches!(
        message.get("role").and_then(Value::as_str),
        Some("tool" | "tool_result")
    )
}

/// Extract tool-call IDs from an assistant message's `tool_calls` array.
pub(super) fn assistant_tool_call_ids(message: &Value) -> Vec<String> {
    let Some(calls) = message.get("tool_calls").and_then(Value::as_array) else {
        return Vec::new();
    };
    calls
        .iter()
        .filter_map(|c| c.get("id").and_then(Value::as_str).map(str::to_string))
        .collect()
}

/// Extract `tool_call_id` from a tool/tool_result message.
pub(super) fn tool_result_call_id(message: &Value) -> Option<&str> {
    message.get("tool_call_id").and_then(Value::as_str)
}

/// Walk forward past a consecutive run of tool/tool_result messages so the
/// tail boundary never starts mid-group.
fn align_boundary_forward_past_tool_group(history: &[Value], mut idx: usize) -> usize {
    while idx < history.len() && is_tool_role_value(&history[idx]) {
        idx += 1;
    }
    idx
}

// ── Tool-result pruning ──────────────────────────────────────────────

/// Replace oversized tool-result content with [`PRUNED_TOOL_PLACEHOLDER`] in
/// messages before `end_exclusive`, returning the number of messages pruned.
///
/// Preserves lightweight tool results (under the threshold) and everything
/// at or after the protected tail region. Handles both the `role = "tool"`
/// shape (string `content`) and the `role = "tool_result"` shape
/// (`result` JSON + optional `error` string).
pub(super) fn prune_tool_results_before(
    messages: &mut [Value],
    end_exclusive: usize,
    threshold_chars: u32,
) -> usize {
    let threshold = threshold_chars as usize;
    let mut pruned = 0;

    for msg in messages.iter_mut().take(end_exclusive) {
        if !is_tool_role_value(msg) {
            continue;
        }
        if prune_single_tool_result(msg, threshold) {
            pruned += 1;
        }
    }

    pruned
}

/// Replace oversized content on a single tool/tool_result message.
/// Returns `true` if anything was rewritten.
fn prune_single_tool_result(message: &mut Value, threshold: usize) -> bool {
    let mut changed = false;

    // `role = "tool"`: plain string content. Skip if already pruned or
    // under the threshold.
    if let Some(content) = message.get("content").and_then(Value::as_str) {
        if content == PRUNED_TOOL_PLACEHOLDER {
            return false;
        }
        if content.len() > threshold
            && let Some(obj) = message.as_object_mut()
        {
            obj.insert(
                "content".to_string(),
                Value::String(PRUNED_TOOL_PLACEHOLDER.to_string()),
            );
            changed = true;
        }
    }

    // `role = "tool_result"`: structured `result` + optional `error`.
    let result_too_big = message
        .get("result")
        .is_some_and(|r| serde_json::to_string(r).map(|s| s.len()).unwrap_or(0) > threshold);
    let error_too_big = message
        .get("error")
        .and_then(Value::as_str)
        .is_some_and(|e| e.len() > threshold);

    if (result_too_big || error_too_big)
        && let Some(obj) = message.as_object_mut()
    {
        if result_too_big {
            obj.insert(
                "result".to_string(),
                Value::String(PRUNED_TOOL_PLACEHOLDER.to_string()),
            );
            changed = true;
        }
        if error_too_big {
            obj.insert(
                "error".to_string(),
                Value::String(PRUNED_TOOL_PLACEHOLDER.to_string()),
            );
            changed = true;
        }
    }

    changed
}

// ── Tool-pair integrity ──────────────────────────────────────────────

/// Repair orphaned tool_call / tool_result pairs after compaction.
///
/// Two failure modes, both rejected by strict providers (Anthropic,
/// OpenAI strict mode):
///
/// 1. A tool result references a `tool_call_id` whose parent assistant
///    `tool_call` was dropped during pruning. → removed.
/// 2. An assistant `tool_call` has no matching tool result (the result was
///    dropped). → a stub tool result is inserted after the assistant
///    message so the pairing is well-formed.
///
/// Adapted from hermes-agent's `_sanitize_tool_pairs` and openclaw's
/// `repairToolUseResultPairing`.
pub(super) fn sanitize_tool_pairs(messages: Vec<Value>) -> Result<Vec<Value>, CompactionRunError> {
    use std::collections::HashSet;

    // Pass 1: collect surviving tool_call IDs from assistant messages.
    let mut surviving_call_ids: HashSet<String> = HashSet::new();
    for msg in &messages {
        if msg.get("role").and_then(Value::as_str) == Some("assistant") {
            for id in assistant_tool_call_ids(msg) {
                surviving_call_ids.insert(id);
            }
        }
    }

    // Pass 2: collect the call IDs referenced by tool results.
    let mut result_call_ids: HashSet<String> = HashSet::new();
    for msg in &messages {
        if is_tool_role_value(msg)
            && let Some(id) = tool_result_call_id(msg)
        {
            result_call_ids.insert(id.to_string());
        }
    }

    // Pass 3: drop tool results whose call_id is no longer in the history.
    let orphaned: HashSet<String> = result_call_ids
        .difference(&surviving_call_ids)
        .cloned()
        .collect();
    let filtered: Vec<Value> = if orphaned.is_empty() {
        messages
    } else {
        messages
            .into_iter()
            .filter(|m| {
                if !is_tool_role_value(m) {
                    return true;
                }
                tool_result_call_id(m).is_none_or(|id| !orphaned.contains(id))
            })
            .collect()
    };

    // Pass 4: for every surviving assistant tool_call missing a matching
    // tool result, insert a stub tool message immediately after the parent.
    // We rebuild a new vec so we can splice stubs in the right positions.
    let mut patched: Vec<Value> = Vec::with_capacity(filtered.len());
    let mut satisfied: HashSet<String> = HashSet::new();

    // First pass to record which call IDs already have results further down
    // the history. We only need the count of surviving results here.
    for msg in &filtered {
        if is_tool_role_value(msg)
            && let Some(id) = tool_result_call_id(msg)
        {
            satisfied.insert(id.to_string());
        }
    }

    for msg in filtered {
        let is_assistant = msg.get("role").and_then(Value::as_str) == Some("assistant");
        let tool_calls = if is_assistant {
            assistant_tool_call_ids(&msg)
        } else {
            Vec::new()
        };
        patched.push(msg);
        for call_id in tool_calls {
            if !satisfied.contains(&call_id) {
                patched.push(stub_tool_result(&call_id));
                satisfied.insert(call_id);
            }
        }
    }

    Ok(patched)
}

/// Build a `role: tool` stub message for an orphaned assistant tool_call.
fn stub_tool_result(tool_call_id: &str) -> Value {
    let msg = PersistedMessage::Tool {
        tool_call_id: tool_call_id.to_string(),
        content: "[Result from earlier conversation — see context summary above]".to_string(),
        created_at: Some(now_ms()),
    };
    msg.to_value()
}

// ── Summary / marker message builders ────────────────────────────────

/// Build a single user message that replaces the dropped middle region in
/// `recency_preserving` mode.
///
/// Counts each message by role so the LLM retry has a quick sense of what
/// was elided, then notes that recent turns are preserved verbatim below.
pub(super) fn build_middle_marker(middle: &[Value]) -> Value {
    let mut users = 0usize;
    let mut assistants = 0usize;
    let mut tools = 0usize;
    for msg in middle {
        match msg.get("role").and_then(Value::as_str) {
            Some("user") => users += 1,
            Some("assistant") => assistants += 1,
            Some("tool") | Some("tool_result") => tools += 1,
            _ => {},
        }
    }

    let body = format!(
        "[Conversation Compacted]\n\n\
         {total} earlier messages were elided to save context space \
         ({users} user, {assistants} assistant, {tools} tool). \
         Recent messages are preserved verbatim below. \
         Use chat.compaction.mode = \"structured\" (when available) for a \
         full semantic summary of the omitted middle region.",
        total = middle.len(),
        users = users,
        assistants = assistants,
        tools = tools,
    );

    let msg = PersistedMessage::User {
        content: MessageContent::Text(body),
        created_at: Some(now_ms()),
        audio: None,
        documents: None,
        channel: None,
        seq: None,
        run_id: None,
    };
    msg.to_value()
}

/// Wrap a summary string in a `PersistedMessage::User` ready for
/// `replace_history`.
///
/// Using the `user` role (not `assistant`) avoids breaking strict providers
/// (e.g. llama.cpp) that require every assistant message to follow a user
/// message, and keeps the summary in the conversation turn array for
/// providers using the Responses API (which promote system messages to
/// instructions and drop them from turns).
pub(super) fn build_summary_message(body: &str) -> Value {
    let msg = PersistedMessage::User {
        content: MessageContent::Text(format!("[Conversation Summary]\n\n{body}")),
        created_at: Some(now_ms()),
        audio: None,
        documents: None,
        channel: None,
        seq: None,
        run_id: None,
    };
    msg.to_value()
}
