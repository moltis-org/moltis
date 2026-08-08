//! Shared MCP server import utilities.
//!
//! Provides a common merge function used by all import sources to write
//! MCP server entries into Moltis's `mcp-servers.json`.

use std::{collections::HashMap, io::Write, path::Path};

use {
    fs2::FileExt,
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

    let _lock = match acquire_registry_lock(dest_path) {
        Ok(lock) => lock,
        Err(error) => {
            return CategoryReport::failed(ImportCategory::McpServers, error);
        },
    };

    let mut document = if dest_path.is_file() {
        match std::fs::read_to_string(dest_path) {
            Ok(content) => match ImportMcpRegistryDocument::parse(&content) {
                Ok(document) => document,
                Err(e) => {
                    return CategoryReport::failed(
                        ImportCategory::McpServers,
                        format!("existing mcp-servers.json is malformed: {e}"),
                    );
                },
            },
            Err(e) => {
                return CategoryReport::failed(
                    ImportCategory::McpServers,
                    format!("failed to read existing mcp-servers.json: {e}"),
                );
            },
        }
    } else {
        ImportMcpRegistryDocument::default()
    };

    let mut imported = 0;
    let mut skipped = 0;

    for (name, server) in servers {
        if document.servers().contains_key(name) {
            debug!(name, "MCP server already exists, skipping");
            skipped += 1;
            continue;
        }

        debug!(name, command = %server.command, "importing MCP server");
        let value = match serde_json::to_value(server) {
            Ok(value) => value,
            Err(e) => {
                return CategoryReport::failed(
                    ImportCategory::McpServers,
                    format!("failed to serialize MCP server '{name}': {e}"),
                );
            },
        };
        document.servers_mut().insert(name.clone(), value);
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
        let json = match serde_json::to_string_pretty(&document.into_structured_value()) {
            Ok(j) => j,
            Err(e) => {
                return CategoryReport::failed(
                    ImportCategory::McpServers,
                    format!("failed to serialize MCP servers: {e}"),
                );
            },
        };
        if let Err(e) = atomic_write(dest_path, json.as_bytes()) {
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

fn acquire_registry_lock(dest_path: &Path) -> Result<std::fs::File, String> {
    let data_dir = dest_path.parent().unwrap_or_else(|| Path::new("."));
    let lock_dir = data_dir.join("mcp-repositories");
    std::fs::create_dir_all(&lock_dir)
        .map_err(|error| format!("failed to create MCP registry lock directory: {error}"))?;
    let lock_path = lock_dir.join(".lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| format!("failed to open MCP registry lock: {error}"))?;
    file.try_lock_exclusive().map_err(|error| {
        if error.kind() == std::io::ErrorKind::WouldBlock {
            "MCP registry is in use by the running gateway; import before startup or use MCP management APIs"
                .to_string()
        } else {
            format!("failed to lock MCP registry: {error}")
        }
    })?;
    Ok(file)
}

fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    temporary.write_all(data)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[derive(Debug, Default)]
struct ImportMcpRegistryDocument {
    top_level: serde_json::Map<String, serde_json::Value>,
    servers: serde_json::Map<String, serde_json::Value>,
}

impl ImportMcpRegistryDocument {
    fn parse(content: &str) -> serde_json::Result<Self> {
        let value: serde_json::Value = serde_json::from_str(content)?;
        let mut top_level = value.as_object().cloned().ok_or_else(|| {
            <serde_json::Error as serde::de::Error>::custom("MCP registry must be a JSON object")
        })?;

        if top_level.contains_key("servers") || top_level.contains_key("repositories") {
            let servers = top_level
                .remove("servers")
                .map(|value| {
                    value.as_object().cloned().ok_or_else(|| {
                        <serde_json::Error as serde::de::Error>::custom(
                            "structured MCP registry field 'servers' must be an object",
                        )
                    })
                })
                .transpose()?
                .unwrap_or_default();
            return Ok(Self { top_level, servers });
        }

        Ok(Self {
            servers: top_level,
            top_level: serde_json::Map::new(),
        })
    }

    fn servers(&self) -> &serde_json::Map<String, serde_json::Value> {
        &self.servers
    }

    fn servers_mut(&mut self) -> &mut serde_json::Map<String, serde_json::Value> {
        &mut self.servers
    }

    fn into_structured_value(mut self) -> serde_json::Value {
        self.top_level.insert(
            "servers".to_string(),
            serde_json::Value::Object(self.servers),
        );
        serde_json::Value::Object(self.top_level)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct RegistryShape {
        servers: HashMap<String, ImportMcpServer>,
    }

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

        let content = std::fs::read_to_string(&dest).unwrap();
        let loaded: RegistryShape = serde_json::from_str(&content).unwrap();
        assert_eq!(loaded.servers["test"].command, "test-server");
    }

    #[test]
    fn merge_skips_duplicates() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("mcp-servers.json");

        std::fs::write(
            &dest,
            r#"{"servers":{"existing":{"command":"old","args":[],"env":{},"enabled":true}}}"#,
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
    fn merge_migrates_legacy_flat_file_and_preserves_existing() {
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
        let loaded: RegistryShape = serde_json::from_str(&content).unwrap();
        assert_eq!(loaded.servers["old"].command, "old");
        assert_eq!(loaded.servers["new"].command, "new-server");

        let value: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(value.get("old").is_none());
    }

    #[test]
    fn merge_preserves_structured_registry_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("mcp-servers.json");
        std::fs::write(
            &dest,
            r#"{
                "servers": {
                    "managed": {
                        "command": "managed",
                        "managed_origin": {
                            "approval": {"commit":"abc","config_digest":"digest"}
                        }
                    }
                },
                "repositories": {"repo-1":{"alias":"managed-tools"}},
                "future_registry_field": {"version":2}
            }"#,
        )
        .unwrap();

        let before: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&dest).unwrap()).unwrap();

        let servers = HashMap::from([("new".to_string(), ImportMcpServer {
            command: "new".to_string(),
            ..Default::default()
        })]);
        let report = merge_mcp_servers(&servers, &dest);
        assert_eq!(report.items_imported, 1);

        let content = std::fs::read_to_string(&dest).unwrap();
        let loaded: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(loaded["repositories"], before["repositories"]);
        assert_eq!(
            loaded["future_registry_field"],
            before["future_registry_field"]
        );
        assert_eq!(
            loaded["servers"]["managed"]["managed_origin"]["approval"],
            before["servers"]["managed"]["managed_origin"]["approval"]
        );
        assert_eq!(loaded["servers"]["new"]["command"], "new");
    }

    #[test]
    fn malformed_structured_registry_returns_failed() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("mcp-servers.json");
        std::fs::write(&dest, r#"{"servers":"invalid","repositories":{}}"#).unwrap();

        let servers = HashMap::from([("new".to_string(), ImportMcpServer {
            command: "new".to_string(),
            ..Default::default()
        })]);
        let report = merge_mcp_servers(&servers, &dest);

        assert_eq!(report.status, ImportStatus::Failed);
        assert_eq!(
            std::fs::read_to_string(&dest).unwrap(),
            r#"{"servers":"invalid","repositories":{}}"#
        );
    }

    #[test]
    fn merge_rejects_concurrent_gateway_registry_owner() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("mcp-servers.json");
        let lock_dir = tmp.path().join("mcp-repositories");
        std::fs::create_dir_all(&lock_dir).unwrap();
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(lock_dir.join(".lock"))
            .unwrap();
        lock.try_lock_exclusive().unwrap();

        let servers = HashMap::from([("new".to_string(), ImportMcpServer {
            command: "new".to_string(),
            ..Default::default()
        })]);
        let report = merge_mcp_servers(&servers, &dest);

        assert_eq!(report.status, ImportStatus::Failed);
        assert!(report.errors[0].contains("running gateway"));
        assert!(!dest.exists());
    }

    #[test]
    fn structured_registry_without_servers_is_not_treated_as_legacy() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("mcp-servers.json");
        std::fs::write(
            &dest,
            r#"{"repositories":{"repo-1":{"alias":"managed-tools"}}}"#,
        )
        .unwrap();

        let servers = HashMap::from([("new".to_string(), ImportMcpServer {
            command: "new".to_string(),
            ..Default::default()
        })]);
        let report = merge_mcp_servers(&servers, &dest);

        assert_eq!(report.status, ImportStatus::Success);
        let loaded: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&dest).unwrap()).unwrap();
        assert_eq!(loaded["repositories"]["repo-1"]["alias"], "managed-tools");
        assert!(loaded["servers"].get("repositories").is_none());
        assert_eq!(loaded["servers"]["new"]["command"], "new");
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
