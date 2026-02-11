//! McpRegistry: persisted configuration of MCP servers (add/remove/enable/disable).

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use {
    anyhow::{Context, Result},
    serde::{Deserialize, Serialize},
    tracing::{debug, info},
};

/// Transport type for MCP server connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TransportType {
    #[default]
    Stdio,
    Sse,
}

/// Configuration for a single MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub transport: TransportType,
    /// URL for SSE transport. Required when `transport` is `Sse`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

fn default_true() -> bool {
    true
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            command: String::new(),
            args: Vec::new(),
            env: HashMap::new(),
            enabled: true,
            transport: TransportType::default(),
            url: None,
        }
    }
}

/// Persisted registry of MCP server configurations.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpRegistry {
    #[serde(default)]
    pub servers: HashMap<String, McpServerConfig>,
    /// File path for persistence (not serialized).
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl McpRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load from a JSON file, or return empty if the file doesn't exist.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            debug!(path = %path.display(), "MCP registry file not found, using empty");
            return Ok(Self {
                path: Some(path.to_path_buf()),
                ..Default::default()
            });
        }

        let data = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read MCP registry: {}", path.display()))?;
        let mut registry: Self = serde_json::from_str(&data)
            .with_context(|| format!("failed to parse MCP registry: {}", path.display()))?;
        registry.path = Some(path.to_path_buf());
        Ok(registry)
    }

    /// Save to the registry file.
    pub fn save(&self) -> Result<()> {
        let path = self.path.as_ref().context("no path set for MCP registry")?;
        let data = serde_json::to_string_pretty(self)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, data)?;
        info!(path = %path.display(), "saved MCP registry");
        Ok(())
    }

    /// Add or update a server configuration.
    pub fn add(&mut self, name: String, config: McpServerConfig) -> Result<()> {
        info!(server = %name, command = %config.command, "adding MCP server");
        self.servers.insert(name, config);
        self.save()
    }

    /// Remove a server configuration.
    pub fn remove(&mut self, name: &str) -> Result<bool> {
        let removed = self.servers.remove(name).is_some();
        if removed {
            info!(server = %name, "removed MCP server");
            self.save()?;
        }
        Ok(removed)
    }

    /// Enable a server.
    pub fn enable(&mut self, name: &str) -> Result<bool> {
        if let Some(cfg) = self.servers.get_mut(name) {
            cfg.enabled = true;
            self.save()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Disable a server.
    pub fn disable(&mut self, name: &str) -> Result<bool> {
        if let Some(cfg) = self.servers.get_mut(name) {
            cfg.enabled = false;
            self.save()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// List all server names.
    pub fn list(&self) -> Vec<&str> {
        self.servers.keys().map(String::as_str).collect()
    }

    /// Get a server config by name.
    pub fn get(&self, name: &str) -> Option<&McpServerConfig> {
        self.servers.get(name)
    }

    /// Get all enabled server configs.
    pub fn enabled_servers(&self) -> Vec<(&str, &McpServerConfig)> {
        self.servers
            .iter()
            .filter(|(_, cfg)| cfg.enabled)
            .map(|(name, cfg)| (name.as_str(), cfg))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_add_remove() {
        let mut reg = McpRegistry::new();
        reg.servers.insert("test".into(), McpServerConfig {
            command: "echo".into(),
            ..Default::default()
        });
        assert_eq!(reg.list().len(), 1);
        assert!(reg.get("test").is_some());

        reg.servers.remove("test");
        assert!(reg.get("test").is_none());
    }

    #[test]
    fn test_registry_enable_disable() {
        let mut reg = McpRegistry::new();
        reg.servers.insert("srv".into(), McpServerConfig {
            command: "test".into(),
            ..Default::default()
        });

        assert_eq!(reg.enabled_servers().len(), 1);

        reg.servers.get_mut("srv").unwrap().enabled = false;
        assert_eq!(reg.enabled_servers().len(), 0);
    }

    #[test]
    fn test_registry_serialization() {
        let mut reg = McpRegistry::new();
        reg.servers.insert("fs".into(), McpServerConfig {
            command: "mcp-server-filesystem".into(),
            args: vec!["/tmp".into()],
            ..Default::default()
        });

        let json = serde_json::to_string(&reg).unwrap();
        let parsed: McpRegistry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.servers.len(), 1);
        assert_eq!(parsed.servers["fs"].command, "mcp-server-filesystem");
        assert_eq!(parsed.servers["fs"].args, vec!["/tmp"]);
    }

    #[test]
    fn test_load_nonexistent_returns_empty() {
        let reg = McpRegistry::load(Path::new("/nonexistent/path/mcp.json")).unwrap();
        assert!(reg.servers.is_empty());
    }

    #[test]
    fn test_load_and_save_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");

        let mut reg = McpRegistry::load(&path).unwrap();
        reg.servers.insert("test".into(), McpServerConfig {
            command: "echo".into(),
            args: vec!["hello".into()],
            env: HashMap::from([("FOO".into(), "bar".into())]),
            ..Default::default()
        });
        reg.save().unwrap();

        let loaded = McpRegistry::load(&path).unwrap();
        assert_eq!(loaded.servers.len(), 1);
        assert_eq!(loaded.servers["test"].env["FOO"], "bar");
    }
}
