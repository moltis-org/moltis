//! Import data from a Hermes installation into Moltis.
//!
//! Supports detection, scanning, and selective import of:
//! - Credentials from `.env` (API keys mapped to providers)
//! - Skills (`~/.hermes/skills/` with SKILL.md format)
//! - Memory/workspace files (SOUL.md, AGENTS.md, MEMORY.md, USER.md)
//! - Provider configurations from `config.yaml`

pub mod credentials;
pub mod detect;
pub mod memory;
pub mod skills;
