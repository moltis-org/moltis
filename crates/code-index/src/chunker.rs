//! Code chunking for embedding.
//!
//! Uses AST-aware splitting via `moltis-splitter` when a tree-sitter grammar is available
//! for the file extension, falling back to line-based splitting otherwise.

use {crate::store::CodeChunk, moltis_splitter::Chunk};

/// Configuration for code chunking.
#[derive(Debug, Clone)]
pub struct ChunkerConfig {
    /// Target chunk size in words.
    pub chunk_size: usize,
    /// Overlap between chunks in words.
    pub chunk_overlap: usize,
}

impl Default for ChunkerConfig {
    fn default() -> Self {
        Self {
            chunk_size: 200,
            chunk_overlap: 40,
        }
    }
}

/// Chunk file content into pieces using AST-aware splitting.
///
/// Returns chunks with 1-indexed line numbers.
pub fn chunk(
    content: &str,
    file_path: &str,
    extension: &str,
    config: &ChunkerConfig,
) -> Vec<CodeChunk> {
    let splitter_chunks =
        moltis_splitter::chunk_content(content, config.chunk_size, config.chunk_overlap, extension);

    splitter_chunks
        .into_iter()
        .enumerate()
        .map(|(idx, chunk)| map_chunk(chunk, idx, file_path))
        .collect()
}

fn map_chunk(chunk: Chunk, chunk_index: usize, file_path: &str) -> CodeChunk {
    CodeChunk {
        file_path: file_path.to_string(),
        chunk_index,
        content: chunk.text,
        embedding: None,
        start_line: chunk.start_line,
        end_line: chunk.end_line,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> ChunkerConfig {
        ChunkerConfig::default()
    }

    #[test]
    fn test_chunk_empty() {
        let chunks = chunk("", "test.rs", "rs", &default_config());
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_chunk_small_file() {
        let content = "line1\nline2\nline3";
        let chunks = chunk(content, "test.rs", "rs", &default_config());

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 3);
        assert_eq!(chunks[0].content, content);
    }

    #[test]
    fn test_chunk_indices_sequential() {
        let config = ChunkerConfig {
            chunk_size: 5,
            chunk_overlap: 0,
        };
        let content: String = (0..20)
            .map(|i| format!("line{}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let chunks = chunk(&content, "test.rs", "rs", &config);

        for (idx, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.chunk_index, idx);
        }
    }

    #[test]
    fn test_chunk_unknown_extension_falls_back() {
        let content = "line1\nline2\nline3";
        let chunks = chunk(content, "test.xyz", "xyz", &default_config());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start_line, 1);
    }

    #[test]
    fn test_chunk_unicode_content() {
        let config = ChunkerConfig {
            chunk_size: 3,
            chunk_overlap: 0,
        };
        let content = "hello 🌍\nworld 🎉\nfoo ñ\nbar";
        let chunks = chunk(content, "test.rs", "rs", &config);
        assert!(!chunks.is_empty());
    }
}
