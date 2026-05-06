//! Import MCP server configurations from Claude Code and Claude Desktop.
//!
//! Collects MCP servers from three sources, deduplicates, and merges into
//! Moltis's `mcp-servers.json`.
//!
//! Sources:
//! 1. `~/.claude.json` → `mcpServers` (user-level)
//! 2. `~/Library/Application Support/Claude/claude_desktop_config.json` → `mcpServers`
//! 3. Project-level `.mcp.json` → `mcpServers` (if provided)

use std::{collections::HashMap, path::Path};

use {
    moltis_import_core::{
        mcp::{ImportMcpServer, merge_mcp_servers},
        report::CategoryReport,
    },
    tracing::debug,
};

use crate::detect::ClaudeDetection;

/// Import MCP servers from all Claude sources into Moltis.
pub fn import_mcp_servers(detection: &ClaudeDetection, dest_path: &Path) -> CategoryReport {
    let mut all_servers: HashMap<String, ImportMcpServer> = HashMap::new();

    // Source 1: ~/.claude.json mcpServers
    if let Some(ref path) = detection.user_claude_json_path
        && let Some(servers) = extract_mcp_from_claude_json(path)
    {
        for (name, server) in servers {
            all_servers.entry(name).or_insert(server);
        }
    }

    // Source 2: Claude Desktop config
    if let Some(ref path) = detection.desktop_config_path
        && let Some(servers) = extract_mcp_from_claude_json(path)
    {
        for (name, server) in servers {
            all_servers.entry(name).or_insert(server);
        }
    }

    merge_mcp_servers(&all_servers, dest_path)
}

/// Extract MCP servers from a JSON config file (works for both `.claude.json`
/// and Claude Desktop's `claude_desktop_config.json`).
///
/// Looks for `mcpServers` at the top level. Each server entry may have:
/// `command`, `args`, `env`, `cwd`, `url`, `type`/`transport`, `headers`.
fn extract_mcp_from_claude_json(path: &Path) -> Option<HashMap<String, ImportMcpServer>> {
    let content = std::fs::read_to_string(path).ok()?;
    let root: serde_json::Value = serde_json::from_str(&content).ok()?;

    let mcp_servers = root.get("mcpServers").and_then(|v| v.as_object())?;
    let mut result = HashMap::new();

    for (name, value) in mcp_servers {
        let name = name.trim().to_string();
        if name.is_empty() {
            continue;
        }
        let Some(obj) = value.as_object() else {
            continue;
        };

        let command = obj
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        let args: Vec<String> = obj
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let env: HashMap<String, String> = obj
            .get("env")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        // Transport: check "type" or "transport" field
        let transport = obj
            .get("type")
            .or_else(|| obj.get("transport"))
            .and_then(|v| v.as_str())
            .map(String::from);

        let url = obj.get("url").and_then(|v| v.as_str()).map(String::from);

        let headers: HashMap<String, String> = obj
            .get("headers")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        debug!(name, command = %command, "found Claude MCP server");

        result.insert(name, ImportMcpServer {
            command,
            args,
            env,
            enabled: true,
            transport,
            url,
            headers,
        });
    }

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn make_detection() -> ClaudeDetection {
        ClaudeDetection {
            home_dir: None,
            user_settings_path: None,
            user_local_settings_path: None,
            user_claude_json_path: None,
            user_memory_path: None,
            user_skills_dir: None,
            user_commands_dir: None,
            desktop_config_path: None,
            has_data: false,
        }
    }

    #[test]
    fn extract_from_claude_json() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(".claude.json");
        std::fs::write(
            &path,
            r#"{
                "mcpServers": {
                    "my-server": {
                        "command": "npx",
                        "args": ["-y", "@my/mcp-server"],
                        "env": {"API_KEY": "test-123"}
                    },
                    "remote": {
                        "type": "sse",
                        "url": "https://example.com/mcp"
                    }
                }
            }"#,
        )
        .unwrap();

        let servers = extract_mcp_from_claude_json(&path).unwrap();
        assert_eq!(servers.len(), 2);

        let my_server = &servers["my-server"];
        assert_eq!(my_server.command, "npx");
        assert_eq!(my_server.args, vec!["-y", "@my/mcp-server"]);
        assert_eq!(my_server.env.get("API_KEY").unwrap(), "test-123");

        let remote = &servers["remote"];
        assert_eq!(remote.transport.as_deref(), Some("sse"));
        assert_eq!(remote.url.as_deref(), Some("https://example.com/mcp"));
    }

    #[test]
    fn import_from_claude_json_and_desktop() {
        let tmp = tempfile::tempdir().unwrap();

        let claude_json = tmp.path().join(".claude.json");
        std::fs::write(
            &claude_json,
            r#"{"mcpServers":{"server-a":{"command":"a","args":[]}}}"#,
        )
        .unwrap();

        let desktop_config = tmp.path().join("desktop.json");
        std::fs::write(
            &desktop_config,
            r#"{"mcpServers":{"server-b":{"command":"b","args":[]}}}"#,
        )
        .unwrap();

        let dest = tmp.path().join("mcp-servers.json");
        let mut detection = make_detection();
        detection.user_claude_json_path = Some(claude_json);
        detection.desktop_config_path = Some(desktop_config);

        let report = import_mcp_servers(&detection, &dest);
        assert_eq!(report.items_imported, 2);

        let content = std::fs::read_to_string(&dest).unwrap();
        let loaded: HashMap<String, serde_json::Value> = serde_json::from_str(&content).unwrap();
        assert!(loaded.contains_key("server-a"));
        assert!(loaded.contains_key("server-b"));
    }

    #[test]
    fn deduplicates_across_sources() {
        let tmp = tempfile::tempdir().unwrap();

        let claude_json = tmp.path().join(".claude.json");
        std::fs::write(
            &claude_json,
            r#"{"mcpServers":{"same-name":{"command":"from-claude-json","args":[]}}}"#,
        )
        .unwrap();

        let desktop_config = tmp.path().join("desktop.json");
        std::fs::write(
            &desktop_config,
            r#"{"mcpServers":{"same-name":{"command":"from-desktop","args":[]}}}"#,
        )
        .unwrap();

        let dest = tmp.path().join("mcp-servers.json");
        let mut detection = make_detection();
        detection.user_claude_json_path = Some(claude_json);
        detection.desktop_config_path = Some(desktop_config);

        let report = import_mcp_servers(&detection, &dest);
        // First source wins, second is deduplicated internally
        assert_eq!(report.items_imported, 1);
    }

    #[test]
    fn no_sources_returns_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("mcp-servers.json");
        let detection = make_detection();
        let report = import_mcp_servers(&detection, &dest);
        assert_eq!(
            report.status,
            moltis_import_core::report::ImportStatus::Skipped
        );
    }
}
