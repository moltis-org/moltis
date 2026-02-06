//! Skills system: discovery, parsing, registry, and installation.
//!
//! Skills are directories containing a `SKILL.md` file with YAML frontmatter
//! and markdown instructions, following the Agent Skills open standard.

pub mod discover;
pub mod formats;
pub mod install;
pub mod manifest;
pub mod parse;
pub mod prompt_gen;
pub mod registry;
pub mod requirements;
pub mod types;
#[cfg(feature = "file-watcher")]
pub mod watcher;
