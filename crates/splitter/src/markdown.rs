//! Line-based markdown/text chunking with overlap.

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
    let mut chunks = Vec::new();
    let mut start = 0;

    while start < lines.len() {
        let mut token_count = 0usize;
        let mut end = start;

        while end < lines.len() {
            let line_tokens = lines[end].split_whitespace().count();
            if token_count + line_tokens > chunk_size && end > start {
                break;
            }
            token_count += line_tokens;
            end += 1;
        }

        let chunk_text = lines[start..end].join("\n");
        chunks.push(Chunk {
            text: chunk_text,
            start_line: start + 1,
            end_line: end,
        });

        if end >= lines.len() {
            break;
        }

        // Advance by (chunk_lines - overlap_lines), at least 1.
        let _chunk_lines = end - start;
        let mut new_start = end.saturating_sub(overlap);
        if new_start <= start {
            new_start = start + 1;
        }
        start = new_start;
    }

    chunks
}

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
        let lines: Vec<String> = (0..10)
            .map(|i| format!("line {} has several words in it here now ok", i))
            .collect();
        let text = lines.join("\n");

        let chunks = chunk_markdown(&text, 20, 5);
        assert!(chunks.len() > 1);

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
