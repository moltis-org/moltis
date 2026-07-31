//! Import MCP server configuration from OpenClaw.
//!
//! Merges OpenClaw's `mcp-servers.json` into Moltis's MCP registry,
//! skipping servers with duplicate names.

use std::path::Path;

use moltis_import_core::mcp::{ImportMcpServer, merge_mcp_servers};

use crate::{
    detect::OpenClawDetection,
    report::{CategoryReport, ImportCategory, ImportStatus},
};

/// Import MCP servers from OpenClaw into the Moltis MCP registry.
///
/// `dest_mcp_path` is the path to Moltis's `mcp-servers.json`.
pub fn import_mcp_servers(detection: &OpenClawDetection, dest_mcp_path: &Path) -> CategoryReport {
    let src_servers = match collect_mcp_servers(detection) {
        Ok(servers) => servers,
        Err(error) => {
            return CategoryReport::failed(
                ImportCategory::McpServers,
                format!("failed to parse OpenClaw mcp-servers.json: {error}"),
            );
        },
    };
    if src_servers.is_empty() {
        return CategoryReport::skipped(ImportCategory::McpServers);
    }
    let report = merge_mcp_servers(&src_servers, dest_mcp_path);
    CategoryReport {
        category: ImportCategory::McpServers,
        status: match report.status {
            moltis_import_core::report::ImportStatus::Success => ImportStatus::Success,
            moltis_import_core::report::ImportStatus::Partial => ImportStatus::Partial,
            moltis_import_core::report::ImportStatus::Skipped => ImportStatus::Skipped,
            moltis_import_core::report::ImportStatus::Failed => ImportStatus::Failed,
        },
        items_imported: report.items_imported,
        items_updated: report.items_updated,
        items_skipped: report.items_skipped,
        warnings: report.warnings,
        errors: report.errors,
    }
}

/// Collect OpenClaw MCP servers without writing them.
pub fn collect_mcp_servers(
    detection: &OpenClawDetection,
) -> crate::error::Result<std::collections::HashMap<String, ImportMcpServer>> {
    let src_path = detection.home_dir.join("mcp-servers.json");
    if !src_path.is_file() {
        return Ok(std::collections::HashMap::new());
    }
    load_mcp_servers(&src_path)
}

fn load_mcp_servers(
    path: &Path,
) -> crate::error::Result<std::collections::HashMap<String, ImportMcpServer>> {
    let content = std::fs::read_to_string(path)?;
    let servers = serde_json::from_str(&content)?;
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
        let loaded: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(loaded["servers"].get("my-server").is_some());
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
        let loaded: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(loaded["servers"].get("existing-server").is_some());
        assert!(loaded["servers"].get("new-server").is_some());
    }

    #[test]
    fn import_preserves_managed_registry_state() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        std::fs::write(
            home.join("mcp-servers.json"),
            r#"{"imported":{"command":"new","args":[],"env":{},"enabled":true}}"#,
        )
        .unwrap();
        let dest = tmp.path().join("moltis").join("mcp-servers.json");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        let existing = serde_json::json!({
            "servers": {
                "managed": {
                    "command": "managed",
                    "managed_origin": {
                        "approval": {"commit": "abc", "config_digest": "digest"}
                    }
                }
            },
            "repositories": {"repo-1": {"alias": "managed-tools"}},
            "future_registry_field": {"version": 2}
        });
        std::fs::write(&dest, serde_json::to_string(&existing).unwrap()).unwrap();

        let report = import_mcp_servers(&make_detection(home), &dest);

        assert_eq!(report.status, ImportStatus::Success);
        let loaded: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&dest).unwrap()).unwrap();
        assert_eq!(loaded["repositories"], existing["repositories"]);
        assert_eq!(
            loaded["future_registry_field"],
            existing["future_registry_field"]
        );
        assert_eq!(
            loaded["servers"]["managed"]["managed_origin"]["approval"],
            existing["servers"]["managed"]["managed_origin"]["approval"]
        );
        assert_eq!(loaded["servers"]["imported"]["command"], "new");
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
