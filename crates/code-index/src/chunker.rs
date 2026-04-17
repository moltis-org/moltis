//! Code chunking for embedding.
//!
//! Splits source code files into semantically meaningful chunks.
//! Uses a line-based approach with overlap, respecting function boundaries when possible.

use crate::store::CodeChunk;

/// Configuration for code chunking.
#[derive(Debug, Clone)]
pub struct ChunkerConfig {
    /// Target chunk size in lines.
    pub chunk_size_lines: usize,
    /// Overlap between chunks in lines.
    pub overlap_lines: usize,
    /// Maximum chunk size in bytes (approximate).
    pub max_chunk_bytes: usize,
}

impl Default for ChunkerConfig {
    fn default() -> Self {
        Self {
            chunk_size_lines: 50,
            overlap_lines: 5,
            max_chunk_bytes: 4000,
        }
    }
}

/// Chunker for source code files.
pub struct CodeChunker {
    config: ChunkerConfig,
}

impl CodeChunker {
    /// Create a new chunker with the given config.
    pub fn new(config: ChunkerConfig) -> Self {
        Self { config }
    }

    /// Chunk file content into pieces.
    ///
    /// Returns chunks with 1-indexed line numbers.
    pub fn chunk(&self, content: &str, file_path: &str) -> Vec<CodeChunk> {
        let lines: Vec<&str> = content.lines().collect();
        if lines.is_empty() {
            return Vec::new();
        }

        let mut chunks = Vec::new();
        let mut start_line = 0;
        let total_lines = lines.len();

        while start_line < total_lines {
            let end_line = (start_line + self.config.chunk_size_lines).min(total_lines);

            // Build chunk content
            let chunk_lines = &lines[start_line..end_line];
            let chunk_content = chunk_lines.join("\n");

            // Check byte size and split further if needed
            if chunk_content.len() > self.config.max_chunk_bytes && end_line - start_line > 10 {
                // Split by byte size
                let mid = (start_line + end_line) / 2;
                let first_half = &lines[start_line..mid];
                let second_half = &lines[mid..end_line];

                for (idx, half) in [first_half, second_half].iter().enumerate() {
                    let content = half.join("\n");
                    let half_start = if idx == 0 {
                        start_line
                    } else {
                        mid
                    };
                    let half_end = if idx == 0 {
                        mid
                    } else {
                        end_line
                    };

                    chunks.push(CodeChunk {
                        file_path: file_path.to_string(),
                        chunk_index: chunks.len(),
                        content,
                        embedding: None,
                        start_line: half_start + 1,
                        end_line: half_end,
                    });
                }
            } else {
                chunks.push(CodeChunk {
                    file_path: file_path.to_string(),
                    chunk_index: chunks.len(),
                    content: chunk_content,
                    embedding: None,
                    start_line: start_line + 1,
                    end_line,
                });
            }

            // Move forward with overlap
            let step = self
                .config
                .chunk_size_lines
                .saturating_sub(self.config.overlap_lines);
            start_line += step;

            // Prevent infinite loop on small files
            if step == 0 {
                break;
            }
        }

        // Renumber chunk indices to be sequential
        for (idx, chunk) in chunks.iter_mut().enumerate() {
            chunk.chunk_index = idx;
        }

        chunks
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_empty() {
        let chunker = CodeChunker::new(ChunkerConfig::default());
        let chunks = chunker.chunk("", "test.rs");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_chunk_small_file() {
        let chunker = CodeChunker::new(ChunkerConfig::default());
        let content = "line1\nline2\nline3";
        let chunks = chunker.chunk(content, "test.rs");

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 3);
        assert_eq!(chunks[0].content, content);
    }

    #[test]
    fn test_chunk_large_file() {
        let config = ChunkerConfig {
            chunk_size_lines: 10,
            overlap_lines: 2,
            max_chunk_bytes: 10000,
        };
        let chunker = CodeChunker::new(config);

        let mut content = String::new();
        for i in 0..100 {
            content.push_str(&format!("line{}\n", i));
        }

        let chunks = chunker.chunk(&content, "test.rs");

        // 100 lines with chunk_size 10 and overlap 2:
        // chunk 0: lines 0-9
        // chunk 1: lines 8-17 (overlap 2)
        // chunk 2: lines 16-25
        // etc.
        assert!(chunks.len() > 1);

        // Verify line numbers
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 10);

        // Verify overlap
        assert_eq!(chunks[1].start_line, 9);
    }

    #[test]
    fn test_chunk_indices_sequential() {
        let config = ChunkerConfig {
            chunk_size_lines: 5,
            overlap_lines: 0,
            max_chunk_bytes: 10000,
        };
        let chunker = CodeChunker::new(config);

        let content = (0..20)
            .map(|i| format!("line{}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let chunks = chunker.chunk(&content, "test.rs");

        for (idx, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.chunk_index, idx);
        }
    }

    #[test]
    fn test_chunk_exact_boundary() {
        let config = ChunkerConfig {
            chunk_size_lines: 10,
            overlap_lines: 0,
            max_chunk_bytes: 100_000,
        };
        let chunker = CodeChunker::new(config);
        let content: String = (0..10)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");

        let chunks = chunker.chunk(&content, "test.rs");
        assert_eq!(
            chunks.len(),
            1,
            "exactly 10 lines with chunk_size 10 should be 1 chunk"
        );
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 10);
    }

    #[test]
    fn test_chunk_single_long_line() {
        let config = ChunkerConfig {
            chunk_size_lines: 50,
            overlap_lines: 0,
            max_chunk_bytes: 20,
        };
        let chunker = CodeChunker::new(config);
        // One line that exceeds max_chunk_bytes
        let content = "a".repeat(100);

        let chunks = chunker.chunk(&content, "test.rs");
        // Single line > max_chunk_bytes with end_line - start_line <= 10, should still produce a chunk
        // (the byte-size split only fires when end_line - start_line > 10)
        assert!(!chunks.is_empty(), "should produce at least one chunk");
    }

    #[test]
    fn test_chunk_unicode_content() {
        let config = ChunkerConfig {
            chunk_size_lines: 3,
            overlap_lines: 0,
            max_chunk_bytes: 100_000,
        };
        let chunker = CodeChunker::new(config);
        let content = "hello 🌍\nworld 🎉\nfoo ñ\nbar";

        let chunks = chunker.chunk(content, "test.rs");
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 3);
        assert_eq!(chunks[1].start_line, 4);
        assert_eq!(chunks[1].end_line, 4);
    }

    #[test]
    fn test_chunk_overlap_consistent() {
        let config = ChunkerConfig {
            chunk_size_lines: 10,
            overlap_lines: 2,
            max_chunk_bytes: 100_000,
        };
        let chunker = CodeChunker::new(config);
        let content: String = (0..50)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");

        let chunks = chunker.chunk(&content, "test.rs");
        assert!(chunks.len() > 1);

        // Verify overlap: last 2 lines of chunk[i] should be first 2 lines of chunk[i+1]
        for i in 0..chunks.len().saturating_sub(1) {
            let overlap_lines_a: Vec<&str> = chunks[i]
                .content
                .lines()
                .rev()
                .take(2)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let overlap_lines_b: Vec<&str> = chunks[i + 1].content.lines().take(2).collect();
            assert_eq!(
                overlap_lines_a,
                overlap_lines_b,
                "overlap mismatch between chunk {} and {}",
                i,
                i + 1
            );
        }
    }
}
