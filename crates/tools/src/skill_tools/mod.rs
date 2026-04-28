//! Agent tools for creating, updating, and deleting personal skills at runtime.
//! Skills are written to `<data_dir>/skills/<name>/SKILL.md` (Personal source).

mod crud;
mod helpers;
mod read_ops;
mod write_ops;

pub use {
    crud::{CreateSkillTool, DeleteSkillTool, UpdateSkillTool},
    read_ops::ReadSkillTool,
    write_ops::{PatchSkillTool, WriteSkillFilesTool},
};

const MAX_SIDECAR_FILES_PER_CALL: usize = 32;
/// Per-sidecar-subdirectory cap used by the read path's listing. Enforcing a
/// per-subdir quota guarantees every populated subdirectory shows up in the
/// listing.
const MAX_SIDECAR_FILES_PER_SUBDIR: usize = 8;
const MAX_SIDECAR_FILE_BYTES: usize = 128 * 1024;
const MAX_SIDECAR_TOTAL_BYTES: usize = 512 * 1024;

/// Cap on the size of a single skill body (SKILL.md or a plugin's `.md` file).
const MAX_SKILL_BODY_BYTES: usize = 256 * 1024;

// Re-export internal helpers for test modules.
#[cfg(test)]
pub(crate) use helpers::{split_frontmatter_body, update_frontmatter_description};

#[cfg(test)]
mod tests;
