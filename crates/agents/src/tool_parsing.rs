//! Text-based tool call parsing for models without native tool calling.
//!
//! Extracts structured `ToolCall` values from free-form LLM text output.
//! Supports multiple formats:
//!
//! 1. **Fenced blocks**: `` ```tool_call\n{...}\n``` ``
//! 2. **XML function calls**: `<function=name><parameter=key>value</parameter></function>`
//! 3. **XML invoke calls**: `<invoke name="..."><arg name="...">value</arg></invoke>`
//! 4. **Zhipu (Z.AI) XML**: `<tool_call>name<arg_key>k</arg_key><arg_value>v</arg_value></tool_call>`
//! 5. **Bare JSON**: `{"tool": "name", "arguments": {...}}`

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

fn is_valid_tool_name(tool_name: &str) -> bool {
    !tool_name.is_empty()
        && tool_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn new_text_tool_call(
    name: &str,
    arguments: serde_json::Map<String, serde_json::Value>,
) -> ToolCall {
    ToolCall {
        id: new_synthetic_tool_call_id("text"),
        name: name.to_string(),
        arguments: serde_json::Value::Object(arguments),
        metadata: None,
    }
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

    // 3. Find XML <invoke name="..."><arg name="...">value</arg></invoke> blocks.
    collect_invoke_blocks(text, &mut blocks);

    // 4. Find Zhipu (Z.AI) <tool_call>name<arg_key>...</arg_key><arg_value>...</arg_value></tool_call> blocks.
    collect_zhipu_blocks(text, &mut blocks);

    // 5. Find bare JSON {"tool": ...} blocks.
    collect_bare_json_blocks(text, &mut blocks);

    if blocks.is_empty() {
        return (vec![], Some(text.to_string()));
    }

    // Sort by start position and de-overlap (keep first occurrence).
    blocks.sort_by_key(|b| b.start);
    let mut merged: Vec<ParsedBlock> = Vec::with_capacity(blocks.len());
    for block in blocks {
        if let Some(last) = merged.last()
            && block.start < last.end
        {
            continue; // overlapping — skip
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
    (lower.contains("\"tool\"")
        || lower.contains("tool_call")
        || lower.contains("<function=")
        || lower.contains("<invoke")
        || lower.contains("<arg_key>")
        || lower.contains("<arg_value>"))
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
        if !is_valid_tool_name(tool_name) {
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
                tool_call: new_text_tool_call(tool_name, args),
                start: abs_start,
                end: final_end,
            });
        }
        search_from = final_end;
    }
}

// ── XML <invoke> parser ─────────────────────────────────────────────────────

/// Extract the value of a named attribute from an XML attribute string.
/// E.g. `extract_xml_attr(r#"name="exec" id="1""#, "name")` → `Some("exec")`.
fn extract_xml_attr<'a>(attr_str: &'a str, attr_name: &str) -> Option<&'a str> {
    let needle = format!("{attr_name}=\"");
    let start = attr_str.find(&needle)?;
    let value_start = start + needle.len();
    let rest = attr_str.get(value_start..)?;
    let end = rest.find('"')?;
    Some(&rest[..end])
}

/// Collect `<invoke name="tool"><arg name="key">value</arg></invoke>` blocks.
fn collect_invoke_blocks(text: &str, blocks: &mut Vec<ParsedBlock>) {
    let start_marker = "<invoke";
    let mut search_from = 0;

    while let Some(start_rel) = text[search_from..].find(start_marker) {
        let abs_start = search_from + start_rel;
        let after_marker = abs_start + start_marker.len();
        let rest = &text[after_marker..];

        // Find the closing `>` of the opening `<invoke ...>` tag.
        let Some(open_end_rel) = rest.find('>') else {
            break;
        };

        let attr_str = &rest[..open_end_rel];
        let Some(tool_name) = extract_xml_attr(attr_str, "name") else {
            search_from = after_marker + open_end_rel + 1;
            continue;
        };
        if !is_valid_tool_name(tool_name) {
            search_from = after_marker + open_end_rel + 1;
            continue;
        }

        let body_start = after_marker + open_end_rel + 1;
        let Some(after_open) = text.get(body_start..) else {
            break;
        };
        let Some(body_end_rel) = after_open.find("</invoke>") else {
            search_from = body_start;
            continue;
        };
        let body = &after_open[..body_end_rel];
        let abs_end = body_start + body_end_rel + "</invoke>".len();

        // Parse <arg name="key">value</arg> pairs.
        let mut args = serde_json::Map::new();
        let mut found = false;
        let mut cursor = 0usize;
        while let Some(arg_rel) = body[cursor..].find("<arg") {
            let arg_start = cursor + arg_rel;
            let after_arg_tag = arg_start + "<arg".len();
            let Some(arg_rest) = body.get(after_arg_tag..) else {
                break;
            };
            let Some(arg_gt) = arg_rest.find('>') else {
                break;
            };
            let arg_attrs = &arg_rest[..arg_gt];
            let Some(param_name) = extract_xml_attr(arg_attrs, "name") else {
                cursor = after_arg_tag + arg_gt + 1;
                continue;
            };
            if param_name.is_empty() {
                cursor = after_arg_tag + arg_gt + 1;
                continue;
            }

            let value_start = after_arg_tag + arg_gt + 1;
            let Some(value_rest) = body.get(value_start..) else {
                break;
            };
            let Some(value_end_rel) = value_rest.find("</arg>") else {
                break;
            };
            let value_raw = body
                .get(value_start..value_start + value_end_rel)
                .unwrap_or("")
                .trim();

            args.insert(param_name.to_string(), parse_param_value(value_raw));
            found = true;
            cursor = value_start + value_end_rel + "</arg>".len();
        }

        if found {
            blocks.push(ParsedBlock {
                tool_call: new_text_tool_call(tool_name, args),
                start: abs_start,
                end: abs_end,
            });
        }
        search_from = abs_end;
    }
}

// ── Zhipu (Z.AI) XML parser ─────────────────────────────────────────────────

/// Collect Zhipu/Z.AI proprietary tool-call blocks.
///
/// Shape (as emitted by Zhipu's `tool_mode = "text"`):
///
/// ```xml
/// <tool_call>exec<arg_key>command</arg_key><arg_value>ls -la</arg_value>
/// <arg_key>timeout</arg_key><arg_value>10</arg_value></tool_call>
/// ```
///
/// The tool name is the plain-text prefix between `<tool_call>` and the first
/// `<arg_key>`; arguments are interleaved `<arg_key>`/`<arg_value>` pairs.
///
/// Skipped when:
/// - the body begins with `{` — that's a JSON-wrapped `<tool_call>{...}</tool_call>`
///   handled by `response_sanitizer::recover_tool_calls_from_content` or the
///   bare-JSON parser (and the merge step de-overlaps so we don't double-count);
/// - the extracted tool name is empty or contains non-identifier characters —
///   prevents stray prose like "<tool_call>maybe</tool_call>" from matching;
/// - a `<tool_call>` has no closing `</tool_call>` — stop parsing because no
///   later complete block can be recovered from the remaining suffix.
fn collect_zhipu_blocks(text: &str, blocks: &mut Vec<ParsedBlock>) {
    let open = "<tool_call>";
    let close = "</tool_call>";
    let mut cursor = 0;

    while let Some(rel_start) = text[cursor..].find(open) {
        let abs_start = cursor + rel_start;
        let content_start = abs_start + open.len();
        let Some(rel_end) = text[content_start..].find(close) else {
            break;
        };
        let abs_end = content_start + rel_end + close.len();
        let inner = &text[content_start..content_start + rel_end];

        // Defer to the JSON-wrapper recovery path for `<tool_call>{...}</tool_call>`.
        if inner.trim_start().starts_with('{') {
            cursor = abs_end;
            continue;
        }

        // Tool name is everything before the first `<arg_key>` (or the whole
        // inner body if there are no args at all).
        let name_end = inner.find("<arg_key>").unwrap_or(inner.len());
        let tool_name = inner[..name_end].trim();
        if !is_valid_tool_name(tool_name) {
            // Not a Zhipu-shaped block — advance past the opener so a later
            // `<tool_call>` can still match.
            cursor = abs_start + open.len();
            continue;
        }

        // A block with no <arg_key>/<arg_value> pairs is ambiguous — let other
        // parsers (e.g. the `<function=...>` collector inside the wrapper) own
        // it rather than producing a zero-argument call.
        if name_end == inner.len() {
            cursor = abs_end;
            continue;
        }

        let mut args = serde_json::Map::new();
        let mut arg_cursor = name_end;
        let mut parsed_any = false;
        while let Some(key_rel) = inner[arg_cursor..].find("<arg_key>") {
            let key_content_start = arg_cursor + key_rel + "<arg_key>".len();
            let Some(key_end) = inner[key_content_start..].find("</arg_key>") else {
                break;
            };
            let key = inner[key_content_start..key_content_start + key_end].trim();

            let after_key = key_content_start + key_end + "</arg_key>".len();
            let Some(val_rel) = inner[after_key..].find("<arg_value>") else {
                break;
            };
            let val_content_start = after_key + val_rel + "<arg_value>".len();
            let Some(val_end) = inner[val_content_start..].find("</arg_value>") else {
                break;
            };
            let val = &inner[val_content_start..val_content_start + val_end];

            if !key.is_empty() {
                args.insert(key.to_string(), parse_param_value(val));
                parsed_any = true;
            }
            arg_cursor = val_content_start + val_end + "</arg_value>".len();
        }

        if parsed_any {
            blocks.push(ParsedBlock {
                tool_call: new_text_tool_call(tool_name, args),
                start: abs_start,
                end: abs_end,
            });
        }
        cursor = abs_end;
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
        metadata: None,
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
    fn parse_xml_function_preserves_falsy_and_null_values() {
        let text = r#"<tool_call>
<function=grep>
<parameter=offset>0</parameter>
<parameter=multiline>false</parameter>
<parameter=type>null</parameter>
</function>
</tool_call>"#;
        let (calls, remaining) = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "grep");
        assert_eq!(calls[0].arguments["offset"], 0);
        assert_eq!(calls[0].arguments["multiline"], false);
        assert!(calls[0].arguments["type"].is_null());
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
    fn parse_bare_json_preserves_falsy_and_null_values() {
        let text =
            r#"{"tool": "grep", "arguments": {"offset": 0, "multiline": false, "type": null}}"#;
        let (calls, _) = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "grep");
        assert_eq!(calls[0].arguments["offset"], 0);
        assert_eq!(calls[0].arguments["multiline"], false);
        assert!(calls[0].arguments["type"].is_null());
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

    // ── invoke XML format ─────────────────────────────────────────────

    #[test]
    fn parse_single_invoke_block() {
        let text = r#"I'll execute the command.
<invoke name="exec"><arg name="command">ls -la</arg></invoke>
Done."#;
        let (calls, remaining) = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "exec");
        assert_eq!(calls[0].arguments["command"], "ls -la");
        let rem = remaining.unwrap();
        assert!(rem.contains("I'll execute the command."));
        assert!(rem.contains("Done."));
    }

    #[test]
    fn parse_invoke_multiple_args() {
        let text = r#"<invoke name="web_fetch"><arg name="url">https://example.com</arg><arg name="method">GET</arg></invoke>"#;
        let (calls, remaining) = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web_fetch");
        assert_eq!(calls[0].arguments["url"], "https://example.com");
        assert_eq!(calls[0].arguments["method"], "GET");
        assert!(
            remaining.is_none() || remaining.as_deref() == Some(""),
            "remaining: {remaining:?}"
        );
    }

    #[test]
    fn looks_like_failed_invoke() {
        assert!(looks_like_failed_tool_call(&Some(
            r#"<invoke name="exec"><arg name="command">ls"#.into()
        )));
    }

    #[test]
    fn extract_xml_attr_basic() {
        assert_eq!(
            extract_xml_attr(r#" name="exec" id="1""#, "name"),
            Some("exec")
        );
        assert_eq!(extract_xml_attr(r#" name="exec" id="1""#, "id"), Some("1"));
        assert_eq!(extract_xml_attr(r#" name="exec""#, "missing"), None);
    }

    // ── Backward compatibility: existing formats unaffected by invoke parser ──

    /// Fenced blocks must still work identically after adding the invoke parser.
    #[test]
    fn backward_compat_fenced_still_works() {
        let text = r#"```tool_call
{"tool": "exec", "arguments": {"command": "pwd"}}
```"#;
        let (calls, remaining) = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "exec");
        assert_eq!(calls[0].arguments["command"], "pwd");
        assert!(
            remaining.is_none() || remaining.as_deref() == Some(""),
            "remaining: {remaining:?}"
        );
    }

    /// XML <function=...> format must still work identically.
    #[test]
    fn backward_compat_function_xml_still_works() {
        let text = r#"<function=exec>
<parameter=command>ls</parameter>
</function>"#;
        let (calls, remaining) = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "exec");
        assert_eq!(calls[0].arguments["command"], "ls");
        assert!(
            remaining.is_none() || remaining.as_deref() == Some(""),
            "remaining: {remaining:?}"
        );
    }

    /// Bare JSON must still work identically.
    #[test]
    fn backward_compat_bare_json_still_works() {
        let text = r#"Let me run: {"tool": "exec", "arguments": {"command": "whoami"}}"#;
        let (calls, remaining) = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "exec");
        let rem = remaining.unwrap();
        assert!(rem.contains("Let me run:"));
    }

    // ── Invoke parser edge cases ──────────────────────────────────────

    /// Invoke without any <arg> children should NOT produce a tool call.
    #[test]
    fn invoke_without_args_not_parsed() {
        let text = r#"<invoke name="exec"></invoke>"#;
        let (calls, remaining) = parse_tool_calls_from_text(text);
        assert!(
            calls.is_empty(),
            "invoke with no args should not produce a call"
        );
        assert_eq!(remaining.as_deref(), Some(text));
    }

    /// Invoke with missing closing tag is gracefully skipped.
    #[test]
    fn invoke_unclosed_skipped() {
        let text = r#"before <invoke name="exec"><arg name="cmd">ls</arg> after"#;
        let (calls, _remaining) = parse_tool_calls_from_text(text);
        assert!(
            calls.is_empty(),
            "unclosed invoke should not produce a call"
        );
    }

    /// Invoke without name attribute is skipped.
    #[test]
    fn invoke_missing_name_attr_skipped() {
        let text = r#"<invoke><arg name="cmd">ls</arg></invoke>"#;
        let (calls, _) = parse_tool_calls_from_text(text);
        assert!(calls.is_empty());
    }

    /// Invoke with empty name is skipped.
    #[test]
    fn invoke_empty_name_skipped() {
        let text = r#"<invoke name=""><arg name="cmd">ls</arg></invoke>"#;
        let (calls, _) = parse_tool_calls_from_text(text);
        assert!(calls.is_empty());
    }

    /// Invoke with JSON value in arg body is parsed as structured value.
    #[test]
    fn invoke_json_arg_value() {
        let text = r#"<invoke name="exec"><arg name="config">{"verbose": true}</arg></invoke>"#;
        let (calls, _) = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["config"]["verbose"], true);
    }

    /// Invoke with multiline arg value.
    #[test]
    fn invoke_multiline_arg_value() {
        let text = r#"<invoke name="exec"><arg name="command">echo "hello
world"</arg></invoke>"#;
        let (calls, _) = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["command"], "echo \"hello\nworld\"");
    }

    // ── Mixed format: invoke does not interfere with other parsers ────

    /// Fenced block + invoke block together: both parsed correctly.
    #[test]
    fn mixed_fenced_and_invoke() {
        let text = r#"Step 1:
```tool_call
{"tool": "exec", "arguments": {"command": "mkdir test"}}
```
Step 2:
<invoke name="exec"><arg name="command">cd test</arg></invoke>"#;
        let (calls, remaining) = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].arguments["command"], "mkdir test");
        assert_eq!(calls[1].name, "exec");
        assert_eq!(calls[1].arguments["command"], "cd test");
        let rem = remaining.unwrap();
        assert!(rem.contains("Step 1:"));
        assert!(rem.contains("Step 2:"));
    }

    /// Multiple invoke blocks in one text.
    #[test]
    fn multiple_invoke_blocks() {
        let text = r#"<invoke name="exec"><arg name="command">ls</arg></invoke>
<invoke name="web_search"><arg name="query">rust</arg></invoke>"#;
        let (calls, _) = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "exec");
        assert_eq!(calls[1].name, "web_search");
    }

    // ── looks_like_failed_tool_call: no false positives ──────────────

    /// English prose containing "invoke" must NOT be flagged as a failed tool call.
    #[test]
    fn looks_like_failed_invoke_no_false_positive_english() {
        // No `<invoke` — just the word "invoke" in prose.
        assert!(!looks_like_failed_tool_call(&Some(
            "I'll invoke the API to get results.".into()
        )));
    }

    /// Valid invoke block that is successfully parsed should NOT be flagged.
    #[test]
    fn looks_like_failed_invoke_valid_block_not_flagged() {
        // This is a well-formed invoke that parses successfully — parse_tool_call_from_text
        // returns Some, so looks_like_failed_tool_call returns false.
        let valid = r#"<invoke name="exec"><arg name="command">ls</arg></invoke>"#;
        assert!(!looks_like_failed_tool_call(&Some(valid.into())));
    }

    // ── Zhipu (Z.AI) XML format ─────────────────────────────────────
    //
    // Regression coverage for GitHub issue #637: Z.AI `tool_mode="text"` falls
    // back to a proprietary XML shape that previously leaked into channel
    // streams unparsed because none of the sibling collectors recognized it.

    #[test]
    fn parse_single_zhipu_block() {
        let text = r#"I'll run the command.
<tool_call>exec<arg_key>command</arg_key><arg_value>grep -A20 'hello' /tmp/test.txt</arg_value><arg_key>timeout</arg_key><arg_value>10</arg_value></tool_call>
Done."#;
        let (calls, remaining) = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "exec");
        assert_eq!(
            calls[0].arguments["command"],
            "grep -A20 'hello' /tmp/test.txt"
        );
        // `parse_param_value` promotes JSON-parseable strings — `10` becomes a
        // number, matching how the `<function=...>` collector treats values.
        assert_eq!(calls[0].arguments["timeout"], 10);
        let rem = remaining.unwrap();
        assert!(rem.contains("I'll run the command."));
        assert!(rem.contains("Done."));
    }

    #[test]
    fn parse_zhipu_block_single_arg() {
        let text = r#"<tool_call>web_search<arg_key>query</arg_key><arg_value>rust lifetimes</arg_value></tool_call>"#;
        let (calls, remaining) = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web_search");
        assert_eq!(calls[0].arguments["query"], "rust lifetimes");
        assert!(
            remaining.is_none() || remaining.as_deref() == Some(""),
            "remaining: {remaining:?}"
        );
    }

    #[test]
    fn parse_multiple_zhipu_blocks() {
        let text = r#"<tool_call>exec<arg_key>command</arg_key><arg_value>ls</arg_value></tool_call>
between
<tool_call>exec<arg_key>command</arg_key><arg_value>pwd</arg_value></tool_call>"#;
        let (calls, remaining) = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].arguments["command"], "ls");
        assert_eq!(calls[1].arguments["command"], "pwd");
        let rem = remaining.unwrap();
        assert!(rem.contains("between"));
    }

    /// Unclosed Zhipu block is gracefully skipped — no panic, no partial call.
    #[test]
    fn zhipu_unclosed_block_skipped() {
        let text =
            r#"before <tool_call>exec<arg_key>command</arg_key><arg_value>ls</arg_value> no-close"#;
        let (calls, _remaining) = parse_tool_calls_from_text(text);
        assert!(calls.is_empty());
    }

    /// Empty tool name is rejected (not a Zhipu block, advance past opener).
    #[test]
    fn zhipu_empty_tool_name_skipped() {
        let text = r#"<tool_call><arg_key>command</arg_key><arg_value>ls</arg_value></tool_call>"#;
        let (calls, _) = parse_tool_calls_from_text(text);
        assert!(calls.is_empty());
    }

    /// Tool name with whitespace/punctuation (not a bare identifier) is rejected.
    #[test]
    fn zhipu_prose_tool_name_skipped() {
        let text = r#"<tool_call>maybe I should</tool_call>"#;
        let (calls, _) = parse_tool_calls_from_text(text);
        assert!(calls.is_empty());
    }

    /// A valid-looking Zhipu tool name without any arg pairs is ambiguous and
    /// should be skipped rather than inventing a zero-argument call.
    #[test]
    fn zhipu_no_arg_pairs_skipped() {
        let text = r#"<tool_call>exec</tool_call>"#;
        let (calls, remaining) = parse_tool_calls_from_text(text);
        assert!(calls.is_empty());
        assert_eq!(remaining.as_deref(), Some(text));
    }

    /// A Zhipu block with a JSON body (`<tool_call>{"tool":...}</tool_call>`)
    /// must defer to the JSON/recover path — the bare-JSON collector handles
    /// it, and the merge step de-overlaps so we don't double-count.
    #[test]
    fn zhipu_parser_defers_to_json_wrapper() {
        let text = r#"<tool_call>{"tool": "exec", "arguments": {"command": "ls"}}</tool_call>"#;
        let (calls, _) = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "exec");
        assert_eq!(calls[0].arguments["command"], "ls");
    }

    /// Zhipu arg values that look like JSON should be promoted to structured
    /// values (matches sibling collector behavior via `parse_param_value`).
    #[test]
    fn zhipu_json_arg_value_promoted() {
        let text = r#"<tool_call>exec<arg_key>config</arg_key><arg_value>{"verbose": true}</arg_value></tool_call>"#;
        let (calls, _) = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["config"]["verbose"], true);
    }

    /// Zhipu block with a multiline arg value.
    #[test]
    fn zhipu_multiline_arg_value() {
        let text = "<tool_call>exec<arg_key>command</arg_key><arg_value>echo \"hello\nworld\"</arg_value></tool_call>";
        let (calls, _) = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["command"], "echo \"hello\nworld\"");
    }

    /// Empty arg keys are ignored; without any valid key/value pairs the block
    /// must be skipped entirely.
    #[test]
    fn zhipu_empty_arg_key_skipped() {
        let text = r#"<tool_call>exec<arg_key>   </arg_key><arg_value>ls</arg_value></tool_call>"#;
        let (calls, remaining) = parse_tool_calls_from_text(text);
        assert!(calls.is_empty());
        assert_eq!(remaining.as_deref(), Some(text));
    }

    #[test]
    fn zhipu_missing_arg_key_close_skipped() {
        let text = r#"<tool_call>exec<arg_key>command<arg_value>ls</arg_value></tool_call>"#;
        let (calls, remaining) = parse_tool_calls_from_text(text);
        assert!(calls.is_empty());
        assert_eq!(remaining.as_deref(), Some(text));
    }

    #[test]
    fn zhipu_missing_arg_value_open_skipped() {
        let text = r#"<tool_call>exec<arg_key>command</arg_key>ls</tool_call>"#;
        let (calls, remaining) = parse_tool_calls_from_text(text);
        assert!(calls.is_empty());
        assert_eq!(remaining.as_deref(), Some(text));
    }

    #[test]
    fn zhipu_missing_arg_value_close_skipped() {
        let text = r#"<tool_call>exec<arg_key>command</arg_key><arg_value>ls</tool_call>"#;
        let (calls, remaining) = parse_tool_calls_from_text(text);
        assert!(calls.is_empty());
        assert_eq!(remaining.as_deref(), Some(text));
    }

    /// Exact leaked output from issue #637 must round-trip cleanly.
    #[test]
    fn zhipu_regression_issue_637() {
        let text = r#"<tool_call>exec<arg_key>command</arg_key><arg_value>grep -A20 'hello' /tmp/test.txt</arg_value><arg_key>timeout</arg_key><arg_value>10</arg_value></tool_call>"#;
        let (calls, remaining) = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 1, "expected exactly one parsed tool call");
        assert_eq!(calls[0].name, "exec");
        assert_eq!(
            calls[0].arguments["command"],
            "grep -A20 'hello' /tmp/test.txt"
        );
        assert_eq!(calls[0].arguments["timeout"], 10);
        assert!(
            remaining.is_none() || remaining.as_deref() == Some(""),
            "remaining should be empty, got: {remaining:?}"
        );
    }

    /// A mixed turn with one fenced block and one Zhipu block must yield two
    /// distinct calls in source order and preserve interstitial prose.
    #[test]
    fn mixed_fenced_and_zhipu() {
        let text = r#"Step 1:
```tool_call
{"tool": "exec", "arguments": {"command": "mkdir test"}}
```
Step 2:
<tool_call>exec<arg_key>command</arg_key><arg_value>cd test</arg_value></tool_call>"#;
        let (calls, remaining) = parse_tool_calls_from_text(text);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].arguments["command"], "mkdir test");
        assert_eq!(calls[1].arguments["command"], "cd test");
        let rem = remaining.unwrap();
        assert!(rem.contains("Step 1:"));
        assert!(rem.contains("Step 2:"));
    }

    /// A truncated Zhipu block (model was cut off mid-stream) should be flagged
    /// as a failed tool call so the runner can trigger the malformed-retry path.
    /// Uses `<arg_key>`/`<arg_value>` signal — these are genuinely new markers
    /// added for Zhipu support, so we verify they flip the heuristic on.
    #[test]
    fn looks_like_failed_zhipu_truncated() {
        assert!(looks_like_failed_tool_call(&Some(
            r#"<tool_call>exec<arg_key>command</arg_key><arg_value>grep"#.into()
        )));
    }

    /// A valid Zhipu block that parses successfully must NOT be flagged as failed.
    #[test]
    fn looks_like_failed_zhipu_valid_block_not_flagged() {
        let valid =
            r#"<tool_call>exec<arg_key>command</arg_key><arg_value>ls</arg_value></tool_call>"#;
        assert!(!looks_like_failed_tool_call(&Some(valid.into())));
    }
}
