use pulldown_cmark::{Options, Parser, html};

const MAX_MESSAGE_LEN: usize = 32_000;

pub fn markdown_to_html(input: &str) -> String {
    let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS;
    let parser = Parser::new_ext(input, opts);
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    html_output
}

pub fn chunk_message(text: &str, max_len: usize) -> Vec<&str> {
    if text.len() <= max_len {
        return vec![text];
    }

    let mut chunks = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let end = (start + max_len).min(text.len());
        let boundary = if end < text.len() {
            text[start..end]
                .rfind('\n')
                .map(|pos| start + pos + 1)
                .unwrap_or(end)
        } else {
            end
        };
        chunks.push(&text[start..boundary]);
        start = boundary;
    }
    chunks
}

pub fn default_max_message_len() -> usize {
    MAX_MESSAGE_LEN
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_markdown() {
        let html = markdown_to_html("**bold** and *italic*");
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("<em>italic</em>"));
    }

    #[test]
    fn code_block() {
        let html = markdown_to_html("```rust\nfn main() {}\n```");
        assert!(html.contains("<code"));
        assert!(html.contains("fn main()"));
    }

    #[test]
    fn chunk_short_message() {
        let chunks = chunk_message("hello", 100);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "hello");
    }

    #[test]
    fn chunk_long_message() {
        let text = "a\n".repeat(100);
        let chunks = chunk_message(&text, 50);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.len() <= 50);
        }
    }
}
