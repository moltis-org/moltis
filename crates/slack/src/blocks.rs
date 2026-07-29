//! Markdown → Slack Block Kit rendering (opt-in via `rich_blocks`).
//!
//! Renders an assistant reply's block-level structure (headings, dividers,
//! fenced code, paragraphs) into Block Kit blocks so long replies read better
//! than one flat `mrkdwn` blob. Inline formatting (bold/italic/links/inline
//! code) is left to Slack's `mrkdwn`, produced by [`markdown_to_slack`].
//!
//! Hard rule (from hermes): a rich render must never lose a message. If the
//! content cannot be represented within Slack's limits, [`markdown_to_blocks`]
//! returns `None` and the caller falls back to plain chunked text.

use serde_json::{Value, json};

use crate::markdown::markdown_to_slack;

/// Slack Block Kit limits.
const MAX_BLOCKS: usize = 50;
const MAX_SECTION_CHARS: usize = 3000;
const MAX_HEADER_CHARS: usize = 150;

/// Convert markdown to a Block Kit block array, or `None` if it should fall back
/// to plain text (empty, or too large to represent within Slack's limits).
#[must_use]
pub fn markdown_to_blocks(markdown: &str) -> Option<Vec<Value>> {
    let mut blocks: Vec<Value> = Vec::new();
    let mut paragraph = String::new();
    // Accumulated fenced-code body plus the backtick count that opened it.
    let mut code: Option<(String, usize)> = None;

    let flush_paragraph = |paragraph: &mut String, blocks: &mut Vec<Value>| {
        let trimmed = paragraph.trim();
        if !trimmed.is_empty() {
            for chunk in split_by_limit(&markdown_to_slack(trimmed), MAX_SECTION_CHARS) {
                blocks.push(section_block(&chunk));
            }
        }
        paragraph.clear();
    };

    for line in markdown.lines() {
        // Fence handling. CommonMark allows a bare closing run to be longer
        // than its opener; a fence with an info string remains body text.
        if let Some(ticks) = opening_fence_len(line) {
            match &code {
                Some((buf, open_ticks)) => {
                    if is_closing_fence(line, *open_ticks) {
                        push_code_blocks(buf, &mut blocks);
                        code = None;
                        continue;
                    }
                },
                None => {
                    flush_paragraph(&mut paragraph, &mut blocks);
                    code = Some((String::new(), ticks));
                    continue;
                },
            }
        }

        if let Some((buf, _)) = code.as_mut() {
            buf.push_str(line);
            buf.push('\n');
            continue;
        }

        let trimmed = line.trim();

        // Thematic break → divider.
        if matches!(trimmed, "---" | "***" | "___") {
            flush_paragraph(&mut paragraph, &mut blocks);
            blocks.push(json!({ "type": "divider" }));
            continue;
        }

        // ATX heading → header block. A header cannot hold more than
        // MAX_HEADER_CHARS and Slack has no continuation, so an overlong
        // heading is rendered as a bold section instead of being truncated —
        // the renderer must never drop content.
        if let Some(text) = heading_text(trimmed) {
            flush_paragraph(&mut paragraph, &mut blocks);
            if text.chars().count() > MAX_HEADER_CHARS {
                for chunk in split_by_limit(&format!("*{text}*"), MAX_SECTION_CHARS) {
                    blocks.push(section_block(&chunk));
                }
            } else {
                blocks.push(header_block(&strip_inline_markdown(&text)));
            }
            continue;
        }

        // Blank line → paragraph boundary.
        if trimmed.is_empty() {
            flush_paragraph(&mut paragraph, &mut blocks);
            continue;
        }

        if !paragraph.is_empty() {
            paragraph.push('\n');
        }
        paragraph.push_str(line);
    }

    // Flush trailing state.
    if let Some((buf, _)) = code.take() {
        // Unterminated fence: still emit what we have.
        push_code_blocks(&buf, &mut blocks);
    }
    flush_paragraph(&mut paragraph, &mut blocks);

    // Never lose a message: bail to plain text on empty or over-limit output.
    if blocks.is_empty() || blocks.len() > MAX_BLOCKS {
        return None;
    }
    Some(blocks)
}

/// Backtick count if this line opens a fence (three or more backticks,
/// optionally followed by an info string such as ```rust).
fn opening_fence_len(line: &str) -> Option<usize> {
    let trimmed = line.trim_start();
    let ticks = trimmed.chars().take_while(|c| *c == '`').count();
    (ticks >= 3).then_some(ticks)
}

/// A bare backtick run closes a block when it is at least as long as the opener.
fn is_closing_fence(line: &str, open_ticks: usize) -> bool {
    let trimmed = line.trim();
    trimmed.len() >= open_ticks && trimmed.chars().all(|c| c == '`')
}

/// Extract heading text from an ATX heading line (`# …` … `###### …`).
fn heading_text(line: &str) -> Option<String> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
        Some(line[hashes..].trim().to_string())
    } else {
        None
    }
}

/// Reduce a heading to plain text for a `header` block.
///
/// A header's text is a `plain_text` field, which Slack renders verbatim — it
/// does not interpret mrkdwn. Headings from an LLM routinely carry inline
/// formatting (`## Install \`foo\``, `## **Important**`), and passing that
/// through unchanged shows the raw syntax to the user.
///
/// Only unambiguously paired constructs are unwrapped: links keep their label,
/// and inline code, bold and strikethrough lose their delimiters. Single `*`/`_`
/// are deliberately left alone — in a heading they are far more likely to be
/// part of an identifier (`API_KEY`, `snake_case`) than an emphasis marker, and
/// stripping them would corrupt the text rather than tidy it.
fn strip_inline_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(index) = rest.find(['`', '[', '*', '~']) {
        let (head, tail) = rest.split_at(index);
        out.push_str(head);

        // `label` of a `[label](url)` link, dropping the target.
        if let Some(after) = tail.strip_prefix('[')
            && let Some((label, remainder)) = after.split_once("](")
            && let Some((_url, remainder)) = remainder.split_once(')')
        {
            out.push_str(&strip_inline_markdown(label));
            rest = remainder;
            continue;
        }

        // Paired `code`, **bold**, ~~strike~~ — delimiters removed, body kept.
        let delimiter = match tail.as_bytes() {
            [b'`', ..] => "`",
            [b'*', b'*', ..] => "**",
            [b'~', b'~', ..] => "~~",
            _ => "",
        };
        if !delimiter.is_empty()
            && let Some(after) = tail.strip_prefix(delimiter)
            && let Some((body, remainder)) = after.split_once(delimiter)
        {
            out.push_str(&strip_inline_markdown(body));
            rest = remainder;
            continue;
        }

        // Unpaired marker: keep it verbatim and move past it.
        let mut chars = tail.chars();
        if let Some(ch) = chars.next() {
            out.push(ch);
        }
        rest = chars.as_str();
    }
    out.push_str(rest);
    out
}

/// Build a header block. Callers must ensure `text` fits [`MAX_HEADER_CHARS`];
/// longer headings are rendered as sections so nothing is lost.
fn header_block(text: &str) -> Value {
    json!({
        "type": "header",
        "text": { "type": "plain_text", "text": text, "emoji": true },
    })
}

fn section_block(mrkdwn: &str) -> Value {
    json!({
        "type": "section",
        "text": { "type": "mrkdwn", "text": mrkdwn },
    })
}

/// Emit a code block as one or more fenced section blocks. Oversized code is
/// split on the *raw* content and each chunk re-wrapped in its own ``` fence,
/// so a split never produces unbalanced/mismatched code delimiters.
fn push_code_blocks(code: &str, blocks: &mut Vec<Value>) {
    // Reserve room for the fence delimiters ("```\n" + "\n```") in each chunk.
    let overhead = "```\n\n```".chars().count();
    let body_limit = MAX_SECTION_CHARS.saturating_sub(overhead).max(1);
    // Only the single newline the parser appends per line is dropped; further
    // trailing blank lines are part of the snippet.
    let body = code.strip_suffix('\n').unwrap_or(code);
    for piece in split_by_limit(body, body_limit) {
        blocks.push(section_block(&format!("```\n{piece}\n```")));
    }
}

/// Split a body into pieces no longer than `limit` chars, preferring line
/// boundaries and hard-splitting any single line that exceeds the limit.
fn split_by_limit(text: &str, limit: usize) -> Vec<String> {
    if text.chars().count() <= limit {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    for line in text.split_inclusive('\n') {
        if current.chars().count() + line.chars().count() > limit && !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
        // A single line longer than the limit is hard-split by char boundary.
        if line.chars().count() > limit {
            let mut buf = line;
            while buf.chars().count() > limit {
                let cut = buf.char_indices().nth(limit).map_or(buf.len(), |(i, _)| i);
                out.push(buf[..cut].to_string());
                buf = &buf[cut..];
            }
            current.push_str(buf);
        } else {
            current.push_str(line);
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn empty_returns_none() {
        assert!(markdown_to_blocks("   \n  ").is_none());
    }

    #[test]
    fn heading_becomes_header_block() {
        let blocks = markdown_to_blocks("# Title\n\nbody text").unwrap();
        assert_eq!(blocks[0]["type"], "header");
        assert_eq!(blocks[0]["text"]["text"], "Title");
        assert_eq!(blocks[1]["type"], "section");
    }

    #[test]
    fn thematic_break_becomes_divider() {
        let blocks = markdown_to_blocks("a\n\n---\n\nb").unwrap();
        assert!(blocks.iter().any(|b| b["type"] == "divider"));
    }

    #[test]
    fn fenced_code_is_preserved() {
        let blocks = markdown_to_blocks("intro\n\n```\nlet x = 1;\n```").unwrap();
        let has_code = blocks.iter().any(|b| {
            b["text"]["text"]
                .as_str()
                .is_some_and(|t| t.contains("```"))
        });
        assert!(has_code);
    }

    #[test]
    fn non_heading_hashtag_is_not_a_header() {
        // `#nospace` is not an ATX heading.
        let blocks = markdown_to_blocks("#notaheading").unwrap();
        assert_eq!(blocks[0]["type"], "section");
    }

    #[test]
    fn overlong_heading_becomes_a_section_not_truncated() {
        let heading = "H".repeat(400);
        let blocks = markdown_to_blocks(&format!("# {heading}")).unwrap();
        // Rendered as a section, and the full text survives.
        assert_eq!(blocks[0]["type"], "section");
        let rendered: String = blocks
            .iter()
            .filter_map(|b| b["text"]["text"].as_str())
            .collect();
        assert!(rendered.contains(&heading), "heading content was lost");
    }

    #[test]
    fn longer_backtick_run_closes_fence() {
        let md = "```\nbefore\n````\nafter";
        let blocks = markdown_to_blocks(md).unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["text"]["text"], "```\nbefore\n```");
        assert_eq!(blocks[1]["text"]["text"], "after");
    }

    #[test]
    fn info_string_fence_round_trips_content() {
        let md = "```rust\nlet x = 1;\n```";
        let blocks = markdown_to_blocks(md).unwrap();
        let rendered: String = blocks
            .iter()
            .filter_map(|b| b["text"]["text"].as_str())
            .collect();
        assert!(rendered.contains("let x = 1;"), "code body lost");
        assert_eq!(rendered.matches("```").count(), 2, "unbalanced fences");
    }

    #[test]
    fn interior_blank_lines_in_code_are_preserved() {
        let md = "```\na\n\n\nb\n```";
        let blocks = markdown_to_blocks(md).unwrap();
        let rendered: String = blocks
            .iter()
            .filter_map(|b| b["text"]["text"].as_str())
            .collect();
        assert!(
            rendered.contains("a\n\n\nb"),
            "blank lines collapsed: {rendered:?}"
        );
    }

    #[test]
    fn heading_inline_formatting_is_not_shown_as_raw_syntax() {
        // A header block's text is `plain_text`: Slack shows it verbatim, so
        // markdown delimiters must not reach it.
        let blocks = markdown_to_blocks("## Install `foo` and **restart**").unwrap();
        assert_eq!(blocks[0]["type"], "header");
        assert_eq!(blocks[0]["text"]["text"], "Install foo and restart");
    }

    #[test]
    fn heading_link_keeps_its_label_and_drops_the_target() {
        let blocks = markdown_to_blocks("# See [the docs](https://example.com)").unwrap();
        assert_eq!(blocks[0]["text"]["text"], "See the docs");
    }

    #[test]
    fn heading_identifiers_with_underscores_and_stars_survive_intact() {
        // Single `*`/`_` in a heading are far more likely to be part of an
        // identifier than an emphasis marker; stripping them would corrupt it.
        for heading in ["API_KEY setup", "glob a*b", "snake_case_name"] {
            let blocks = markdown_to_blocks(&format!("# {heading}")).unwrap();
            assert_eq!(blocks[0]["text"]["text"], heading, "{heading}");
        }
    }

    #[test]
    fn unpaired_heading_delimiters_are_left_verbatim() {
        let blocks = markdown_to_blocks("# 50% off `unclosed").unwrap();
        assert_eq!(blocks[0]["text"]["text"], "50% off `unclosed");
    }

    #[test]
    fn over_limit_falls_back_to_none() {
        // Many dividers exceed the 50-block cap → fall back to plain text.
        let md = (0..60).map(|_| "---").collect::<Vec<_>>().join("\n\n");
        assert!(markdown_to_blocks(&md).is_none());
    }

    #[test]
    fn long_section_is_split() {
        let long = "x".repeat(7000);
        let blocks = markdown_to_blocks(&long).unwrap();
        assert!(blocks.len() >= 2);
        for b in &blocks {
            assert!(b["text"]["text"].as_str().unwrap().chars().count() <= MAX_SECTION_CHARS);
        }
    }

    #[test]
    fn long_fenced_code_splits_into_balanced_fences() {
        // A code block far larger than MAX_SECTION_CHARS, many lines.
        let code: String = (0..500).map(|i| format!("line {i} of code\n")).collect();
        let md = format!("```\n{code}```");
        let blocks = markdown_to_blocks(&md).unwrap();
        assert!(blocks.len() >= 2, "expected the code block to split");
        for b in &blocks {
            let text = b["text"]["text"].as_str().unwrap();
            // Every chunk is a self-contained, balanced code fence within limits.
            assert!(
                text.starts_with("```\n"),
                "chunk missing opening fence: {text:.40}"
            );
            assert!(text.ends_with("\n```"), "chunk missing closing fence");
            assert_eq!(text.matches("```").count(), 2, "unbalanced fences in chunk");
            assert!(text.chars().count() <= MAX_SECTION_CHARS);
        }
    }
}
