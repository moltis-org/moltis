//! `Read` tool — typed, line-numbered file reads.
//!
//! Matches Claude Code's `Read` tool schema: `file_path`, optional `offset`
//! and `limit`. Returns a structured payload with `content` (cat -n style),
//! `total_lines`, and `truncated` flags so the LLM can tell whether it has
//! the full file.
//!
//! Format-specific dispatchers live in submodules so new formats (e.g.
//! `.ipynb`, `.docx`) can be added without growing this file.

pub(crate) mod image;
pub(crate) mod pdf;

mod tool;

pub use tool::*;
