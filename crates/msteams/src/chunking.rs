//! Message text chunking for Teams' message size limits.
//!
//! Teams truncates messages at approximately 28 KB, but a practical safe limit
//! is 4000 characters. This module splits long messages at paragraph, line, or
//! word boundaries.

/// Default maximum characters per chunk for Teams messages.
pub const DEFAULT_CHUNK_LIMIT: usize = 4000;

/// Split `text` into chunks of at most `max_len` characters.
///
/// Tries to split at paragraph boundaries (`\n\n`), then line boundaries
/// (`\n`), then the last space before the limit. Falls back to a hard cut
/// at `max_len` if no split point is found.
pub fn chunk_message(text: &str, max_len: usize) -> Vec<&str> {
    if max_len == 0 {
        return vec![text];
    }
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

        let split_window_end = split_window_end(remaining, max_len);
        let window = &remaining[..split_window_end];
        let split_at = find_split_point(window);

        let (chunk, rest) = remaining.split_at(split_at);
        let chunk = chunk.trim_end();
        if !chunk.is_empty() {
            chunks.push(chunk);
        }
        remaining = rest.trim_start();
    }

    chunks
}

fn split_window_end(text: &str, max_len: usize) -> usize {
    let split_window_end = floor_char_boundary(text, max_len);
    if split_window_end > 0 {
        return split_window_end;
    }
    text.chars()
        .next()
        .map(char::len_utf8)
        .unwrap_or(text.len())
}

fn find_split_point(window: &str) -> usize {
    // Prefer paragraph boundary.
    if let Some(pos) = window.rfind("\n\n") {
        return pos;
    }
    // Then line boundary.
    if let Some(pos) = window.rfind('\n') {
        return pos;
    }
    // Then last space.
    if let Some(pos) = window.rfind(' ') {
        return pos;
    }
    // Hard cut at char boundary.
    floor_char_boundary(window, window.len())
}

/// Find the largest byte offset <= `pos` that falls on a UTF-8 character boundary.
fn floor_char_boundary(s: &str, pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    let mut i = pos;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string() {
        assert_eq!(chunk_message("", 100), vec![""]);
    }

    #[test]
    fn under_limit() {
        assert_eq!(chunk_message("hello world", 100), vec!["hello world"]);
    }

    #[test]
    fn exact_limit() {
        let text = "abcde";
        assert_eq!(chunk_message(text, 5), vec!["abcde"]);
    }

    #[test]
    fn splits_at_paragraph() {
        let text = "first paragraph\n\nsecond paragraph";
        let chunks = chunk_message(text, 20);
        assert_eq!(chunks, vec!["first paragraph", "second paragraph"]);
    }

    #[test]
    fn splits_at_newline() {
        let text = "first line\nsecond line that is longer";
        let chunks = chunk_message(text, 15);
        assert_eq!(chunks[0], "first line");
    }

    #[test]
    fn splits_at_space() {
        let text = "word1 word2 word3 word4";
        let chunks = chunk_message(text, 12);
        assert_eq!(chunks[0], "word1 word2");
    }

    #[test]
    fn hard_cut_no_space() {
        let text = "abcdefghijklmnop";
        let chunks = chunk_message(text, 5);
        assert_eq!(chunks[0], "abcde");
    }

    #[test]
    fn multi_chunk() {
        let text = "aaaa bbbb cccc dddd";
        let chunks = chunk_message(text, 9);
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(chunk.len() <= 9);
        }
    }

    #[test]
    fn zero_limit_returns_whole() {
        let text = "hello world";
        assert_eq!(chunk_message(text, 0), vec!["hello world"]);
    }

    #[test]
    fn unicode_safe() {
        // Each emoji is 4 bytes
        let text = "\u{1F600}\u{1F600}\u{1F600}\u{1F600}\u{1F600}";
        let chunks = chunk_message(text, 8); // fits exactly 2 emoji
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(chunk.is_char_boundary(chunk.len()));
        }
    }

    #[test]
    fn unicode_safe_when_limit_lands_inside_first_codepoint() {
        let text = "😀abc";
        let chunks = chunk_message(text, 1);
        assert_eq!(chunks[0], "😀");
        assert_eq!(chunks.concat(), text);
    }
}
