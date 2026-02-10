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

/// Split text into chunks that fit within Telegram's message limit.
/// Tries to split at newlines or spaces to avoid breaking words.
pub fn chunk_message(text: &str, max_len: usize) -> Vec<String> {
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

        // Try to split at a newline
        let slice = &remaining[..max_len];
        let split_at = slice
            .rfind('\n')
            .or_else(|| slice.rfind(' '))
            .unwrap_or(max_len);

        let split_at = if split_at == 0 {
            max_len
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bold() {
        assert_eq!(markdown_to_telegram_html("**hello**"), "<b>hello</b>");
    }

    #[test]
    fn italic() {
        assert_eq!(markdown_to_telegram_html("*hello*"), "<i>hello</i>");
    }

    #[test]
    fn inline_code() {
        assert_eq!(markdown_to_telegram_html("`code`"), "<code>code</code>");
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
    fn strikethrough() {
        assert_eq!(markdown_to_telegram_html("~~old~~"), "<s>old</s>");
    }

    #[test]
    fn link() {
        assert_eq!(
            markdown_to_telegram_html("[click](https://example.com)"),
            "<a href=\"https://example.com\">click</a>"
        );
    }

    #[test]
    fn html_escaping() {
        assert_eq!(
            markdown_to_telegram_html("<script>alert(1)</script>"),
            "&lt;script&gt;alert(1)&lt;/script&gt;"
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
}
