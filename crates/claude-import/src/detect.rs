//! Detection of Claude Code and Claude Desktop installations.

use std::path::{Path, PathBuf};

use tracing::info;

/// Result of scanning for Claude Code / Claude Desktop installations.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ClaudeDetection {
    /// Claude Code home directory (`~/.claude/`).
    pub home_dir: Option<PathBuf>,
    /// User-level settings (`~/.claude/settings.json`).
    pub user_settings_path: Option<PathBuf>,
    /// User-level local settings (`~/.claude/settings.local.json`).
    pub user_local_settings_path: Option<PathBuf>,
    /// Global user config (`~/.claude.json`) — may contain MCP servers.
    pub user_claude_json_path: Option<PathBuf>,
    /// User memory file (`~/.claude/CLAUDE.md`).
    pub user_memory_path: Option<PathBuf>,
    /// User skills directory (`~/.claude/skills/`).
    pub user_skills_dir: Option<PathBuf>,
    /// User commands directory (`~/.claude/commands/`).
    pub user_commands_dir: Option<PathBuf>,
    /// Claude Desktop config (macOS path).
    pub desktop_config_path: Option<PathBuf>,
    /// Whether anything meaningful was found.
    pub has_data: bool,
}

/// Detect Claude Code and Claude Desktop installations.
///
/// Checks `~/.claude/`, `~/.claude.json`, and the Claude Desktop config path.
pub fn detect() -> Option<ClaudeDetection> {
    let home = dirs_next::home_dir()?;
    detect_at(&home)
}

/// Detect Claude installations relative to a given home directory (for testing).
pub fn detect_at(home: &Path) -> Option<ClaudeDetection> {
    let claude_home = home.join(".claude");
    let claude_json = home.join(".claude.json");
    // macOS: ~/Library/Application Support/Claude/claude_desktop_config.json
    let desktop_config_macos = home
        .join("Library")
        .join("Application Support")
        .join("Claude")
        .join("claude_desktop_config.json");
    // Linux: ~/.config/Claude/claude_desktop_config.json
    let desktop_config_linux = home
        .join(".config")
        .join("Claude")
        .join("claude_desktop_config.json");

    let home_dir = if claude_home.is_dir() {
        Some(claude_home.clone())
    } else {
        None
    };

    let user_settings_path = check_file(&claude_home.join("settings.json"));
    let user_local_settings_path = check_file(&claude_home.join("settings.local.json"));
    let user_claude_json_path = check_file(&claude_json);
    let user_memory_path = check_file(&claude_home.join("CLAUDE.md"));
    let user_skills_dir = check_dir(&claude_home.join("skills"));
    let user_commands_dir = check_dir(&claude_home.join("commands"));
    let desktop_config_path =
        check_file(&desktop_config_macos).or_else(|| check_file(&desktop_config_linux));

    let has_data =
        home_dir.is_some() || user_claude_json_path.is_some() || desktop_config_path.is_some();

    if !has_data {
        info!("claude detect: no Claude Code or Claude Desktop installation found");
        return None;
    }

    info!(
        home_dir = ?home_dir,
        has_settings = user_settings_path.is_some(),
        has_claude_json = user_claude_json_path.is_some(),
        has_skills = user_skills_dir.is_some(),
        has_commands = user_commands_dir.is_some(),
        has_desktop = desktop_config_path.is_some(),
        "claude detect: installation found"
    );

    Some(ClaudeDetection {
        home_dir,
        user_settings_path,
        user_local_settings_path,
        user_claude_json_path,
        user_memory_path,
        user_skills_dir,
        user_commands_dir,
        desktop_config_path,
        has_data,
    })
}

fn check_file(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        Some(path.to_path_buf())
    } else {
        None
    }
}

fn check_dir(path: &Path) -> Option<PathBuf> {
    if path.is_dir() {
        Some(path.to_path_buf())
    } else {
        None
    }
}

/// Summary of what data is available for import from Claude.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ClaudeScan {
    pub mcp_servers_count: usize,
    pub skills_count: usize,
    pub commands_count: usize,
    pub has_memory: bool,
    pub has_desktop_config: bool,
    /// Items that require manual review (hooks, permissions, env vars).
    pub manual_items: Vec<String>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn detect_at_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(detect_at(tmp.path()).is_none());
    }

    #[test]
    fn detect_at_with_claude_home() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        std::fs::create_dir_all(&claude).unwrap();
        std::fs::write(claude.join("settings.json"), "{}").unwrap();

        let detection = detect_at(tmp.path()).unwrap();
        assert!(detection.home_dir.is_some());
        assert!(detection.user_settings_path.is_some());
        assert!(detection.has_data);
    }

    #[test]
    fn detect_at_with_skills_and_commands() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        std::fs::create_dir_all(claude.join("skills").join("my-skill")).unwrap();
        std::fs::create_dir_all(claude.join("commands")).unwrap();

        let detection = detect_at(tmp.path()).unwrap();
        assert!(detection.user_skills_dir.is_some());
        assert!(detection.user_commands_dir.is_some());
    }

    #[test]
    fn detect_at_with_claude_json() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".claude.json"), r#"{"mcpServers":{}}"#).unwrap();

        let detection = detect_at(tmp.path()).unwrap();
        assert!(detection.user_claude_json_path.is_some());
        assert!(detection.has_data);
    }
}
