//! Import MCP server configuration from OpenClaw.
//!
//! Merges OpenClaw's `mcp-servers.json` into Moltis's MCP registry,
//! skipping servers with duplicate names.

use std::{collections::HashMap, path::Path};

use tracing::debug;

use crate::{
    detect::OpenClawDetection,
    report::{CategoryReport, ImportCategory, ImportStatus},
    types::OpenClawMcpServer,
};

/// Import MCP servers from OpenClaw into the Moltis MCP registry.
///
/// `dest_mcp_path` is the path to Moltis's `mcp-servers.json`.
pub fn import_mcp_servers(detection: &OpenClawDetection, dest_mcp_path: &Path) -> CategoryReport {
    let src_path = detection.home_dir.join("mcp-servers.json");
    if !src_path.is_file() {
        return CategoryReport::skipped(ImportCategory::McpServers);
    }

    let src_servers = match load_mcp_servers(&src_path) {
        Ok(s) => s,
        Err(e) => {
            return CategoryReport::failed(
                ImportCategory::McpServers,
                format!("failed to parse OpenClaw mcp-servers.json: {e}"),
            );
        },
    };

    if src_servers.is_empty() {
        return CategoryReport::skipped(ImportCategory::McpServers);
    }

    // Load existing Moltis MCP servers
    let mut existing = if dest_mcp_path.is_file() {
        load_mcp_servers(dest_mcp_path).unwrap_or_default()
    } else {
        HashMap::new()
    };

    let mut imported = 0;
    let mut skipped = 0;

    for (name, server) in &src_servers {
        if existing.contains_key(name) {
            debug!(name, "MCP server already exists, skipping");
            skipped += 1;
            continue;
        }

        debug!(name, command = %server.command, "importing MCP server");
        existing.insert(name.clone(), server.clone());
        imported += 1;
    }

    if imported > 0 {
        if let Some(parent) = dest_mcp_path.parent()
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
        if let Err(e) = std::fs::write(dest_mcp_path, json) {
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

fn load_mcp_servers(path: &Path) -> crate::error::Result<HashMap<String, OpenClawMcpServer>> {
    let content = std::fs::read_to_string(path)?;
    let servers: HashMap<String, OpenClawMcpServer> = serde_json::from_str(&content)?;
    Ok(servers)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn make_detection(home: &Path) -> OpenClawDetection {
        OpenClawDetection {
            home_dir: home.to_path_buf(),
            has_config: false,
            has_credentials: false,
            has_mcp_servers: true,
            workspace_dir: home.join("workspace"),
            has_memory: false,
            has_skills: false,
            agent_ids: Vec::new(),
            session_count: 0,
            unsupported_channels: Vec::new(),
            has_workspace_files: false,
            workspace_files_found: Vec::new(),
        }
    }

    #[test]
    fn import_new_mcp_servers() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest = tmp.path().join("moltis").join("mcp-servers.json");

        std::fs::write(
            home.join("mcp-servers.json"),
            r#"{"my-server":{"command":"my-server","args":["--port","3000"],"env":{},"enabled":true}}"#,
        )
        .unwrap();

        let detection = make_detection(home);
        let report = import_mcp_servers(&detection, &dest);

        assert_eq!(report.status, ImportStatus::Success);
        assert_eq!(report.items_imported, 1);
        assert!(dest.is_file());

        let content = std::fs::read_to_string(&dest).unwrap();
        let loaded: HashMap<String, OpenClawMcpServer> = serde_json::from_str(&content).unwrap();
        assert!(loaded.contains_key("my-server"));
    }

    #[test]
    fn import_merges_with_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest_dir = tmp.path().join("moltis");
        std::fs::create_dir_all(&dest_dir).unwrap();
        let dest = dest_dir.join("mcp-servers.json");

        // Existing Moltis servers
        std::fs::write(
            &dest,
            r#"{"existing-server":{"command":"existing","args":[],"env":{},"enabled":true}}"#,
        )
        .unwrap();

        // OpenClaw servers
        std::fs::write(
            home.join("mcp-servers.json"),
            r#"{"new-server":{"command":"new","args":[],"env":{},"enabled":true}}"#,
        )
        .unwrap();

        let detection = make_detection(home);
        let report = import_mcp_servers(&detection, &dest);

        assert_eq!(report.items_imported, 1);

        let content = std::fs::read_to_string(&dest).unwrap();
        let loaded: HashMap<String, OpenClawMcpServer> = serde_json::from_str(&content).unwrap();
        assert!(loaded.contains_key("existing-server"));
        assert!(loaded.contains_key("new-server"));
    }

    #[test]
    fn import_skips_duplicates() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dest_dir = tmp.path().join("moltis");
        std::fs::create_dir_all(&dest_dir).unwrap();
        let dest = dest_dir.join("mcp-servers.json");

        std::fs::write(
            &dest,
            r#"{"same-name":{"command":"existing","args":[],"env":{},"enabled":true}}"#,
        )
        .unwrap();

        std::fs::write(
            home.join("mcp-servers.json"),
            r#"{"same-name":{"command":"different","args":[],"env":{},"enabled":true}}"#,
        )
        .unwrap();

        let detection = make_detection(home);
        let report = import_mcp_servers(&detection, &dest);

        assert_eq!(report.items_imported, 0);
        assert_eq!(report.items_skipped, 1);
    }

    #[test]
    fn no_mcp_file_returns_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let detection = make_detection(tmp.path());
        let report = import_mcp_servers(&detection, &tmp.path().join("dest.json"));
        assert_eq!(report.status, ImportStatus::Skipped);
    }
}
