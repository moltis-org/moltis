//! Codebase indexing for moltis.
//!
//! Indexes git-tracked source files per project with pluggable backends
//! (QMD default, pgvector future). Reuses existing embedding, chunking,
//! and search infrastructure from `moltis-memory` and `moltis-qmd`.

pub mod config;
pub mod delta;
pub mod discover;
pub mod error;
pub mod filter;
pub mod snapshot_store;
pub mod types;

// Optional backends, gated behind feature flags.
#[cfg(feature = "qmd")]
pub mod backend_qmd;

// Search result adapter.
#[cfg(feature = "qmd")]
pub mod search;

// Agent tools (behind qmd feature since they require CodeIndex with backend).
#[cfg(feature = "qmd")]
pub mod tools;

// Index orchestrator.
pub mod index;

// File watcher for incremental reindexing.
#[cfg(feature = "file-watcher")]
pub mod watcher;

// Re-export primary types.
pub use config::CodeIndexConfig;
pub use delta::SyncDelta;
pub use delta::HashSnapshot;
pub use error::{Error, Result};
pub use index::CodeIndex;
pub use snapshot_store::SnapshotStore;
pub use types::{CodeChunk, FileEntry, FilteredFile, IndexStatus, Language, SearchResult};