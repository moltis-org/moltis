//! Text chunking for memory — re-exports from moltis-splitter.

// Re-export the public API so existing consumers of `moltis_memory::chunker` continue to work.
pub use moltis_splitter::{Chunk, chunk_content, chunk_markdown};
