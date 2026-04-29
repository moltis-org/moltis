//! Import MCP server configurations from Codex CLI.
//!
//! Reads MCP servers from `~/.codex/config.toml` under `[mcp_servers]`
//! and merges them into Moltis's `mcp-servers.json`.

use std::{collections::HashMap, path::Path};

use {
    moltis_import_core::{
        mcp::{ImportMcpServer, merge_mcp_servers},
        report::CategoryReport,
    },
    tracing::debug,
};

use crate::detect::CodexDetection;

/// Import MCP servers from Codex config into Moltis.
pub fn import_mcp_servers(detection: &CodexDetection, dest_path: &Path) -> CategoryReport {
    let Some(ref config_path) = detection.config_path else {
        return CategoryReport::skipped(moltis_import_core::report::ImportCategory::McpServers);
    };

    let servers = match extract_mcp_from_config(config_path) {
        Some(s) if !s.is_empty() => s,
        _ => {
            return CategoryReport::skipped(moltis_import_core::report::ImportCategory::McpServers);
        },
    };

    merge_mcp_servers(&servers, dest_path)
}

/// Count MCP servers configured in the Codex config file.
pub fn count_mcp_servers(detection: &CodexDetection) -> usize {
    detection
        .config_path
        .as_ref()
        .and_then(|p| extract_mcp_from_config(p))
        .map(|s| s.len())
        .unwrap_or(0)
}

/// Extract MCP servers from a Codex `config.toml`.
///
/// Expects a `[mcp_servers.<name>]` table structure with fields:
/// `command`, `args`, `env`, `url`, `type`/`transport`.
fn extract_mcp_from_config(path: &Path) -> Option<HashMap<String, ImportMcpServer>> {
    let content = std::fs::read_to_string(path).ok()?;
    let root: toml::Value = content.parse().ok()?;

    let mcp_table = root.get("mcp_servers").and_then(|v| v.as_table())?;
    let mut result = HashMap::new();

    for (name, value) in mcp_table {
        let name = name.trim().to_string();
        if name.is_empty() {
            continue;
        }
        let Some(obj) = value.as_table() else {
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
            .and_then(|v| v.as_table())
            .map(|tbl| {
                tbl.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        let transport = obj
            .get("type")
            .or_else(|| obj.get("transport"))
            .and_then(|v| v.as_str())
            .map(String::from);

        let url = obj.get("url").and_then(|v| v.as_str()).map(String::from);

        let headers: HashMap<String, String> = obj
            .get("headers")
            .and_then(|v| v.as_table())
            .map(|tbl| {
                tbl.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        debug!(name, command = %command, "found Codex MCP server");

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

    fn make_detection() -> CodexDetection {
        CodexDetection {
            home_dir: std::path::PathBuf::new(),
            config_path: None,
            instructions_path: None,
            has_data: false,
        }
    }

    #[test]
    fn extract_from_config_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[model]
name = "o3"

[mcp_servers.my-server]
command = "npx"
args = ["-y", "@my/mcp-server"]

[mcp_servers.my-server.env]
API_KEY = "test-123"

[mcp_servers.remote]
type = "sse"
url = "https://example.com/mcp"
"#,
        )
        .unwrap();

        let servers = extract_mcp_from_config(&path).unwrap();
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
    fn import_mcp_servers_from_config() {
        let tmp = tempfile::tempdir().unwrap();

        let config = tmp.path().join("config.toml");
        std::fs::write(
            &config,
            r#"
[mcp_servers.server-a]
command = "a"
args = []
"#,
        )
        .unwrap();

        let dest = tmp.path().join("mcp-servers.json");
        let mut detection = make_detection();
        detection.config_path = Some(config);

        let report = import_mcp_servers(&detection, &dest);
        assert_eq!(report.items_imported, 1);

        let content = std::fs::read_to_string(&dest).unwrap();
        let loaded: HashMap<String, serde_json::Value> = serde_json::from_str(&content).unwrap();
        assert!(loaded.contains_key("server-a"));
    }

    #[test]
    fn no_mcp_section_returns_skipped() {
        let tmp = tempfile::tempdir().unwrap();

        let config = tmp.path().join("config.toml");
        std::fs::write(&config, "[model]\nname = \"o3\"\n").unwrap();

        let dest = tmp.path().join("mcp-servers.json");
        let mut detection = make_detection();
        detection.config_path = Some(config);

        let report = import_mcp_servers(&detection, &dest);
        assert_eq!(
            report.status,
            moltis_import_core::report::ImportStatus::Skipped
        );
    }

    #[test]
    fn no_config_returns_skipped() {
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
