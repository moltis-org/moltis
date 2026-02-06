//! QMD memory backend for moltis.
//!
//! This crate provides an alternative memory backend that uses the QMD sidecar process
//! for hybrid search (BM25 + vector + LLM reranking).
//!
//! QMD must be installed separately. See: https://github.com/qmd/qmd

mod manager;
mod store;

pub use {
    manager::{QmdManager, QmdManagerConfig},
    store::QmdStore,
};
