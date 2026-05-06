//! QMD memory backend for moltis.
//!
//! This crate provides an alternative memory backend that uses the QMD sidecar process
//! for hybrid search (BM25 + vector + LLM reranking).
//!
//! QMD must be installed separately. See: https://github.com/tobi/qmd

pub mod error;
mod manager;
mod runtime;

pub use {
    error::Error,
    manager::{QmdCollection, QmdManager, QmdManagerConfig, QmdSearchResult, SearchMode},
    runtime::QmdMemoryRuntime,
};
