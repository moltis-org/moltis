// ── Attribution ──────────────────────────────────────────────────────
// Deterministic compaction extraction adapted from claw-code (ultraworkers/claw-code).
// Original source: rust/crates/runtime/src/compact.rs
// License: MIT — Copyright (c) ultraworkers
// https://github.com/ultraworkers/claw-code

//! Deterministic conversation compaction — zero LLM calls.
//!
//! Extracts structured summaries from session history by inspecting JSON message
//! values directly. Produces consistent, auditable output for the same input.

use serde_json::Value;

const COMPACT_CONTINUATION_PREAMBLE: &str = "This session is being continued from a previous conversation that ran out of context. \
    The summary below covers the earlier portion of the conversation.\n\n";
const COMPACT_RECENT_MESSAGES_NOTE: &str = "Recent messages are preserved verbatim.";
const COMPACT_DIRECT_RESUME_INSTRUCTION: &str = "Continue the conversation from where it left off without asking the user any further \
    questions. Resume directly — do not acknowledge the summary, do not recap what was \
    happening, and do not preface with continuation text.";

/// Maximum total characters for a compaction summary.
pub const SUMMARY_MAX_CHARS: usize = 1_200;
/// Maximum number of lines in a compaction summary (excluding omission notice).
pub const SUMMARY_MAX_LINES: usize = 24;
/// Maximum characters per line in a compaction summary.
pub const SUMMARY_MAX_LINE_CHARS: usize = 160;

/// Max chars for content previews (tool results, user requests, timeline).
const PREVIEW_CHARS: usize = 160;
/// Max chars for "current work" preview.
const CURRENT_WORK_CHARS: usize = 200;

// ── Public API ────────────────────────────────────────────────────────

/// Produce a structured summary string from a slice of JSON message values.
///
/// Extracts: message counts by role, tool names, key files, recent user requests,
/// pending work, current work, and a verbatim timeline.
/// Zero LLM calls — pure string/JSON inspection.
#[must_use]
pub fn summarize_messages(messages: &[Value]) -> String {
    let user_count = messages.iter().filter(|m| m["role"] == "user").count();
    let assistant_count = messages.iter().filter(|m| m["role"] == "assistant").count();
    let tool_count = messages
        .iter()
        .filter(|m| m["role"] == "tool" || m["role"] == "tool_result")
        .count();

    let tool_names = collect_unique_tool_names(messages);

    let mut lines = vec![
        "<summary>".to_string(),
        "Conversation summary:".to_string(),
        format!(
            "- Scope: {} earlier messages compacted (user={}, assistant={}, tool={}).",
            messages.len(),
            user_count,
            assistant_count,
            tool_count
        ),
    ];

    if !tool_names.is_empty() {
        lines.push(format!("- Tools mentioned: {}.", tool_names.join(", ")));
    }

    let recent_user_requests = collect_recent_role_summaries(messages, "user", 3);
    if !recent_user_requests.is_empty() {
        lines.push("- Recent user requests:".to_string());
        lines.extend(recent_user_requests.into_iter().map(|r| format!("  - {r}")));
    }

    let pending_work = infer_pending_work(messages);
    if !pending_work.is_empty() {
        lines.push("- Pending work:".to_string());
        lines.extend(pending_work.into_iter().map(|item| format!("  - {item}")));
    }

    let key_files = collect_key_files(messages);
    if !key_files.is_empty() {
        lines.push(format!("- Key files referenced: {}.", key_files.join(", ")));
    }

    if let Some(current_work) = infer_current_work(messages) {
        lines.push(format!("- Current work: {current_work}"));
    }

    lines.push("- Key timeline:".to_string());
    lines.extend(build_timeline_entries(messages));
    lines.push("</summary>".to_string());
    lines.join("\n")
}

/// Merge a previous compaction summary with a new one for re-compaction.
///
/// Preserves previous highlights, drops old timeline, adds new highlights + timeline.
#[must_use]
pub fn merge_compact_summaries(existing_summary: Option<&str>, new_summary: &str) -> String {
    let Some(existing_summary) = existing_summary else {
        return new_summary.to_string();
    };

    let prev = parse_summary_sections(existing_summary);
    let curr = parse_summary_sections(new_summary);

    let mut lines = vec!["<summary>".to_string(), "Conversation summary:".to_string()];

    if !prev.highlights.is_empty() {
        lines.push("- Previously compacted context:".to_string());
        lines.extend(prev.highlights.into_iter().map(|l| format!("  {l}")));
    }

    if !curr.highlights.is_empty() {
        lines.push("- Newly compacted context:".to_string());
        lines.extend(curr.highlights.into_iter().map(|l| format!("  {l}")));
    }

    if !curr.timeline.is_empty() {
        lines.push("- Key timeline:".to_string());
        lines.extend(curr.timeline.into_iter().map(|l| format!("  {l}")));
    }

    lines.push("</summary>".to_string());
    lines.join("\n")
}

/// Perform deterministic compaction on a message history slice.
///
/// Extracts any existing summary, summarizes remaining messages, merges them.
/// Returns the merged summary string (before budget enforcement), or `None`
/// if the input history is empty or the result would be empty.
#[must_use]
pub fn compute_compaction_summary(history: &[Value]) -> Option<String> {
    if history.is_empty() {
        return None;
    }
    let existing = extract_existing_compacted_summary(history);
    let start = usize::from(existing.is_some());
    let new_summary = summarize_messages(&history[start..]);
    let merged = merge_compact_summaries(existing.as_deref(), &new_summary);
    let trimmed = merged.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Build the synthetic continuation message injected after compaction.
///
/// Three parts: preamble + formatted summary, recent-messages note, direct-resume instruction.
#[must_use]
pub fn get_compact_continuation_message(summary: &str, recent_messages_preserved: bool) -> String {
    let mut base = format!(
        "{COMPACT_CONTINUATION_PREAMBLE}{}",
        format_compact_summary(summary)
    );

    if recent_messages_preserved {
        base.push_str("\n\n");
        base.push_str(COMPACT_RECENT_MESSAGES_NOTE);
    }

    base.push('\n');
    base.push_str(COMPACT_DIRECT_RESUME_INSTRUCTION);

    base
}

/// Format the raw `<summary>...</summary>` block for user-facing display.
///
/// Strips analysis blocks, extracts summary content, collapses blank lines.
#[must_use]
pub fn format_compact_summary(summary: &str) -> String {
    let without_analysis = strip_tag_block(summary, "analysis");
    let formatted = if let Some(content) = extract_tag_block(&without_analysis, "summary") {
        without_analysis.replacen(
            &format!("<summary>{content}</summary>"),
            &format!("Summary:\n{}", content.trim()),
            1,
        )
    } else {
        without_analysis
    };

    collapse_blank_lines(&formatted).trim().to_string()
}

/// Detect whether history[0] is a previous compaction summary.
///
/// Returns `Some(summary_text)` if detected, `None` otherwise.
#[must_use]
pub fn extract_existing_compacted_summary(history: &[Value]) -> Option<String> {
    let first = history.first()?;
    let content = first.get("content").and_then(Value::as_str)?;
    // Strip outer wrapper and continuation boilerplate so that only the
    // actual summary text is returned for re-compaction. Without this,
    // `parse_summary_sections` would capture the preamble and resume
    // instruction as highlights, corrupting the merged summary.
    let summary_text = content
        .strip_prefix("[Conversation Summary]\n\n")
        .map(|s| s.strip_prefix(COMPACT_CONTINUATION_PREAMBLE).unwrap_or(s))
        .or_else(|| {
            content
                .starts_with(COMPACT_CONTINUATION_PREAMBLE)
                .then(|| &content[COMPACT_CONTINUATION_PREAMBLE.len()..])
        })?;
    // Strip the trailing directive and (optional) recent-messages note.
    // Trimming whitespace between each strip is important: the continuation
    // builder separates the summary, note, and directive with plain newlines,
    // so a naïve `trim_end_matches` of the exact constants would miss when a
    // `\n` sits between a match and the end of the string.
    let summary_text = summary_text.trim_end();
    let summary_text = summary_text
        .strip_suffix(COMPACT_DIRECT_RESUME_INSTRUCTION)
        .unwrap_or(summary_text)
        .trim_end();
    let summary_text = summary_text
        .strip_suffix(COMPACT_RECENT_MESSAGES_NOTE)
        .unwrap_or(summary_text);
    let summary = summary_text.trim();
    if summary.is_empty() {
        return None;
    }
    Some(summary.to_string())
}

/// Compress a compaction summary to fit within budget constraints.
///
/// Enforces: max 1,200 chars total, max 24 lines, max 160 chars per line.
/// Deduplicates lines (case-insensitive), preserves headers and bullets,
/// and appends an omission notice when lines are dropped.
///
/// If the input is wrapped in `<summary>...</summary>`, the wrapper is
/// stripped before compression and re-applied after, so budget-driven
/// line dropping can never reorder the tags relative to the body.
#[must_use]
pub fn compress_summary(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        return String::new();
    }

    // Detect and strip outer <summary>/</summary> wrapper. Without this the
    // budget-enforcement pass could bucket both tags as headers and emit them
    // adjacently at the top of the result, leaving the body outside the tags.
    let (has_open, has_close, inner) = split_summary_wrapper(text);

    // Reserve budget for the wrapper tags we'll re-apply at the end. Each
    // reserved tag costs its own line plus a joining newline.
    let mut wrapper_lines = 0usize;
    let mut wrapper_chars = 0usize;
    if has_open {
        wrapper_lines += 1;
        wrapper_chars += "<summary>".len() + 1;
    }
    if has_close {
        wrapper_lines += 1;
        wrapper_chars += "</summary>".len() + 1;
    }
    let inner_max_lines = SUMMARY_MAX_LINES.saturating_sub(wrapper_lines);
    let inner_max_chars = SUMMARY_MAX_CHARS.saturating_sub(wrapper_chars);

    let compressed = compress_body(inner, inner_max_chars, inner_max_lines);

    match (has_open, has_close) {
        (true, true) => format!("<summary>\n{compressed}\n</summary>"),
        (true, false) => format!("<summary>\n{compressed}"),
        (false, true) => format!("{compressed}\n</summary>"),
        (false, false) => compressed,
    }
}

/// Detect the `<summary>`/`</summary>` wrapper on a trimmed summary string.
///
/// Returns `(has_open, has_close, inner)` where `inner` has had any matched
/// wrapper lines removed.
fn split_summary_wrapper(text: &str) -> (bool, bool, &str) {
    let (has_open, after_open) = text
        .strip_prefix("<summary>\n")
        .map_or((false, text), |rest| (true, rest));
    let (has_close, inner) = after_open
        .strip_suffix("\n</summary>")
        .map_or((false, after_open), |rest| (true, rest));
    (has_open, has_close, inner)
}

/// Budget-enforce a plain (already-unwrapped) compaction body.
///
/// Same semantics as the public wrapper but with explicit limits so the
/// caller can reserve space for any outer wrapping.
fn compress_body(text: &str, max_chars: usize, max_lines: usize) -> String {
    let text = text.trim();
    if text.is_empty() {
        return String::new();
    }

    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return String::new();
    }

    // Step 1: deduplicate lines (case-insensitive) + truncate long lines.
    let mut seen = std::collections::HashSet::new();
    let mut deduped: Vec<String> = Vec::with_capacity(lines.len());
    for line in lines {
        let key = line.trim().to_ascii_lowercase();
        if seen.insert(key) {
            deduped.push(truncate_line(line, SUMMARY_MAX_LINE_CHARS));
        }
    }
    drop(seen);

    // Step 2: check if already within budget.
    let joined = deduped.join("\n");
    if deduped.len() <= max_lines && joined.len() <= max_chars {
        return joined;
    }

    // Step 3: priority-based line dropping.
    // Headers (#) always kept. Bullets (- * •) survive longer than plain lines.
    let mut headers: Vec<String> = Vec::new();
    let mut bullets: Vec<String> = Vec::new();
    let mut other: Vec<String> = Vec::new();

    for line in deduped {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            headers.push(line);
        } else if trimmed.starts_with(['-', '*', '\u{2022}']) {
            bullets.push(line);
        } else {
            other.push(line);
        }
    }

    // Candidates ordered for dropping: other at the tail (dropped first), bullets kept longer.
    let mut candidates: Vec<String> = Vec::new();
    candidates.extend(bullets);
    candidates.extend(other);

    let header_count = headers.len();

    // Check if keeping all candidates fits.
    if header_count + candidates.len() <= max_lines
        && total_join_len(&headers, &candidates) <= max_chars
    {
        let mut result = headers;
        result.extend(candidates);
        return result.join("\n");
    }

    // Drop lines from the end of candidates, accounting for omission notice.
    let omission_notice = |n: usize| format!("[... {n} lines omitted for brevity]");

    for drop_count in 1..=candidates.len() {
        let keep = &candidates[..candidates.len() - drop_count];
        let line_count = header_count + keep.len() + 1;
        if line_count > max_lines {
            continue;
        }

        let notice = omission_notice(drop_count);
        let len = total_join_len(&headers, keep) + notice.len() + 1;
        if len <= max_chars {
            let mut result = headers;
            result.extend(keep.iter().cloned());
            result.push(notice);
            return result.join("\n");
        }
    }

    // Edge case: even dropping all candidates, headers alone are too long.
    // Reserve space for the worst-case notice (all candidates AND every header
    // dropped) so the final notice can never push us past `max_chars`.
    let all_dropped = candidates.len();
    let max_possible_drops = all_dropped + headers.len();
    let notice_len = omission_notice(max_possible_drops).len() + 1;
    let mut budget = max_chars.saturating_sub(notice_len);

    let mut result: Vec<String> = Vec::new();
    let mut header_drops = 0usize;
    for line in &headers {
        let needed = line.len() + usize::from(!result.is_empty());
        if needed > budget || result.len() + 1 >= max_lines {
            header_drops += 1;
            continue;
        }
        budget -= needed;
        result.push(line.clone());
    }

    result.push(omission_notice(all_dropped + header_drops));
    result.join("\n")
}

// ── Private helpers ──────────────────────────────────────────────────

/// Format a single timeline entry: `  - role: content_preview`.
fn timeline_entry(message: &Value) -> String {
    let role = message["role"].as_str().unwrap_or("unknown");
    let content = extract_content_preview(message);
    format!("  - {role}: {content}")
}

/// Build timeline entries for a message slice, with head+tail truncation.
fn build_timeline_entries(messages: &[Value]) -> Vec<String> {
    const HEAD: usize = 3;
    const TAIL: usize = 5;

    if messages.len() <= HEAD + TAIL {
        return messages.iter().map(timeline_entry).collect();
    }

    let mut entries: Vec<String> = messages[..HEAD].iter().map(timeline_entry).collect();
    let omitted = messages.len() - HEAD - TAIL;
    entries.push(format!("  - ... ({omitted} messages omitted) ..."));
    entries.extend(messages[messages.len() - TAIL..].iter().map(timeline_entry));
    entries
}

/// Extract a text preview from a JSON message value.
fn extract_content_preview(message: &Value) -> String {
    let mut parts = Vec::new();

    // Text content
    if let Some(text) = message.get("content").and_then(Value::as_str) {
        parts.push(truncate_to(text, PREVIEW_CHARS));
    } else if let Some(blocks) = message.get("content").and_then(Value::as_array) {
        for block in blocks {
            if block["type"] == "text"
                && let Some(text) = block.get("text").and_then(Value::as_str)
            {
                parts.push(truncate_to(text, PREVIEW_CHARS));
            }
        }
    }

    // Tool calls
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let name = call
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let args = call
                .get("function")
                .and_then(|f| f.get("arguments"))
                .and_then(Value::as_str)
                .unwrap_or("{}");
            parts.push(truncate_to(
                &format!("tool_use {name}({args})"),
                PREVIEW_CHARS,
            ));
        }
    }

    // Tool result
    if message["role"] == "tool" || message["role"] == "tool_result" {
        let tool_name = message
            .get("tool_name")
            .or_else(|| message.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let is_error = message.get("error").and_then(Value::as_str).is_some()
            || message
                .get("success")
                .and_then(Value::as_bool)
                .is_some_and(|s| !s);
        let result_text = message.get("content").and_then(Value::as_str).unwrap_or("");
        let prefix = if is_error {
            "error "
        } else {
            ""
        };
        parts.push(truncate_to(
            &format!("tool_result {tool_name}: {prefix}{result_text}"),
            PREVIEW_CHARS,
        ));
    }

    if parts.is_empty() {
        "(empty)".to_string()
    } else {
        parts.join(" | ")
    }
}

/// Collect unique, sorted tool names from message history.
///
/// Pulls names from assistant `tool_calls[].function.name` on any message and
/// from `tool_name`/`name` only on tool-result messages — the OpenAI format
/// uses `name` on user messages too, which would otherwise leak user handles
/// into the "Tools mentioned" field.
fn collect_unique_tool_names(messages: &[Value]) -> Vec<&str> {
    let mut names: Vec<&str> = messages
        .iter()
        .flat_map(|m| {
            let mut out = Vec::new();
            if let Some(calls) = m.get("tool_calls").and_then(Value::as_array) {
                for call in calls {
                    if let Some(name) = call
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(Value::as_str)
                    {
                        out.push(name);
                    }
                }
            }
            if is_tool_role(m) {
                if let Some(name) = m.get("tool_name").and_then(Value::as_str) {
                    out.push(name);
                }
                if let Some(name) = m.get("name").and_then(Value::as_str) {
                    out.push(name);
                }
            }
            out
        })
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// True if a message's role is `tool` or `tool_result`.
fn is_tool_role(message: &Value) -> bool {
    matches!(
        message.get("role").and_then(Value::as_str),
        Some("tool" | "tool_result")
    )
}

/// Collect recent text previews for messages matching a given role.
fn collect_recent_role_summaries(messages: &[Value], role: &str, limit: usize) -> Vec<String> {
    messages
        .iter()
        .filter(|m| m["role"] == role)
        .rev()
        .filter_map(first_text_block)
        .take(limit)
        .map(|text| truncate_to(text, PREVIEW_CHARS))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

/// Keyword-based inference of pending work items.
fn infer_pending_work(messages: &[Value]) -> Vec<String> {
    const KEYWORDS: &[&str] = &["todo", "next", "pending", "follow up", "remaining"];

    messages
        .iter()
        .rev()
        .filter_map(first_text_block)
        .filter(|text| {
            let lowered = text.to_ascii_lowercase();
            KEYWORDS.iter().any(|kw| lowered.contains(kw))
        })
        .take(3)
        .map(|text| truncate_to(text, PREVIEW_CHARS))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

/// Extract file paths with interesting extensions from message content.
fn collect_key_files(messages: &[Value]) -> Vec<String> {
    let mut files: Vec<String> = messages
        .iter()
        .flat_map(|m| {
            let mut texts: Vec<&str> = Vec::new();
            // Reuse first_text_block for main content extraction.
            if let Some(text) = first_text_block(m) {
                texts.push(text);
            }
            // Additionally check tool_calls arguments for file paths.
            if let Some(calls) = m.get("tool_calls").and_then(Value::as_array) {
                for call in calls {
                    if let Some(args) = call
                        .get("function")
                        .and_then(|f| f.get("arguments"))
                        .and_then(Value::as_str)
                    {
                        texts.push(args);
                    }
                }
            }
            texts
                .into_iter()
                .flat_map(extract_file_candidates)
                .collect::<Vec<_>>()
        })
        .collect();
    files.sort();
    files.dedup();
    files.into_iter().take(8).collect()
}

/// Infer the most recent non-empty assistant text as "current work".
fn infer_current_work(messages: &[Value]) -> Option<String> {
    messages
        .iter()
        .rev()
        .filter(|m| m["role"] == "assistant")
        .filter_map(first_text_block)
        .find(|text| !text.trim().is_empty())
        .map(|text| truncate_to(text, CURRENT_WORK_CHARS))
}

/// Extract the first non-empty text from a JSON message value.
fn first_text_block(message: &Value) -> Option<&str> {
    if let Some(text) = message.get("content").and_then(Value::as_str) {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }
    if let Some(blocks) = message.get("content").and_then(Value::as_array) {
        for block in blocks {
            if block["type"] == "text"
                && let Some(text) = block.get("text").and_then(Value::as_str)
            {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed);
                }
            }
        }
    }
    None
}

/// Parsed sections of a formatted summary.
struct SummarySections {
    highlights: Vec<String>,
    timeline: Vec<String>,
}

/// Parse a summary into highlights and timeline sections in a single pass.
fn parse_summary_sections(summary: &str) -> SummarySections {
    let mut highlights = Vec::new();
    let mut timeline = Vec::new();
    let mut in_timeline = false;

    for line in format_compact_summary(summary).lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() || trimmed == "Summary:" || trimmed == "Conversation summary:" {
            if in_timeline {
                break;
            }
            continue;
        }
        if trimmed == "- Key timeline:" {
            in_timeline = true;
            continue;
        }
        if in_timeline {
            timeline.push(trimmed.to_string());
        } else {
            highlights.push(trimmed.to_string());
        }
    }

    SummarySections {
        highlights,
        timeline,
    }
}

/// Extract file path candidates from content using whitespace splitting.
fn extract_file_candidates(content: &str) -> Vec<String> {
    content
        .split_whitespace()
        .filter_map(|token| {
            let candidate = token.trim_matches(|c: char| {
                matches!(
                    c,
                    ',' | '.' | ':' | ';' | ')' | '(' | '"' | '\'' | '`' | '{' | '}'
                )
            });
            if candidate.contains('/') && has_interesting_extension(candidate) {
                Some(candidate.to_string())
            } else {
                None
            }
        })
        .collect()
}

/// Check if a path has an interesting source code extension.
fn has_interesting_extension(candidate: &str) -> bool {
    std::path::Path::new(candidate)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            [
                "rs", "ts", "tsx", "js", "json", "md", "py", "toml", "yaml", "yml", "go", "css",
                "html", "sql", "sh", "cfg", "ini", "jsx", "jsonc",
            ]
            .iter()
            .any(|expected| ext.eq_ignore_ascii_case(expected))
        })
}

/// Truncate content to max_len bytes, appending ellipsis if truncated.
fn truncate_to(content: &str, max_len: usize) -> String {
    if content.len() <= max_len {
        return content.to_string();
    }
    let mut truncated = content[..content.floor_char_boundary(max_len)].to_string();
    truncated.push('…');
    truncated
}

/// Truncate a single line to max bytes (no ellipsis — used by compress_summary).
fn truncate_line(line: &str, max_bytes: usize) -> String {
    if line.len() <= max_bytes {
        line.to_string()
    } else {
        line[..line.floor_char_boundary(max_bytes)].to_string()
    }
}

/// Calculate total byte length of headers + candidates when joined with newlines.
fn total_join_len(headers: &[String], candidates: &[String]) -> usize {
    let total: usize = headers
        .iter()
        .chain(candidates.iter())
        .map(|s| s.len())
        .sum();
    let count = headers.len() + candidates.len();
    if count == 0 {
        0
    } else {
        total + count - 1
    }
}

/// Collapse consecutive blank lines into a single newline.
fn collapse_blank_lines(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut last_blank = false;
    for line in content.lines() {
        let is_blank = line.trim().is_empty();
        if is_blank && last_blank {
            continue;
        }
        result.push_str(line);
        result.push('\n');
        last_blank = is_blank;
    }
    result
}

/// Extract the content between `<tag>...</tag>` markers.
fn extract_tag_block(content: &str, tag: &str) -> Option<String> {
    let start_marker = format!("<{tag}>");
    let end_marker = format!("</{tag}>");
    let start_idx = content.find(&start_marker)? + start_marker.len();
    let end_idx = content[start_idx..].find(&end_marker)? + start_idx;
    Some(content[start_idx..end_idx].to_string())
}

/// Remove a `<tag>...</tag>` block from content.
fn strip_tag_block(content: &str, tag: &str) -> String {
    let start_marker = format!("<{tag}>");
    let end_marker = format!("</{tag}>");
    if let Some(start_idx) = content.find(&start_marker) {
        let after_start = start_idx + start_marker.len();
        if let Some(end_rel) = content[after_start..].find(&end_marker) {
            let end_idx = after_start + end_rel + end_marker.len();
            let mut stripped = String::with_capacity(content.len());
            stripped.push_str(&content[..start_idx]);
            stripped.push_str(&content[end_idx..]);
            return stripped;
        }
    }
    content.to_string()
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, serde_json::json};

    fn make_user(text: &str) -> Value {
        json!({
            "role": "user",
            "content": text
        })
    }

    fn make_assistant(text: &str) -> Value {
        json!({
            "role": "assistant",
            "content": text
        })
    }

    fn make_assistant_with_tools(text: &str, tool_names: &[&str]) -> Value {
        let calls: Vec<Value> = tool_names
            .iter()
            .map(|name| {
                json!({
                    "id": format!("call_{name}"),
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": "{}"
                    }
                })
            })
            .collect();
        json!({
            "role": "assistant",
            "content": text,
            "tool_calls": calls
        })
    }

    fn make_tool_result(tool_name: &str, content: &str, success: bool) -> Value {
        json!({
            "role": "tool_result",
            "tool_name": tool_name,
            "content": content,
            "success": success
        })
    }

    // ── summarize_messages ──────────────────────────────────────────

    #[test]
    fn summarize_messages_basic() {
        let messages = vec![
            make_user("hello"),
            make_assistant("hi there"),
            make_user("how are you"),
            make_assistant("doing well"),
        ];
        let summary = summarize_messages(&messages);
        assert!(summary.contains("<summary>"));
        assert!(summary.contains("</summary>"));
        assert!(summary.contains("user=2"));
        assert!(summary.contains("assistant=2"));
        assert!(summary.contains("tool=0"));
        assert!(summary.contains("Scope: 4 earlier messages"));
    }

    #[test]
    fn summarize_messages_with_tools() {
        let messages = vec![
            make_user("run a search"),
            make_assistant_with_tools("searching", &["search", "read_file"]),
            make_tool_result("search", "found 5 files", true),
            make_tool_result("read_file", "file contents", true),
        ];
        let summary = summarize_messages(&messages);
        assert!(summary.contains("Tools mentioned: read_file, search"));
        assert!(summary.contains("tool=2"));
    }

    #[test]
    fn summarize_messages_key_files() {
        let messages = vec![make_user(
            "Update crates/chat/src/compaction.rs and src/main.rs next.",
        )];
        let summary = summarize_messages(&messages);
        assert!(summary.contains("crates/chat/src/compaction.rs"));
        assert!(summary.contains("src/main.rs"));
    }

    #[test]
    fn summarize_messages_pending_work() {
        let messages = vec![
            make_user("do something"),
            make_assistant("Next: update the tests and follow up on remaining items."),
        ];
        let summary = summarize_messages(&messages);
        assert!(summary.contains("Pending work:"));
        assert!(summary.contains("Next: update the tests"));
    }

    #[test]
    fn summarize_messages_empty() {
        let summary = summarize_messages(&[]);
        assert!(summary.contains("user=0"));
        assert!(summary.contains("assistant=0"));
    }

    // ── merge_compact_summaries ──────────────────────────────────────

    #[test]
    fn merge_compact_summaries_first_compaction() {
        let new = "<summary>Conversation summary:\n- Scope: 4 messages.\n- Key timeline:\n  - user: hello\n</summary>";
        let result = merge_compact_summaries(None, new);
        assert_eq!(result, new);
    }

    #[test]
    fn merge_compact_summaries_recompaction() {
        let existing = "<summary>Conversation summary:\n- Scope: 2 messages.\n- Key files: src/main.rs.\n- Key timeline:\n  - user: old\n</summary>";
        let new = "<summary>Conversation summary:\n- Scope: 3 messages.\n- Key files: lib.rs.\n- Key timeline:\n  - user: new\n</summary>";

        let merged = merge_compact_summaries(Some(existing), new);
        assert!(merged.contains("Previously compacted context:"));
        assert!(merged.contains("Newly compacted context:"));
        assert!(merged.contains("Key files: src/main.rs"));
        assert!(merged.contains("Key files: lib.rs"));
        // Old timeline should be dropped, new timeline kept
        assert!(merged.contains("- user: new"));
    }

    // ── extract_existing_compacted_summary ───────────────────────────

    #[test]
    fn extract_existing_compacted_summary_detected() {
        let history = vec![json!({
            "role": "user",
            "content": "[Conversation Summary]\n\nSome summary text here"
        })];
        let result = extract_existing_compacted_summary(&history);
        assert_eq!(result, Some("Some summary text here".to_string()));
    }

    #[test]
    fn extract_existing_compacted_summary_not_found() {
        let history = vec![make_user("normal message")];
        let result = extract_existing_compacted_summary(&history);
        assert_eq!(result, None);
    }

    #[test]
    fn extract_existing_compacted_summary_empty_history() {
        let result: Option<String> = extract_existing_compacted_summary(&[]);
        assert_eq!(result, None);
    }

    // ── get_compact_continuation_message ─────────────────────────────

    #[test]
    fn get_compact_continuation_message_full() {
        let summary = "<summary>Test summary</summary>";
        let msg = get_compact_continuation_message(summary, true);
        assert!(msg.contains("continued from a previous conversation"));
        assert!(msg.contains("Summary:"));
        assert!(msg.contains("Test summary"));
        assert!(msg.contains("Recent messages are preserved verbatim"));
        assert!(msg.contains("Continue the conversation from where it left off"));
    }

    #[test]
    fn get_compact_continuation_message_no_recent() {
        let summary = "<summary>Test</summary>";
        let msg = get_compact_continuation_message(summary, false);
        assert!(!msg.contains("Recent messages are preserved"));
        assert!(msg.contains("Continue the conversation"));
    }

    // ── format_compact_summary ───────────────────────────────────────

    #[test]
    fn format_compact_summary_extracts_tag() {
        let raw = "<analysis>scratch</analysis>\n<summary>Kept work</summary>";
        let formatted = format_compact_summary(raw);
        assert_eq!(formatted, "Summary:\nKept work");
    }

    #[test]
    fn format_compact_summary_no_tags() {
        let raw = "Just plain text summary";
        let formatted = format_compact_summary(raw);
        assert_eq!(formatted, "Just plain text summary");
    }

    // ── collect_key_files ────────────────────────────────────────────

    #[test]
    fn collect_key_files_various_extensions() {
        let messages = vec![make_user(
            "Update src/main.rs and crates/lib.ts plus config/config.json and docs/README.md",
        )];
        let files = collect_key_files(&messages);
        assert!(files.contains(&"src/main.rs".to_string()));
        assert!(files.contains(&"config/config.json".to_string()));
        assert!(files.contains(&"docs/README.md".to_string()));
    }

    #[test]
    fn collect_key_files_multiple_tool_calls() {
        // Assistant with multiple simultaneous tool calls — all args should be scanned.
        let messages = vec![json!({
            "role": "assistant",
            "content": "reading files",
            "tool_calls": [
                {
                    "id": "call_1",
                    "type": "function",
                    "function": {
                        "name": "read_file",
                        "arguments": "{\"path\": \"src/main.rs\"}"
                    }
                },
                {
                    "id": "call_2",
                    "type": "function",
                    "function": {
                        "name": "read_file",
                        "arguments": "{\"path\": \"lib/core.ts\"}"
                    }
                }
            ]
        })];
        let files = collect_key_files(&messages);
        assert!(
            files.contains(&"src/main.rs".to_string()),
            "first tool call file should be found"
        );
        assert!(
            files.contains(&"lib/core.ts".to_string()),
            "second tool call file should be found"
        );
    }

    // ── helper unit tests ────────────────────────────────────────────

    #[test]
    fn truncate_to_short() {
        assert_eq!(truncate_to("hello", 10), "hello");
    }

    #[test]
    fn truncate_to_long() {
        let long = "x".repeat(200);
        let truncated = truncate_to(&long, 160);
        assert!(truncated.ends_with('…'));
        assert!(
            truncated.len() <= 163,
            "truncated len should be <= 163, got {}",
            truncated.len()
        );
    }

    #[test]
    fn extract_tag_block_found() {
        let text = "before <foo>content</foo> after";
        assert_eq!(extract_tag_block(text, "foo"), Some("content".to_string()));
    }

    #[test]
    fn extract_tag_block_missing() {
        assert_eq!(extract_tag_block("no tags here", "foo"), None);
    }

    #[test]
    fn strip_tag_block_removes() {
        let text = "before <analysis>junk</analysis> after";
        assert_eq!(strip_tag_block(text, "analysis"), "before  after");
    }

    #[test]
    fn strip_tag_block_anchors_after_start() {
        // Malformed input with end marker before start marker.
        let text = "before </analysis> middle <analysis>junk</analysis> after";
        let result = strip_tag_block(text, "analysis");
        assert!(
            result.contains("before </analysis> middle"),
            "content before start marker should be preserved"
        );
        assert!(
            !result.contains("junk"),
            "content inside tag block should be stripped"
        );
        assert!(
            result.contains("after"),
            "content after end marker should be preserved"
        );
    }

    #[test]
    fn collapse_blank_lines_deduplicates() {
        let text = "a\n\n\nb\n\nc";
        assert_eq!(collapse_blank_lines(text), "a\n\nb\n\nc\n");
    }

    // ── compute_compaction_summary ──────────────────────────────────

    #[test]
    fn compute_compaction_summary_basic() {
        let history = vec![
            make_user("hello"),
            make_assistant("hi there"),
            make_user("how are you"),
            make_assistant("doing well"),
        ];
        let summary = compute_compaction_summary(&history).expect("basic history compacts");
        assert!(summary.contains("<summary>"));
        assert!(summary.contains("Scope: 4 earlier messages"));
    }

    #[test]
    fn compute_compaction_summary_empty() {
        // Empty history returns None so callers can surface a meaningful error.
        assert!(compute_compaction_summary(&[]).is_none());
    }

    // ── summarize_messages timeline truncation ──────────────────────

    #[test]
    fn summarize_messages_timeline_truncated() {
        // 12 messages: first 3 + last 5 shown, 4 omitted
        let mut messages = Vec::new();
        for i in 0..12 {
            messages.push(make_user(&format!("msg {i}")));
        }
        let summary = summarize_messages(&messages);
        assert!(summary.contains("msg 0"));
        assert!(summary.contains("msg 2"));
        assert!(summary.contains("4 messages omitted"));
        assert!(summary.contains("msg 7"));
        assert!(summary.contains("msg 11"));
        assert!(summary.contains("(4 messages omitted)"));
    }

    // ── extract_existing_compacted_summary preamble format ──────────

    #[test]
    fn extract_existing_compacted_summary_preamble_format() {
        let content = "This session is being continued from a previous conversation that ran out of context. \
             The summary below covers the earlier portion of the conversation.\n\nSome summary content";
        let history = vec![json!({
            "role": "user",
            "content": content,
        })];
        let result =
            extract_existing_compacted_summary(&history).expect("preamble-only format extracts");
        assert_eq!(result, "Some summary content");
        assert!(!result.contains("continued from a previous"));
    }

    #[test]
    fn extract_existing_compacted_summary_strips_full_wrapping() {
        // Build the actual persisted shape via the same helper production uses,
        // so the test exercises the real round-trip instead of a hand-rolled
        // approximation that could drift from the builder.
        let raw_summary = "<summary>Actual summary here</summary>";
        let continuation = get_compact_continuation_message(raw_summary, false);
        let persisted = format!("[Conversation Summary]\n\n{continuation}");
        let history = vec![json!({ "role": "user", "content": persisted })];

        let result =
            extract_existing_compacted_summary(&history).expect("production wrapping extracts");
        assert!(result.contains("Actual summary here"));
        assert!(!result.contains("continued from a previous"));
        assert!(!result.contains("Continue the conversation from where it left off"));
    }

    #[test]
    fn extract_existing_compacted_summary_recent_messages_preserved() {
        // When recent_messages_preserved=true, the note sits between the
        // summary and the resume directive separated by newlines — the suffix
        // stripping must still peel off both, not just the directive.
        let raw_summary = "<summary>Inner body</summary>";
        let continuation = get_compact_continuation_message(raw_summary, true);
        let persisted = format!("[Conversation Summary]\n\n{continuation}");
        let history = vec![json!({ "role": "user", "content": persisted })];

        let result =
            extract_existing_compacted_summary(&history).expect("recent-preserved extracts");
        assert!(result.contains("Inner body"));
        assert!(
            !result.contains("Recent messages are preserved verbatim"),
            "recent-messages note must be stripped, got: {result}"
        );
        assert!(!result.contains("Continue the conversation from where it left off"));
    }

    // ── collect_key_files expanded extensions ───────────────────────

    #[test]
    fn collect_key_files_python_and_toml() {
        let messages = vec![make_user(
            "Update src/app.py and config/settings.toml plus deploy/deploy.yaml",
        )];
        let files = collect_key_files(&messages);
        assert!(files.contains(&"src/app.py".to_string()));
        assert!(files.contains(&"config/settings.toml".to_string()));
        assert!(files.contains(&"deploy/deploy.yaml".to_string()));
    }

    // ── compress_summary (budget enforcement) ───────────────────────

    #[test]
    fn compress_summary_under_budget_returns_unchanged() {
        let input = "# Summary\n\n- Key point one\n- Key point two\nDone.";
        let result = compress_summary(input);
        assert_eq!(result, input);
    }

    #[test]
    fn compress_summary_over_char_limit() {
        let mut lines = vec!["# Summary".to_string()];
        for i in 0..30 {
            lines.push(format!(
                "- This is line {i} with some padding text to make it longer than usual"
            ));
        }
        let input = lines.join("\n");
        assert!(input.len() > 1_200, "input should exceed 1200 chars");

        let result = compress_summary(&input);
        assert!(
            result.len() <= 1_200,
            "result must be <= 1200 chars, got {}",
            result.len()
        );
        assert!(
            result.contains("lines omitted"),
            "should have omission notice"
        );
    }

    #[test]
    fn compress_summary_over_line_count() {
        let mut lines = vec!["# Summary".to_string()];
        for i in 0..40 {
            lines.push(format!("Line {i}"));
        }
        let input = lines.join("\n");

        let result = compress_summary(&input);
        let result_lines: Vec<&str> = result.lines().collect();
        assert!(
            result_lines.len() <= 25,
            "result should be <= 25 lines (24 + notice), got {}",
            result_lines.len()
        );
        assert!(result.contains("lines omitted"));
    }

    #[test]
    fn compress_summary_long_line_truncation() {
        let long_line: String = "x".repeat(200);
        let input = format!("Header\n{long_line}");
        let result = compress_summary(&input);

        let result_lines: Vec<&str> = result.lines().collect();
        assert_eq!(result_lines.len(), 2);
        assert!(
            result_lines[1].len() <= 160,
            "long line should be <= 160 chars, got {}",
            result_lines[1].len()
        );
    }

    #[test]
    fn compress_summary_deduplication() {
        let input = "Alpha\nalpha\nBeta\nBETA\nGamma";
        let result = compress_summary(input);
        let result_lines: Vec<&str> = result.lines().collect();
        assert_eq!(result_lines, vec!["Alpha", "Beta", "Gamma"]);
    }

    #[test]
    fn compress_summary_header_preservation() {
        let mut lines = vec!["# Section One".to_string()];
        for i in 0..30 {
            lines.push(format!(
                "Body line {i} with enough text to fill up space here"
            ));
        }
        lines.push("## Section Two".to_string());
        for i in 0..10 {
            lines.push(format!("- Bullet {i} important"));
        }
        let input = lines.join("\n");

        let result = compress_summary(&input);
        assert!(
            result.contains("# Section One"),
            "headers should be preserved"
        );
        assert!(
            result.contains("## Section Two"),
            "second header should be preserved"
        );
        assert!(result.contains("lines omitted"));
    }

    #[test]
    fn compress_summary_priority_drops_other_before_bullets() {
        // Build input that exceeds line budget with both bullets and plain lines.
        let mut lines = vec![String::new()];
        for i in 0..20 {
            lines.push(format!("Plain body line number {i} with filler text"));
        }
        for i in 0..10 {
            lines.push(format!("- Important bullet point {i}"));
        }
        let input = lines.join("\n");

        let result = compress_summary(&input);

        // Count retained bullets vs plain lines.
        let retained_bullets: Vec<_> = result
            .lines()
            .filter(|l| l.trim_start().starts_with('-'))
            .collect();
        let retained_plain: Vec<_> = result
            .lines()
            .filter(|l| {
                let t = l.trim_start();
                !t.is_empty() && !t.starts_with('#') && !t.starts_with('-') && !t.starts_with("[")
            })
            .collect();

        // All 10 bullets must survive since they have higher priority.
        assert_eq!(
            retained_bullets.len(),
            10,
            "all bullets should be retained when there are fewer bullets than budget space"
        );
        // Some plain lines must be dropped (only 23 slots for 30 non-header lines).
        assert!(
            retained_plain.len() < 20,
            "some plain lines should be dropped to fit budget"
        );
    }

    #[test]
    fn compress_summary_empty_input() {
        assert_eq!(compress_summary(""), "");
        assert_eq!(compress_summary("   "), "");
    }

    #[test]
    fn compress_summary_single_very_long_line() {
        let long_line = "a".repeat(2_000);
        let result = compress_summary(&long_line);

        assert!(
            result.len() <= 160,
            "single long line should be truncated, got {} chars",
            result.len()
        );
    }

    #[test]
    fn compress_summary_protects_summary_tags_under_budget_pressure() {
        // Build a summary that exceeds the 1200-char budget due to timeline entries.
        // Timeline alone can reach 8 entries × ~160 chars ≈ 1280 chars.
        let mut lines = vec!["<summary>".to_string(), "Conversation summary:".to_string()];
        lines.push("- Scope: 50 messages.".to_string());
        lines.push("- Tools mentioned: exec, read_file, write_file.".to_string());
        lines.push("- Key files referenced: src/main.rs, lib/core.ts.".to_string());
        lines.push("- Key timeline:".to_string());
        for i in 0..15 {
            lines.push(format!(
                "  - user: This is a long timeline entry number {i} with enough padding to push the total over budget"
            ));
        }
        lines.push("</summary>".to_string());
        let input = lines.join("\n");
        assert!(input.len() > 1_200, "test input must exceed budget");

        let result = compress_summary(&input);
        assert!(
            result.starts_with("<summary>\n"),
            "opening <summary> must remain at the start, got: {result}"
        );
        assert!(
            result.ends_with("\n</summary>"),
            "closing </summary> must remain at the end, got: {result}"
        );
        assert!(
            result.len() <= 1_200,
            "result must fit within budget, got {} chars",
            result.len()
        );

        // The tag wrapping must still be parseable structurally — i.e. the
        // body text should live *between* the tags, not after the closing tag.
        let inner = extract_tag_block(&result, "summary").expect("summary tags wrap content");
        assert!(
            inner.contains("- Scope: 50 messages"),
            "body content must be inside <summary>...</summary>, got: {inner}"
        );
    }
}
