//! Memory management: markdown files → chunked → embedded → hybrid search in SQLite.

pub mod chunker;
pub mod config;
#[cfg(test)]
pub mod contract;
pub mod embeddings;
pub mod embeddings_batch;
pub mod embeddings_fallback;
#[cfg(feature = "local-embeddings")]
#[allow(unsafe_code)] // FFI wrappers for llama-cpp-2 require unsafe Send/Sync impls.
pub mod embeddings_local;
pub mod embeddings_openai;
pub mod error;
pub mod manager;
pub mod reranking;
pub mod runtime;
pub mod schema;
pub mod search;
pub mod session_export;
pub(crate) mod splitter;
pub mod store;
pub mod store_sqlite;
pub mod tools;
#[cfg(feature = "file-watcher")]
pub mod watcher;
pub mod writer;

// Re-export run_migrations for consistency with other crates.
pub use {
    error::{Error, Result},
    schema::run_migrations,
};
