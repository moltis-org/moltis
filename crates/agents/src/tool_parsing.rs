//! Text-based tool call parsing for models without native tool calling.
//!
//! Extracts structured `ToolCall` values from free-form LLM text output.
//! Supports multiple formats:
//!
//! 1. **Fenced blocks**: `` ```tool_call\n{...}\n``` ``
//! 2. **XML function calls**: `<function=name><parameter=key>value</parameter></function>`
//! 3. **Bare JSON**: `{"tool": "name", "arguments": {...}}`

use std::fmt::Write;

use crate::{json_repair, model::ToolCall};

/// Maximum length for synthetic tool call IDs (OpenAI compatibility).
const SYNTHETIC_TOOL_CALL_ID_MAX_LEN: usize = 40;

/// Generate a synthetic tool-call ID that is OpenAI-compatible (max 40 chars).
pub(crate) fn new_synthetic_tool_call_id(prefix: &str) -> String {
    let mut id = String::new();
    let _ = write!(&mut id, "{prefix}_{}", uuid::Uuid::new_v4().simple());
    if id.len() <= SYNTHETIC_TOOL_CALL_ID_MAX_LEN {
        return id;
    }
    id.truncate(SYNTHETIC_TOOL_CALL_ID_MAX_LEN);
    id
}

/// Result of parsing a single tool call block from text.
struct ParsedBlock {
    tool_call: ToolCall,
    /// Byte range of the block in the source text.
    start: usize,
    end: usize,
}

/// Parse ALL tool call blocks from model text output.
///
/// Returns a list of parsed `ToolCall` values and any remaining text that was
/// not part of a tool call block (interstitial commentary, reasoning, etc.).
///
/// Tries parsers in priority order: fenced, XML function, bare JSON.
pub fn parse_tool_calls_from_text(text: &str) -> (Vec<ToolCall>, Option<String>) {
    let mut blocks: Vec<ParsedBlock> = Vec::new();

    // 1. Find all fenced ```tool_call blocks.
    collect_fenced_blocks(text, &mut blocks);

    // 2. Find all XML <function=...> blocks.
    collect_function_blocks(text, &mut blocks);

    // 3. Find bare JSON {"tool": ...} blocks.
    collect_bare_json_blocks(text, &mut blocks);

    if blocks.is_empty() {
        return (vec![], Some(text.to_string()));
    }

    // Sort by start position and de-overlap (keep first occurrence).
    blocks.sort_by_key(|b| b.start);
    let mut merged: Vec<ParsedBlock> = Vec::with_capacity(blocks.len());
    for block in blocks {
        if let Some(last) = merged.last() {
            if block.start < last.end {
                continue; // overlapping — skip
            }
        }
        merged.push(block);
    }

    // Collect remaining text fragments.
    let mut remaining_parts: Vec<&str> = Vec::new();
    let mut cursor = 0;
    for block in &merged {
        let before = trim_tool_call_wrappers(text[cursor..block.start].trim());
        if !before.is_empty() {
            remaining_parts.push(before);
        }
        cursor = block.end;
    }
    let after = trim_tool_call_wrappers(text[cursor..].trim());
    if !after.is_empty() {
        remaining_parts.push(after);
    }

    let tool_calls: Vec<ToolCall> = merged.into_iter().map(|b| b.tool_call).collect();
    let remaining = if remaining_parts.is_empty() {
        None
    } else {
        Some(remaining_parts.join("\n"))
    };

    (tool_calls, remaining)
}

/// Backward-compatible single tool call parser.
///
/// Wraps [`parse_tool_calls_from_text`] and returns only the first match.
pub fn parse_tool_call_from_text(text: &str) -> Option<(ToolCall, Option<String>)> {
    let (calls, remaining) = parse_tool_calls_from_text(text);
    let first = calls.into_iter().next()?;
    Some((first, remaining))
}

/// Heuristic: does the text look like it was *trying* to make a tool call but
/// failed to produce valid output?
pub fn looks_like_failed_tool_call(text: &Option<String>) -> bool {
    let Some(t) = text.as_deref() else {
        return false;
    };
    let lower = t.to_ascii_lowercase();
    (lower.contains("\"tool\"") || lower.contains("tool_call") || lower.contains("<function="))
        && parse_tool_call_from_text(t).is_none()
}

// ── Fenced block parser ─────────────────────────────────────────────────────

fn collect_fenced_blocks(text: &str, blocks: &mut Vec<ParsedBlock>) {
    let start_marker = "```tool_call";
    let mut search_from = 0;

    while let Some(start) = text[search_from..].find(start_marker) {
        let abs_start = search_from + start;
        let after_marker = abs_start + start_marker.len();
        let rest = &text[after_marker..];
        let Some(end_rel) = rest.find("```") else {
            break;
        };
        let json_str = rest[..end_rel].trim();
        let abs_end = after_marker + end_rel + 3; // skip closing ```

        if let Some(parsed) = try_parse_tool_json(json_str) {
            blocks.push(ParsedBlock {
                tool_call: parsed,
                start: abs_start,
                end: abs_end,
            });
        }
        search_from = abs_end;
    }
}

// ── XML function parser ─────────────────────────────────────────────────────

fn collect_function_blocks(text: &str, blocks: &mut Vec<ParsedBlock>) {
    let start_marker = "<function=";
    let mut search_from = 0;

    while let Some(start_rel) = text[search_from..].find(start_marker) {
        let abs_start = search_from + start_rel;
        let after_marker = abs_start + start_marker.len();
        let rest = &text[after_marker..];

        let Some(open_end_rel) = rest.find('>') else {
            break;
        };
        let tool_name = rest[..open_end_rel].trim();
        if tool_name.is_empty()
            || !tool_name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            search_from = after_marker;
            continue;
        }

        let body_start = after_marker + open_end_rel + 1;
        let Some(after_open) = text.get(body_start..) else {
            break;
        };
        let Some(body_end_rel) = after_open.find("</function>") else {
            search_from = body_start;
            continue;
        };
        let body = &after_open[..body_end_rel];
        let abs_end = body_start + body_end_rel + "</function>".len();

        // Extend past any trailing </tool_call>.
        let mut final_end = abs_end;
        let trailing = text.get(abs_end..).unwrap_or("").trim_start();
        if let Some(rest) = trailing.strip_prefix("</tool_call>") {
            final_end = text.len() - rest.len();
        }

        let mut args = serde_json::Map::new();
        let mut found = false;
        let mut cursor = 0usize;
        while let Some(param_rel) = body[cursor..].find("<parameter=") {
            let param_start = cursor + param_rel;
            let after_param_marker = param_start + "<parameter=".len();
            let Some(param_rest) = body.get(after_param_marker..) else {
                break;
            };
            let Some(param_name_end) = param_rest.find('>') else {
                break;
            };
            let param_name = param_rest[..param_name_end].trim();
            if param_name.is_empty()
                || !param_name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                cursor = after_param_marker + param_name_end + 1;
                continue;
            }

            let value_start = after_param_marker + param_name_end + 1;
            let Some(value_rest) = body.get(value_start..) else {
                break;
            };
            let Some(value_end_rel) = value_rest.find("</parameter>") else {
                break;
            };
            let value_raw = body
                .get(value_start..value_start + value_end_rel)
                .unwrap_or("")
                .trim();

            args.insert(param_name.to_string(), parse_param_value(value_raw));
            found = true;
            cursor = value_start + value_end_rel + "</parameter>".len();
        }

        if found {
            blocks.push(ParsedBlock {
                tool_call: ToolCall {
                    id: new_synthetic_tool_call_id("text"),
                    name: tool_name.to_string(),
                    arguments: serde_json::Value::Object(args),
                },
                start: abs_start,
                end: final_end,
            });
        }
        search_from = final_end;
    }
}

// ── Bare JSON parser ────────────────────────────────────────────────────────

fn collect_bare_json_blocks(text: &str, blocks: &mut Vec<ParsedBlock>) {
    let needle = r#""tool""#;
    let mut search_from = 0;

    while let Some(hit_rel) = text[search_from..].find(needle) {
        let abs_hit = search_from + hit_rel;

        // Walk back to find the opening `{`.
        let Some(obj_start) = text[..abs_hit].rfind('{') else {
            search_from = abs_hit + needle.len();
            continue;
        };

        // Check this range isn't already covered by a fenced/XML block.
        if blocks
            .iter()
            .any(|b| obj_start >= b.start && obj_start < b.end)
        {
            search_from = abs_hit + needle.len();
            continue;
        }

        // Brace-count to find end of the JSON object.
        let mut depth = 0i32;
        let mut in_str = false;
        let mut escape = false;
        let mut obj_end = None;
        for (i, ch) in text[obj_start..].char_indices() {
            if in_str {
                if escape {
                    escape = false;
                } else if ch == '\\' {
                    escape = true;
                } else if ch == '"' {
                    in_str = false;
                }
                continue;
            }
            match ch {
                '"' => in_str = true,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        obj_end = Some(obj_start + i + 1);
                        break;
                    }
                },
                _ => {},
            }
        }

        let abs_end = obj_end.unwrap_or(text.len());
        let json_str = &text[obj_start..abs_end];

        if let Some(tc) = try_parse_tool_json(json_str) {
            blocks.push(ParsedBlock {
                tool_call: tc,
                start: obj_start,
                end: abs_end,
            });
            search_from = abs_end;
        } else {
            search_from = abs_hit + needle.len();
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Try to parse a JSON string as a tool call object (`{"tool": ..., "arguments": ...}`).
/// Falls back to `json_repair::repair_json` for slightly malformed JSON.
fn try_parse_tool_json(json_str: &str) -> Option<ToolCall> {
    let parsed = serde_json::from_str::<serde_json::Value>(json_str)
        .ok()
        .or_else(|| json_repair::repair_json(json_str))?;

    let tool_name = parsed["tool"].as_str()?.to_string();
    let arguments = parsed
        .get("arguments")
        .cloned()
        .unwrap_or(serde_json::json!({}));

    Some(ToolCall {
        id: new_synthetic_tool_call_id("text"),
        name: tool_name,
        arguments,
    })
}

fn parse_param_value(value_raw: &str) -> serde_json::Value {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(value_raw) {
        return v;
    }
    serde_json::Value::String(value_raw.to_string())
}

fn trim_tool_call_wrappers(text: &str) -> &str {
    let mut value = text.trim();
    loop {
        let stripped = value
            .strip_prefix("<tool_call>")
            .or_else(|| value.strip_prefix("</tool_call>"));
        match stripped {
            Some(s) => value = s.trim(),
            None => break,
        }
    }
    loop {
        let stripped = value
            .strip_suffix("<tool_call>")
            .or_else(|| value.strip_suffix("</tool_call>"));
        match stripped {
            Some(s) => value = s.trim(),
            None => break,
        }
    }
    value
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_fenced_block() {
        let text = r#"Let me check that.
```tool_call
{"tool": "exec", "arguments": {"command": "ls -la"}}
```
Done."#;
        let (calls, remaining) = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "exec");
        assert_eq!(calls[0].arguments["command"], "ls -la");
        let rem = remaining.unwrap();
        assert!(rem.contains("Let me check that."));
        assert!(rem.contains("Done."));
    }

    #[test]
    fn parse_multiple_fenced_blocks() {
        let text = r#"Step 1:
```tool_call
{"tool": "exec", "arguments": {"command": "mkdir test"}}
```
Step 2:
```tool_call
{"tool": "exec", "arguments": {"command": "cd test"}}
```"#;
        let (calls, remaining) = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].arguments["command"], "mkdir test");
        assert_eq!(calls[1].arguments["command"], "cd test");
        let rem = remaining.unwrap();
        assert!(rem.contains("Step 1:"));
        assert!(rem.contains("Step 2:"));
    }

    #[test]
    fn parse_xml_function_call() {
        let text = r#"<tool_call>
<function=exec>
<parameter=command>pwd</parameter>
</function>
</tool_call>"#;
        let (calls, remaining) = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "exec");
        assert_eq!(calls[0].arguments["command"], "pwd");
        // All text is inside the block, so remaining should be empty/None.
        assert!(
            remaining.is_none() || remaining.as_deref() == Some(""),
            "remaining: {remaining:?}"
        );
    }

    #[test]
    fn parse_bare_json() {
        let text = r#"I'll run that command now: {"tool": "exec", "arguments": {"command": "whoami"}} and report back."#;
        let (calls, remaining) = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "exec");
        let rem = remaining.unwrap();
        assert!(rem.contains("I'll run that command now:"));
        assert!(rem.contains("and report back."));
    }

    #[test]
    fn parse_bare_json_with_trailing_comma() {
        let text = r#"{"tool": "calc", "arguments": {"expression": "2+2",}}"#;
        let (calls, _) = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "calc");
    }

    #[test]
    fn backward_compat_parse_tool_call_from_text() {
        let text = r#"```tool_call
{"tool": "exec", "arguments": {"command": "ls"}}
```"#;
        let (tc, remaining) = parse_tool_call_from_text(text).unwrap();
        assert_eq!(tc.name, "exec");
        assert!(
            remaining.is_none() || remaining.as_deref() == Some(""),
            "remaining: {remaining:?}"
        );
    }

    #[test]
    fn looks_like_failed_tool_call_positive() {
        assert!(looks_like_failed_tool_call(&Some(
            r#"Here's what I'll do: {"tool": "exec", "arguments": {BROKEN"#.into()
        )));
        assert!(looks_like_failed_tool_call(&Some(
            "tool_call something".into()
        )));
    }

    #[test]
    fn looks_like_failed_tool_call_negative() {
        assert!(!looks_like_failed_tool_call(&None));
        assert!(!looks_like_failed_tool_call(&Some(
            "Hello, how can I help?".into()
        )));
    }

    #[test]
    fn no_tool_calls_returns_original_text() {
        let text = "Just a normal response with no tool calls.";
        let (calls, remaining) = parse_tool_calls_from_text(text);
        assert!(calls.is_empty());
        assert_eq!(remaining.as_deref(), Some(text));
    }

    #[test]
    fn fenced_with_malformed_json_repaired() {
        let text = r#"```tool_call
{"tool": "exec", "arguments": {"command": "ls",}}
```"#;
        let (calls, _) = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "exec");
    }

    #[test]
    fn synthetic_ids_are_unique() {
        let id1 = new_synthetic_tool_call_id("text");
        let id2 = new_synthetic_tool_call_id("text");
        assert_ne!(id1, id2);
        assert!(id1.len() <= SYNTHETIC_TOOL_CALL_ID_MAX_LEN);
    }
}
