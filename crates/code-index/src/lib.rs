//! Code-index crate for Moltis — workspace codebase intelligence.
//!
//! Supports multiple backends:
//! - **QMD** (optional, `qmd` feature): External QMD binary for hybrid search.
//! - **Builtin** (optional, `builtin` feature): SQLite + FTS5 with embeddings.
//! - **Config-only**: File discovery and filtering without search.

// Core modules (always available).
pub mod chunker;
pub mod config;
pub mod delta;
pub mod discover;
pub mod error;
pub mod filter;
pub mod index;
#[cfg(feature = "tracing")]
pub mod log;
pub mod snapshot_store;
pub mod store;
pub mod types;

// Optional backends, gated behind feature flags.
#[cfg(feature = "qmd")]
pub mod backend_qmd;

#[cfg(feature = "builtin")]
pub mod store_sqlite;

#[cfg(feature = "file-watcher")]
pub mod watcher;

// Search result adapter (only relevant with QMD).
#[cfg(feature = "qmd")]
pub mod search;

// Agent tools (only relevant with search backend).
#[cfg(any(feature = "qmd", feature = "builtin"))]
pub mod tools;

// Re-exports for convenience.
pub use {
    config::CodeIndexConfig,
    delta::{FileMeta, HashSnapshot},
    error::{Error, Result},
    index::CodeIndex,
    types::{IndexStatus, SearchResult},
};

/// Utility function to sanitize a project ID for use as a QMD collection name.
pub fn sanitize_project_id(project_id: &str) -> String {
    project_id
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_project_id() {
        assert_eq!(sanitize_project_id("my/project"), "my_project");
        assert_eq!(sanitize_project_id("hello world"), "hello_world");
        assert_eq!(sanitize_project_id("foo-bar_baz"), "foo-bar_baz");
    }
}
