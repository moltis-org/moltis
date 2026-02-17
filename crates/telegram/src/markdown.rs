/// Convert a subset of Markdown to Telegram-compatible HTML.
///
/// Telegram supports: `<b>`, `<i>`, `<code>`, `<pre>`, `<a href="">`,
/// `<s>` (strikethrough), `<u>` (underline).
///
/// We handle the most common Markdown constructs:
/// - `**bold**` / `__bold__` → `<b>`
/// - `*italic*` / `_italic_` → `<i>`
/// - `` `code` `` → `<code>`
/// - ``` ```lang\nblock``` ``` → `<pre><code class="language-lang">`
/// - `~~strike~~` → `<s>`
/// - `[text](url)` → `<a href="url">`
///
/// HTML special chars in the input are escaped first.
pub fn markdown_to_telegram_html(md: &str) -> String {
    let escaped = escape_html(md);
    let mut chars = escaped.chars().peekable();
    let mut out = String::with_capacity(md.len());
    let mut in_code = false;

    while let Some(&ch) = chars.peek() {
        // Fenced code blocks: ```
        if !in_code && ch == '`' && peek_n(&mut chars, 3) == "```" {
            // Consume the ```
            chars.next();
            chars.next();
            chars.next();
            // Optional language tag (until newline)
            let mut lang = String::new();
            while let Some(&c) = chars.peek() {
                if c == '\n' {
                    chars.next();
                    break;
                }
                lang.push(c);
                chars.next();
            }
            // Collect until closing ```
            let mut block = String::new();
            loop {
                if chars.peek().is_none() {
                    break;
                }
                if peek_n(&mut chars, 3) == "```" {
                    chars.next();
                    chars.next();
                    chars.next();
                    break;
                }
                let Some(c) = chars.next() else {
                    break;
                };
                block.push(c);
            }
            if lang.is_empty() {
                out.push_str("<pre>");
            } else {
                out.push_str(&format!("<pre><code class=\"language-{lang}\">"));
            }
            out.push_str(&block);
            if lang.is_empty() {
                out.push_str("</pre>");
            } else {
                out.push_str("</code></pre>");
            }
            continue;
        }

        // Inline code: `
        if ch == '`' {
            chars.next();
            if in_code {
                out.push_str("</code>");
                in_code = false;
            } else {
                out.push_str("<code>");
                in_code = true;
            }
            continue;
        }

        // Inside inline code, don't process markdown
        if in_code {
            if let Some(c) = chars.next() {
                out.push(c);
            }
            continue;
        }

        // Strikethrough: ~~
        if ch == '~' && peek_n(&mut chars, 2) == "~~" {
            chars.next();
            chars.next();
            let mut content = String::new();
            loop {
                if chars.peek().is_none() {
                    break;
                }
                if peek_n(&mut chars, 2) == "~~" {
                    chars.next();
                    chars.next();
                    break;
                }
                let Some(c) = chars.next() else {
                    break;
                };
                content.push(c);
            }
            out.push_str("<s>");
            out.push_str(&content);
            out.push_str("</s>");
            continue;
        }

        // Bold: **
        if ch == '*' && peek_n(&mut chars, 2) == "**" {
            chars.next();
            chars.next();
            let mut content = String::new();
            loop {
                if chars.peek().is_none() {
                    break;
                }
                if peek_n(&mut chars, 2) == "**" {
                    chars.next();
                    chars.next();
                    break;
                }
                let Some(c) = chars.next() else {
                    break;
                };
                content.push(c);
            }
            out.push_str("<b>");
            out.push_str(&content);
            out.push_str("</b>");
            continue;
        }

        // Italic: * (single)
        if ch == '*' {
            chars.next();
            let mut content = String::new();
            loop {
                if chars.peek().is_none() {
                    break;
                }
                if chars.peek() == Some(&'*') {
                    chars.next();
                    break;
                }
                let Some(c) = chars.next() else {
                    break;
                };
                content.push(c);
            }
            out.push_str("<i>");
            out.push_str(&content);
            out.push_str("</i>");
            continue;
        }

        // Link: [text](url)
        if ch == '[' {
            chars.next();
            let mut text = String::new();
            let mut found_close = false;
            loop {
                match chars.peek() {
                    None => break,
                    Some(&']') => {
                        chars.next();
                        found_close = true;
                        break;
                    },
                    _ => {
                        let Some(c) = chars.next() else {
                            break;
                        };
                        text.push(c);
                    },
                }
            }
            if found_close && chars.peek() == Some(&'(') {
                chars.next();
                let mut url = String::new();
                loop {
                    match chars.peek() {
                        None => break,
                        Some(&')') => {
                            chars.next();
                            break;
                        },
                        _ => {
                            let Some(c) = chars.next() else {
                                break;
                            };
                            url.push(c);
                        },
                    }
                }
                out.push_str(&format!("<a href=\"{url}\">{text}</a>"));
            } else {
                out.push('[');
                out.push_str(&text);
                if found_close {
                    out.push(']');
                }
            }
            continue;
        }

        if let Some(c) = chars.next() {
            out.push(c);
        }
    }

    // Ensure we never leave an unterminated inline code tag when input ends
    // without a closing backtick (common when splitting long content).
    if in_code {
        out.push_str("</code>");
    }

    out
}

/// Peek at the next `n` characters without consuming them.
fn peek_n(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, n: usize) -> String {
    let collected: Vec<char> = chars.clone().take(n).collect();
    collected.into_iter().collect()
}

/// Escape HTML special characters.
fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Telegram message size limit.
pub const TELEGRAM_MAX_MESSAGE_LEN: usize = 4096;

/// Telegram caption size limit for media messages (voice, photo, document).
pub const TELEGRAM_CAPTION_LIMIT: usize = 1024;

#[must_use]
pub fn truncate_at_char_boundary(text: &str, max_len: usize) -> &str {
    &text[..text.floor_char_boundary(max_len)]
}

/// Split text into chunks that fit within Telegram's message limit.
/// Tries to split at newlines or spaces to avoid breaking words.
pub fn chunk_message(text: &str, max_len: usize) -> Vec<String> {
    if max_len == 0 {
        return Vec::new();
    }

    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }

        let mut split_window_end = remaining.floor_char_boundary(max_len);
        if split_window_end == 0 {
            split_window_end = remaining
                .chars()
                .next()
                .map(char::len_utf8)
                .unwrap_or(remaining.len());
        }

        // Try to split at a newline
        let slice = &remaining[..split_window_end];
        let split_at = slice
            .rfind('\n')
            .or_else(|| slice.rfind(' '))
            .unwrap_or(split_window_end);

        let split_at = if split_at == 0 {
            split_window_end
        } else {
            split_at
        };

        chunks.push(remaining[..split_at].to_string());
        remaining = remaining[split_at..].trim_start_matches('\n');
        if remaining.starts_with(' ') {
            remaining = &remaining[1..];
        }
    }

    chunks
}

/// Split markdown into Telegram-safe HTML chunks that each fit `max_len`.
///
/// Unlike splitting rendered HTML directly, this chunks the markdown source and
/// renders each chunk independently so we don't cut through HTML tags.
pub fn chunk_markdown_html(markdown: &str, max_len: usize) -> Vec<String> {
    if max_len == 0 || markdown.is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut remaining = markdown;
    while !remaining.is_empty() {
        let whole_html = markdown_to_telegram_html(remaining);
        if whole_html.len() <= max_len {
            chunks.push(whole_html);
            break;
        }

        let split_at = best_markdown_split(remaining, max_len);
        let head = &remaining[..split_at];
        chunks.push(markdown_to_telegram_html(head));

        remaining = &remaining[split_at..];
        remaining = remaining.trim_start_matches('\n');
        if remaining.starts_with(' ') {
            remaining = &remaining[1..];
        }
    }

    chunks
}

fn best_markdown_split(markdown: &str, max_len: usize) -> usize {
    let mut boundaries: Vec<usize> = markdown.char_indices().map(|(i, _)| i).collect();
    boundaries.push(markdown.len());

    let mut lo = 1usize;
    let mut hi = boundaries.len().saturating_sub(1);
    let mut best = 0usize;

    while lo <= hi {
        let mid = (lo + hi) / 2;
        let split = boundaries[mid];
        let html_len = markdown_to_telegram_html(&markdown[..split]).len();
        if html_len <= max_len {
            best = split;
            lo = mid + 1;
        } else if mid == 0 {
            break;
        } else {
            hi = mid - 1;
        }
    }

    if best == 0 {
        return boundaries.get(1).copied().unwrap_or(markdown.len());
    }

    let preferred_split = markdown[..best]
        .rfind('\n')
        .or_else(|| markdown[..best].rfind(' '))
        .filter(|pos| *pos > 0)
        .unwrap_or(best);

    if preferred_split == best {
        return best;
    }

    if markdown_to_telegram_html(&markdown[..preferred_split]).len() <= max_len {
        preferred_split
    } else {
        best
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("**hello**", "<b>hello</b>")]
    #[case("*hello*", "<i>hello</i>")]
    #[case("`code`", "<code>code</code>")]
    #[case("~~old~~", "<s>old</s>")]
    #[case("<script>alert(1)</script>", "&lt;script&gt;alert(1)&lt;/script&gt;")]
    fn markdown_inline(#[case] input: &str, #[case] expected: &str) {
        assert_eq!(markdown_to_telegram_html(input), expected);
    }

    #[test]
    fn fenced_code_block() {
        let input = "```rust\nfn main() {}\n```";
        let output = markdown_to_telegram_html(input);
        assert!(output.contains("<pre><code class=\"language-rust\">"));
        assert!(output.contains("fn main() {}"));
        assert!(output.contains("</code></pre>"));
    }

    #[test]
    fn link() {
        assert_eq!(
            markdown_to_telegram_html("[click](https://example.com)"),
            "<a href=\"https://example.com\">click</a>"
        );
    }

    #[test]
    fn chunk_short_message() {
        let chunks = chunk_message("hello", 100);
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn chunk_at_newline() {
        let text = "line1\nline2\nline3";
        let chunks = chunk_message(text, 10);
        assert_eq!(chunks[0], "line1");
        assert_eq!(chunks[1], "line2");
        assert_eq!(chunks[2], "line3");
    }

    #[test]
    fn chunk_at_space() {
        let text = "hello world foo bar";
        let chunks = chunk_message(text, 10);
        assert_eq!(chunks[0], "hello");
        assert_eq!(chunks[1], "world foo");
        assert_eq!(chunks[2], "bar");
    }

    #[test]
    fn truncate_at_char_boundary_handles_utf8() {
        let text = format!("{}л{}", "a".repeat(4095), "z");
        let truncated = truncate_at_char_boundary(&text, 4096);
        assert_eq!(truncated.len(), 4095);
        assert!(truncated.chars().all(|c| c == 'a'));
    }

    #[test]
    fn chunk_message_handles_utf8_boundary() {
        let text = format!("{}лz", "a".repeat(4095));
        let chunks = chunk_message(&text, 4096);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 4095);
        assert_eq!(chunks[1], "лz");
    }

    #[test]
    fn markdown_to_telegram_html_closes_unterminated_inline_code() {
        let output = markdown_to_telegram_html("prefix `unterminated");
        assert_eq!(output, "prefix <code>unterminated</code>");
    }

    #[test]
    fn chunk_markdown_html_respects_limit() {
        let input = "**bold** ".repeat(2_000);
        let chunks = chunk_markdown_html(&input, 4096);
        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(|chunk| chunk.len() <= 4096));
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.matches("<b>").count() == chunk.matches("</b>").count())
        );
    }

    #[test]
    fn chunk_markdown_html_handles_long_inline_code() {
        let input = format!("`{}`", "a".repeat(8_500));
        let chunks = chunk_markdown_html(&input, 4096);
        assert!(chunks.len() >= 2);
        assert!(chunks.iter().all(|chunk| chunk.len() <= 4096));
        assert!(
            chunks.iter().all(|chunk| {
                chunk.matches("<code>").count() == chunk.matches("</code>").count()
            })
        );
    }
}
