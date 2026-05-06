//! Shared MCP server import utilities.
//!
//! Provides a common merge function used by all import sources to write
//! MCP server entries into Moltis's `mcp-servers.json`.

use std::{collections::HashMap, path::Path};

use {
    serde::{Deserialize, Serialize},
    tracing::debug,
};

use crate::report::{CategoryReport, ImportCategory, ImportStatus};

/// A source-agnostic MCP server entry for import.
///
/// This is the common denominator across Claude Code, OpenClaw, and Hermes
/// MCP server formats. Fields are optional except `command` (for stdio) or
/// `url` (for SSE/HTTP transports).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportMcpServer {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Transport type string: "stdio", "sse", or "streamable-http".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    /// URL for SSE/HTTP transports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Custom headers for remote transports.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
}

fn default_true() -> bool {
    true
}

/// Merge a set of MCP servers into Moltis's `mcp-servers.json`.
///
/// Skips servers whose name already exists in the destination file.
/// Creates parent directories and the file if they don't exist.
pub fn merge_mcp_servers(
    servers: &HashMap<String, ImportMcpServer>,
    dest_path: &Path,
) -> CategoryReport {
    if servers.is_empty() {
        return CategoryReport::skipped(ImportCategory::McpServers);
    }

    let mut existing: HashMap<String, serde_json::Value> = if dest_path.is_file() {
        match std::fs::read_to_string(dest_path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(map) => map,
                Err(e) => {
                    return CategoryReport::failed(
                        ImportCategory::McpServers,
                        format!("existing mcp-servers.json is malformed: {e}"),
                    );
                },
            },
            Err(_) => HashMap::new(),
        }
    } else {
        HashMap::new()
    };

    let mut imported = 0;
    let mut skipped = 0;

    for (name, server) in servers {
        if existing.contains_key(name) {
            debug!(name, "MCP server already exists, skipping");
            skipped += 1;
            continue;
        }

        debug!(name, command = %server.command, "importing MCP server");
        let value = serde_json::to_value(server).unwrap_or_default();
        existing.insert(name.clone(), value);
        imported += 1;
    }

    if imported > 0 {
        if let Some(parent) = dest_path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            return CategoryReport::failed(
                ImportCategory::McpServers,
                format!("failed to create directory: {e}"),
            );
        }
        let json = match serde_json::to_string_pretty(&existing) {
            Ok(j) => j,
            Err(e) => {
                return CategoryReport::failed(
                    ImportCategory::McpServers,
                    format!("failed to serialize MCP servers: {e}"),
                );
            },
        };
        if let Err(e) = std::fs::write(dest_path, json) {
            return CategoryReport::failed(
                ImportCategory::McpServers,
                format!("failed to write mcp-servers.json: {e}"),
            );
        }
    }

    let status = if imported == 0 {
        ImportStatus::Skipped
    } else {
        ImportStatus::Success
    };

    CategoryReport {
        category: ImportCategory::McpServers,
        status,
        items_imported: imported,
        items_updated: 0,
        items_skipped: skipped,
        warnings: Vec::new(),
        errors: Vec::new(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn merge_into_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("mcp-servers.json");

        let mut servers = HashMap::new();
        servers.insert("test".to_string(), ImportMcpServer {
            command: "test-server".to_string(),
            args: vec!["--port".to_string(), "3000".to_string()],
            ..Default::default()
        });

        let report = merge_mcp_servers(&servers, &dest);
        assert_eq!(report.status, ImportStatus::Success);
        assert_eq!(report.items_imported, 1);
        assert!(dest.is_file());
    }

    #[test]
    fn merge_skips_duplicates() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("mcp-servers.json");

        std::fs::write(
            &dest,
            r#"{"existing":{"command":"old","args":[],"env":{},"enabled":true}}"#,
        )
        .unwrap();

        let mut servers = HashMap::new();
        servers.insert("existing".to_string(), ImportMcpServer {
            command: "new".to_string(),
            ..Default::default()
        });

        let report = merge_mcp_servers(&servers, &dest);
        assert_eq!(report.items_imported, 0);
        assert_eq!(report.items_skipped, 1);
    }

    #[test]
    fn merge_adds_new_preserves_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("mcp-servers.json");

        std::fs::write(
            &dest,
            r#"{"old":{"command":"old","args":[],"env":{},"enabled":true}}"#,
        )
        .unwrap();

        let mut servers = HashMap::new();
        servers.insert("new".to_string(), ImportMcpServer {
            command: "new-server".to_string(),
            ..Default::default()
        });

        let report = merge_mcp_servers(&servers, &dest);
        assert_eq!(report.items_imported, 1);

        let content = std::fs::read_to_string(&dest).unwrap();
        let loaded: HashMap<String, serde_json::Value> = serde_json::from_str(&content).unwrap();
        assert!(loaded.contains_key("old"));
        assert!(loaded.contains_key("new"));
    }

    #[test]
    fn empty_servers_returns_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("mcp-servers.json");
        let report = merge_mcp_servers(&HashMap::new(), &dest);
        assert_eq!(report.status, ImportStatus::Skipped);
    }

    #[test]
    fn malformed_existing_file_returns_failed() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("mcp-servers.json");
        std::fs::write(&dest, "not valid json {{{").unwrap();

        let mut servers = HashMap::new();
        servers.insert("new".to_string(), ImportMcpServer {
            command: "new-server".to_string(),
            ..Default::default()
        });

        let report = merge_mcp_servers(&servers, &dest);
        assert_eq!(report.status, ImportStatus::Failed);
        assert!(!report.errors.is_empty());
        // Original file should not be overwritten
        let content = std::fs::read_to_string(&dest).unwrap();
        assert_eq!(content, "not valid json {{{");
    }
}
