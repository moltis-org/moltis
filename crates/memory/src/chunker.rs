//! Split markdown text into overlapping chunks by line, targeting a token count.
//!
//! Tokens are approximated as whitespace-split words.

/// A chunk produced by the chunker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub text: String,
    pub start_line: usize,
    pub end_line: usize,
}

/// Split `text` into chunks of approximately `chunk_size` tokens with `overlap` token overlap.
///
/// Lines are never split mid-line. Each chunk records its 1-based start and end line numbers.
pub fn chunk_markdown(text: &str, chunk_size: usize, overlap: usize) -> Vec<Chunk> {
    if text.is_empty() || chunk_size == 0 {
        return vec![];
    }

    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return vec![];
    }

    let line_tokens: Vec<usize> = lines
        .iter()
        .map(|l| l.split_whitespace().count().max(1)) // empty lines count as 1 token
        .collect();

    let mut chunks = Vec::new();
    let mut start = 0;

    while start < lines.len() {
        let mut end = start;
        let mut tokens = 0;

        // Accumulate lines until we reach chunk_size tokens
        while end < lines.len() && tokens + line_tokens[end] <= chunk_size {
            tokens += line_tokens[end];
            end += 1;
        }

        // If we couldn't fit even one line, take it anyway
        if end == start {
            end = start + 1;
        }

        let chunk_text: String = lines[start..end].join("\n");
        chunks.push(Chunk {
            text: chunk_text,
            start_line: start + 1, // 1-based
            end_line: end,         // 1-based inclusive
        });

        if end >= lines.len() {
            break;
        }

        // Move start forward, keeping `overlap` tokens of context
        let mut overlap_tokens = 0;
        let mut new_start = end;
        while new_start > start && overlap_tokens < overlap {
            new_start -= 1;
            overlap_tokens += line_tokens[new_start];
        }

        // Ensure progress
        if new_start <= start {
            new_start = start + 1;
        }
        start = new_start;
    }

    chunks
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty() {
        assert!(chunk_markdown("", 400, 80).is_empty());
    }

    #[test]
    fn test_single_small_chunk() {
        let text = "hello world\nfoo bar";
        let chunks = chunk_markdown(text, 400, 80);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 2);
        assert_eq!(chunks[0].text, text);
    }

    #[test]
    fn test_multiple_chunks_with_overlap() {
        // Create text with ~10 tokens per line, 5 lines = 50 tokens
        let lines: Vec<String> = (0..10)
            .map(|i| format!("line {} has several words in it here now ok", i))
            .collect();
        let text = lines.join("\n");

        let chunks = chunk_markdown(&text, 20, 5);
        assert!(chunks.len() > 1);

        // Verify overlap: last lines of chunk N should overlap with first lines of chunk N+1
        for i in 0..chunks.len() - 1 {
            assert!(
                chunks[i + 1].start_line <= chunks[i].end_line,
                "chunk {} end_line {} should overlap with chunk {} start_line {}",
                i,
                chunks[i].end_line,
                i + 1,
                chunks[i + 1].start_line
            );
        }

        // Verify all lines are covered
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks.last().unwrap().end_line, 10);
    }

    #[test]
    fn test_line_numbers_are_1_based() {
        let text = "a\nb\nc";
        let chunks = chunk_markdown(text, 1, 0);
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 1);
    }

    #[test]
    fn test_zero_chunk_size() {
        assert!(chunk_markdown("hello", 0, 0).is_empty());
    }
}
