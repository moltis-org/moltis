//! Native filesystem tools: `Read`, `Write`, `Edit`, `MultiEdit`, `Glob`, `Grep`.
//!
//! These are the structured, typed alternative to shell-based file I/O via
//! `exec`. They match Claude Code's tool schemas exactly so LLMs trained on
//! those tools encounter the same shape of parameters and responses.
//!
//! See GH moltis-org/moltis#657 for context.
//!
//! Phase 1 (this module) covers host-path execution only. Sandbox routing
//! arrives in phase 2, UX polish (adaptive paging, edit recovery, re-read
//! detection) in phase 3, and operator-facing `[tools.fs]` config in phase 4.

pub mod edit;
pub mod glob;
pub mod grep;
pub mod multi_edit;
pub mod read;
pub mod sandbox_bridge;
pub mod shared;
pub mod write;

mod context;

pub use {
    context::*,
    edit::EditTool,
    glob::GlobTool,
    grep::GrepTool,
    multi_edit::MultiEditTool,
    read::ReadTool,
    shared::{BinaryPolicy, FsPathPolicy, FsState, new_fs_state},
    write::WriteTool,
};

#[cfg(test)]
mod contract_tests;
