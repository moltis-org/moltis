//! Skills system: discovery, parsing, registry, and installation.
//!
//! Skills are directories containing a `SKILL.md` file with YAML frontmatter
//! and markdown instructions, following the Agent Skills open standard.

pub mod discover;
pub mod formats;
pub mod install;
pub mod manifest;
pub mod migration;
pub mod parse;
pub mod portability;
pub mod prompt_gen;
pub mod registry;
pub mod requirements;
pub mod safety;
pub mod types;

/// Canonical list of sidecar subdirectories a skill directory may contain,
/// matching the agentskills.io standard. Both the prompt generator
/// (`prompt_gen.rs`) and the read-side tool (`moltis_tools::skill_tools`)
/// use this constant, so adding a new subdirectory here automatically
/// propagates to the activation instruction and the listing walker — no
/// silent drift between what the prompt advertises and what the tool
/// actually walks.
pub const SIDECAR_SUBDIRS: &[&str] = &["references", "templates", "assets", "scripts"];
#[cfg(feature = "file-watcher")]
pub mod watcher;
