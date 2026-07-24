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
    let mut code: Option<String> = None;

    let flush_paragraph = |paragraph: &mut String, blocks: &mut Vec<Value>| {
        let trimmed = paragraph.trim();
        if !trimmed.is_empty() {
            for chunk in split_section(&markdown_to_slack(trimmed)) {
                blocks.push(section_block(&chunk));
            }
        }
        paragraph.clear();
    };

    for line in markdown.lines() {
        // Fenced code block toggling.
        if line.trim_start().starts_with("```") {
            match code.take() {
                Some(buf) => {
                    // Closing fence: emit the accumulated code as a section.
                    let fenced = format!("```\n{}\n```", buf.trim_end_matches('\n'));
                    for chunk in split_section(&fenced) {
                        blocks.push(section_block(&chunk));
                    }
                },
                None => {
                    // Opening fence: flush any pending paragraph first.
                    flush_paragraph(&mut paragraph, &mut blocks);
                    code = Some(String::new());
                },
            }
            continue;
        }

        if let Some(buf) = code.as_mut() {
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

        // ATX heading → header block.
        if let Some(text) = heading_text(trimmed) {
            flush_paragraph(&mut paragraph, &mut blocks);
            blocks.push(header_block(&text));
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
    if let Some(buf) = code.take() {
        // Unterminated fence: still emit what we have.
        let fenced = format!("```\n{}\n```", buf.trim_end_matches('\n'));
        for chunk in split_section(&fenced) {
            blocks.push(section_block(&chunk));
        }
    }
    flush_paragraph(&mut paragraph, &mut blocks);

    // Never lose a message: bail to plain text on empty or over-limit output.
    if blocks.is_empty() || blocks.len() > MAX_BLOCKS {
        return None;
    }
    Some(blocks)
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

fn header_block(text: &str) -> Value {
    let truncated = truncate_chars(text, MAX_HEADER_CHARS);
    json!({
        "type": "header",
        "text": { "type": "plain_text", "text": truncated, "emoji": true },
    })
}

fn section_block(mrkdwn: &str) -> Value {
    json!({
        "type": "section",
        "text": { "type": "mrkdwn", "text": mrkdwn },
    })
}

/// Split a section body into pieces no longer than [`MAX_SECTION_CHARS`],
/// preferring line boundaries.
fn split_section(text: &str) -> Vec<String> {
    if text.chars().count() <= MAX_SECTION_CHARS {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    for line in text.split_inclusive('\n') {
        if current.chars().count() + line.chars().count() > MAX_SECTION_CHARS && !current.is_empty()
        {
            out.push(std::mem::take(&mut current));
        }
        // A single line longer than the limit is hard-split by char boundary.
        if line.chars().count() > MAX_SECTION_CHARS {
            let mut buf = line;
            while buf.chars().count() > MAX_SECTION_CHARS {
                let cut = buf
                    .char_indices()
                    .nth(MAX_SECTION_CHARS)
                    .map_or(buf.len(), |(i, _)| i);
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

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut = text
        .char_indices()
        .nth(max.saturating_sub(1))
        .map_or(text.len(), |(i, _)| i);
    format!("{}…", &text[..cut])
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
}
