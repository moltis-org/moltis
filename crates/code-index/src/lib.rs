//! Codebase indexing for moltis.
//!
//! Indexes git-tracked source files per project with pluggable backends
//! (QMD default, pgvector future). Reuses existing embedding, chunking,
//! and search infrastructure from `moltis-memory` and `moltis-qmd`.

pub mod config;
pub mod discover;
pub mod error;
pub mod filter;
pub mod types;

// Optional backends, gated behind feature flags.
#[cfg(feature = "qmd")]
pub mod backend_qmd;

// Re-export primary types.
pub use config::CodeIndexConfig;
pub use error::{Error, Result};
pub use types::{Backend, CodeChunk, FileEntry, FilteredFile, IndexStatus, Language};
