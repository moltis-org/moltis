//! Text and code splitting for moltis.
//!
//! Provides [`Chunk`] for split results and [`chunk_content`] as the primary entry point.
//! When the `code-splitter-base` feature is enabled and a tree-sitter grammar is available
//! for the file extension, content is split at AST boundaries (functions, classes, structs).
//! Otherwise, falls back to line-based [`chunk_markdown`].

mod code;
mod markdown;

pub use markdown::{Chunk, chunk_markdown};

/// Average characters per word, used to convert word-count config to character count.
const CHARS_PER_WORD: usize = 5;

/// Split content into chunks, using tree-sitter when a grammar is available for `extension`.
///
/// `chunk_size` and `chunk_overlap` are in **words** (matching existing config semantics).
/// For tree-sitter splitting these are converted to approximate character counts.
pub fn chunk_content(
    text: &str,
    chunk_size: usize,
    chunk_overlap: usize,
    extension: &str,
) -> Vec<Chunk> {
    #[cfg(feature = "code-splitter-base")]
    {
        if let Some(chunks) = code::try_code_split(text, chunk_size, chunk_overlap, extension) {
            return chunks;
        }
    }

    // Suppress unused-variable warning when feature is disabled.
    let _ = extension;

    chunk_markdown(text, chunk_size, chunk_overlap)
}
