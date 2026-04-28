//! Import data from Claude Code and Claude Desktop into Moltis.
//!
//! Supports detection, scanning, and selective import of:
//! - MCP server configurations (from `~/.claude.json`, project `.mcp.json`, Claude Desktop)
//! - Skills (`~/.claude/skills/` with SKILL.md format)
//! - Commands converted to skills (`~/.claude/commands/`)
//! - Memory files (`CLAUDE.md`)
//! - Manual review items (hooks, permissions, env vars)

pub mod detect;
pub mod mcp_servers;
pub mod memory;
pub mod skills;
