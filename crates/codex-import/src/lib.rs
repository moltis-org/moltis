//! Import data from OpenAI Codex CLI into Moltis.
//!
//! Supports detection, scanning, and selective import of:
//! - MCP server configurations (from `~/.codex/config.toml`)
//! - Memory / instructions (`~/.codex/instructions.md`)

pub mod detect;
pub mod mcp_servers;
pub mod memory;
