/// Slack message character limit.
pub const SLACK_MAX_MESSAGE_LEN: usize = 40_000;

/// Convert standard Markdown to Slack mrkdwn format.
///
/// - `**bold**` → `*bold*`
/// - `*italic*` / `_italic_` → `_italic_`
/// - `~~strike~~` → `~strike~`
/// - `[text](url)` → `<url|text>`
/// - `# Header` → `*Header*`
pub fn markdown_to_slack(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            // Bold: **text** → *text*
            '*' if chars.peek() == Some(&'*') => {
                chars.next(); // consume second *
                result.push('*');
            },
            // Link: [text](url) → <url|text>
            '[' => {
                let mut link_text = String::new();
                let mut found_close = false;
                for c in chars.by_ref() {
                    if c == ']' {
                        found_close = true;
                        break;
                    }
                    link_text.push(c);
                }
                if found_close && chars.peek() == Some(&'(') {
                    chars.next(); // consume (
                    let mut url = String::new();
                    for c in chars.by_ref() {
                        if c == ')' {
                            break;
                        }
                        url.push(c);
                    }
                    result.push('<');
                    result.push_str(&url);
                    result.push('|');
                    result.push_str(&link_text);
                    result.push('>');
                } else {
                    // Not a link, output as-is.
                    result.push('[');
                    result.push_str(&link_text);
                    if found_close {
                        result.push(']');
                    }
                }
            },
            // Strikethrough: ~~text~~ → ~text~
            '~' if chars.peek() == Some(&'~') => {
                chars.next(); // consume second ~
                result.push('~');
            },
            // Headers: # Header → *Header*
            '#' if result.is_empty() || result.ends_with('\n') => {
                // Skip '#' and optional space(s).
                while chars.peek() == Some(&'#') {
                    chars.next();
                }
                if chars.peek() == Some(&' ') {
                    chars.next();
                }
                // Collect the header text until newline.
                let mut header = String::new();
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                    header.push(c);
                }
                result.push('*');
                result.push_str(header.trim());
                result.push('*');
                result.push('\n');
            },
            _ => result.push(ch),
        }
    }

    result
}

/// Split a message into chunks respecting Slack's character limit.
///
/// Splits at line boundaries when possible.
pub fn chunk_message(text: &str, max_len: usize) -> Vec<&str> {
    if text.len() <= max_len {
        return vec![text];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining);
            break;
        }

        // Find the best split point (last newline within limit).
        let search_range = &remaining[..max_len];
        let split_at = search_range
            .rfind('\n')
            .map(|pos| pos + 1) // Include the newline in the first chunk.
            .unwrap_or_else(|| {
                // No newline found — split at a space.
                search_range
                    .rfind(' ')
                    .map(|pos| pos + 1)
                    .unwrap_or(remaining.floor_char_boundary(max_len))
            });

        let (chunk, rest) = remaining.split_at(split_at);
        chunks.push(chunk);
        remaining = rest;
    }

    chunks
}

/// Strip `<@BOT_ID>` mentions from inbound text.
pub fn strip_mentions(text: &str, bot_user_id: &str) -> String {
    let mention = format!("<@{bot_user_id}>");
    text.replace(&mention, "").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bold_conversion() {
        assert_eq!(markdown_to_slack("**bold**"), "*bold*");
    }

    #[test]
    fn italic_passthrough() {
        // Single * in markdown is italic, maps to Slack italic.
        assert_eq!(markdown_to_slack("*italic*"), "*italic*");
    }

    #[test]
    fn strikethrough_conversion() {
        assert_eq!(markdown_to_slack("~~strike~~"), "~strike~");
    }

    #[test]
    fn link_conversion() {
        assert_eq!(
            markdown_to_slack("[click here](https://example.com)"),
            "<https://example.com|click here>"
        );
    }

    #[test]
    fn header_conversion() {
        assert_eq!(markdown_to_slack("# Hello World"), "*Hello World*\n");
        assert_eq!(markdown_to_slack("## Sub Header"), "*Sub Header*\n");
    }

    #[test]
    fn mixed_formatting() {
        let input = "**bold** and *italic* and [link](http://x.com)";
        let expected = "*bold* and *italic* and <http://x.com|link>";
        assert_eq!(markdown_to_slack(input), expected);
    }

    #[test]
    fn chunk_short_message() {
        let chunks = chunk_message("hello", 100);
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn chunk_at_newline() {
        let text = "line one\nline two\nline three";
        let chunks = chunk_message(text, 20);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], "line one\nline two\n");
        assert_eq!(chunks[1], "line three");
    }

    #[test]
    fn chunk_long_line() {
        let text = "a".repeat(100);
        let chunks = chunk_message(&text, 40);
        assert!(chunks.len() >= 3);
        for chunk in &chunks {
            assert!(chunk.len() <= 40);
        }
    }

    #[test]
    fn strip_bot_mention() {
        assert_eq!(strip_mentions("<@U12345> hello", "U12345"), "hello");
        assert_eq!(
            strip_mentions("hey <@U12345> there", "U12345"),
            "hey  there"
        );
    }

    #[test]
    fn strip_no_mention() {
        assert_eq!(strip_mentions("hello world", "U12345"), "hello world");
    }

    #[test]
    fn broken_link_passthrough() {
        // Incomplete link syntax should pass through.
        assert_eq!(markdown_to_slack("[no link here"), "[no link here");
    }

    #[test]
    fn bracket_without_paren() {
        assert_eq!(markdown_to_slack("[text] more"), "[text] more");
    }
}
