//! Native filesystem tools.
//!
//! Provides `file_read` and `file_info` as built-in replacements for the
//! MCP filesystem server's read/info tools. Additional tools (write, edit,
//! tree, list, search, create_dir, move) will follow in later phases.

pub mod info;
pub mod read;
