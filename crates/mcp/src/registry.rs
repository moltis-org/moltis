//! McpRegistry: persisted configuration of MCP servers (add/remove/enable/disable).

use std::{
    collections::{BTreeMap, HashMap},
    io::Write,
    path::{Path, PathBuf},
};

use {
    secrecy::{ExposeSecret, Secret},
    serde::{Deserialize, Serialize},
    tracing::{debug, info},
};

use crate::{
    error::{Context, Result},
    managed_repositories::{ManagedOrigin, ManagedRepository, ManagedRepositoryId},
};

/// Transport type for MCP server connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TransportType {
    #[default]
    Stdio,
    Sse,
    #[serde(rename = "streamable-http", alias = "streamable_http", alias = "http")]
    StreamableHttp,
}

impl std::fmt::Display for TransportType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stdio => write!(f, "stdio"),
            Self::Sse => write!(f, "sse"),
            Self::StreamableHttp => write!(f, "streamable-http"),
        }
    }
}

/// Manual OAuth override for MCP servers that don't support standard discovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpOAuthConfig {
    pub client_id: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "moltis_oauth::types::serialize_option_secret",
        deserialize_with = "moltis_oauth::types::deserialize_option_secret"
    )]
    pub client_secret: Option<Secret<String>>,
    pub auth_url: String,
    pub token_url: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// Configuration for a single MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Working directory for stdio server processes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    #[serde(
        default,
        skip_serializing_if = "HashMap::is_empty",
        serialize_with = "serialize_secret_string_map",
        deserialize_with = "deserialize_secret_string_map"
    )]
    pub env: HashMap<String, Secret<String>>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_timeout_secs: Option<u64>,
    #[serde(default)]
    pub transport: TransportType,
    /// URL for remote transport. Required when `transport` is `Sse` or `StreamableHttp`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "moltis_oauth::types::serialize_option_secret",
        deserialize_with = "moltis_oauth::types::deserialize_option_secret"
    )]
    pub url: Option<Secret<String>>,
    /// Custom headers for remote HTTP/SSE transport.
    #[serde(
        default,
        skip_serializing_if = "HashMap::is_empty",
        serialize_with = "serialize_secret_string_map",
        deserialize_with = "deserialize_secret_string_map"
    )]
    pub headers: HashMap<String, Secret<String>>,
    /// Manual OAuth override (skip discovery/dynamic registration).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth: Option<McpOAuthConfig>,
    /// Custom display name for the server (shown in UI instead of technical ID).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Immutable repository provenance for reconciled managed servers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_origin: Option<ManagedOrigin>,
}

fn default_true() -> bool {
    true
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            command: String::new(),
            args: Vec::new(),
            cwd: None,
            env: HashMap::new(),
            enabled: true,
            request_timeout_secs: None,
            transport: TransportType::default(),
            url: None,
            headers: HashMap::new(),
            oauth: None,
            display_name: None,
            managed_origin: None,
        }
    }
}

/// Persisted registry of MCP server configurations.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpRegistry {
    #[serde(default)]
    pub servers: HashMap<String, McpServerConfig>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub repositories: BTreeMap<ManagedRepositoryId, ManagedRepository>,
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
        let value: serde_json::Value = serde_json::from_str(&data)
            .with_context(|| format!("failed to parse MCP registry: {}", path.display()))?;
        let is_structured = value.get("servers").is_some() || value.get("repositories").is_some();
        let mut registry = if is_structured {
            serde_json::from_value::<Self>(value)
                .with_context(|| format!("failed to parse MCP registry: {}", path.display()))?
        } else {
            Self {
                servers: serde_json::from_value(value)
                    .with_context(|| format!("failed to parse MCP registry: {}", path.display()))?,
                repositories: BTreeMap::new(),
                path: None,
            }
        };
        registry.path = Some(path.to_path_buf());
        Ok(registry)
    }

    /// Save to the registry file.
    pub fn save(&self) -> Result<()> {
        let path = self.path.as_ref().context("no path set for MCP registry")?;
        let data = serde_json::to_string_pretty(self)?;
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;

        let mut temp_file = tempfile::NamedTempFile::new_in(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temp_file
                .as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        temp_file.write_all(data.as_bytes())?;
        temp_file.as_file().sync_all()?;
        temp_file.persist(path).map_err(|error| error.error)?;
        info!(path = %path.display(), "saved MCP registry");
        Ok(())
    }

    /// Add or update a server configuration.
    pub fn add(&mut self, name: String, config: McpServerConfig) -> Result<()> {
        if config.managed_origin.is_some() {
            return Err(crate::Error::message(
                "managed MCP servers must be changed through repository reconciliation",
            ));
        }
        if self
            .servers
            .get(&name)
            .is_some_and(|existing| existing.managed_origin.is_some())
        {
            return Err(crate::Error::message(
                "managed MCP server structure cannot be changed manually",
            ));
        }
        info!(server = %name, command = %config.command, "adding MCP server");
        self.servers.insert(name, config);
        self.save()
    }

    /// Remove a server configuration.
    pub fn remove(&mut self, name: &str) -> Result<bool> {
        if self
            .servers
            .get(name)
            .is_some_and(|config| config.managed_origin.is_some())
        {
            return Err(crate::Error::message(
                "managed MCP servers must be removed with their repository",
            ));
        }
        let removed = self.servers.remove(name).is_some();
        if removed {
            info!(server = %name, "removed MCP server");
            self.save()?;
        }
        Ok(removed)
    }

    /// Enable a server.
    pub fn enable(&mut self, name: &str) -> Result<bool> {
        if let Some((config, origin)) = self.servers.get(name).and_then(|config| {
            config
                .managed_origin
                .as_ref()
                .map(|origin| (config, origin))
        }) {
            if let Some(reason) = crate::managed_repositories::managed_approval_block_reason(config)
            {
                return Err(crate::Error::message(format!(
                    "managed MCP server cannot be enabled: {reason}"
                )));
            }
            let active_matches = self
                .repositories
                .get(&origin.repository_id)
                .and_then(|repository| repository.active.as_ref())
                .is_some_and(|active| {
                    active.commit == origin.discovered_commit
                        && crate::managed_repositories::managed_config_is_current(config, active)
                });
            if !active_matches || !origin.is_currently_approved() {
                return Err(crate::Error::message(
                    "managed MCP server is not approved for its current commit and config",
                ));
            }
        }
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

    /// Update user-controlled presentation overlays without changing managed structure.
    pub fn update_managed_overlays(
        &mut self,
        name: &str,
        display_name: Option<String>,
        request_timeout_secs: Option<u64>,
    ) -> Result<bool> {
        let Some(existing) = self.servers.get(name) else {
            return Ok(false);
        };
        if existing.managed_origin.is_none() {
            return Err(crate::Error::message(format!(
                "MCP server '{name}' is not managed"
            )));
        }
        let mut staged = self.clone();
        let config = staged.servers.get_mut(name).ok_or_else(|| {
            crate::Error::message(format!("managed MCP server '{name}' not found"))
        })?;
        config.display_name = display_name;
        config.request_timeout_secs = request_timeout_secs;
        self.commit_staged(staged)?;
        Ok(true)
    }

    /// Convert one managed entry into a manual server before repository removal.
    pub fn detach_managed_server(&mut self, name: &str) -> Result<bool> {
        let Some(origin) = self
            .servers
            .get(name)
            .and_then(|config| config.managed_origin.clone())
        else {
            return Ok(false);
        };
        let mut staged = self.clone();
        if let Some(config) = staged.servers.get_mut(name) {
            config.managed_origin = None;
            config.enabled = false;
        }
        if let Some(repository) = staged.repositories.get_mut(&origin.repository_id) {
            repository.runtime_servers.remove(&origin.identity);
        }
        self.commit_staged(staged)?;
        Ok(true)
    }

    /// List all server names.
    pub fn list(&self) -> Vec<&str> {
        self.servers.keys().map(String::as_str).collect()
    }

    /// Get a server config by name.
    pub fn get(&self, name: &str) -> Option<&McpServerConfig> {
        self.servers.get(name)
    }

    pub(crate) fn commit_staged(&mut self, staged: Self) -> Result<()> {
        staged.save()?;
        *self = staged;
        Ok(())
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

fn serialize_secret_string_map<S: serde::Serializer>(
    values: &HashMap<String, Secret<String>>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    let plain: HashMap<&str, &str> = values
        .iter()
        .map(|(key, value)| (key.as_str(), value.expose_secret().as_str()))
        .collect();
    plain.serialize(serializer)
}

fn deserialize_secret_string_map<'de, D>(
    deserializer: D,
) -> std::result::Result<HashMap<String, Secret<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let plain: HashMap<String, String> = HashMap::deserialize(deserializer)?;
    Ok(plain
        .into_iter()
        .map(|(key, value)| (key, Secret::new(value)))
        .collect())
}

#[cfg(test)]
mod tests {
    use {super::*, secrecy::ExposeSecret};

    #[test]
    fn test_transport_type_deserialization() {
        let json = r#"["stdio", "sse", "streamable-http", "streamable_http", "http"]"#;
        let transports: Vec<TransportType> = serde_json::from_str(json).unwrap();
        assert_eq!(transports, vec![
            TransportType::Stdio,
            TransportType::Sse,
            TransportType::StreamableHttp,
            TransportType::StreamableHttp,
            TransportType::StreamableHttp,
        ]);
    }

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
            request_timeout_secs: Some(45),
            ..Default::default()
        });

        let json = serde_json::to_string(&reg).unwrap();
        let parsed: McpRegistry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.servers.len(), 1);
        assert_eq!(parsed.servers["fs"].command, "mcp-server-filesystem");
        assert_eq!(parsed.servers["fs"].args, vec!["/tmp"]);
        assert_eq!(parsed.servers["fs"].request_timeout_secs, Some(45));
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
            env: HashMap::from([("FOO".into(), Secret::new("bar".into()))]),
            ..Default::default()
        });
        reg.save().unwrap();

        let loaded = McpRegistry::load(&path).unwrap();
        assert_eq!(loaded.servers.len(), 1);
        assert_eq!(loaded.servers["test"].env["FOO"].expose_secret(), "bar");

        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(value["servers"].get("test").is_some());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn test_load_legacy_flat_registry_and_save_migrates_without_data_loss() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        std::fs::write(
            &path,
            r#"{"legacy":{"command":"echo","args":["hello"],"enabled":false}}"#,
        )
        .unwrap();

        let registry = McpRegistry::load(&path).unwrap();
        assert_eq!(registry.servers["legacy"].command, "echo");
        assert_eq!(registry.servers["legacy"].args, ["hello"]);
        assert!(!registry.servers["legacy"].enabled);
        registry.save().unwrap();

        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["servers"]["legacy"]["command"], "echo");
        assert!(value.get("legacy").is_none());
    }

    #[test]
    fn test_load_rejects_malformed_structured_registry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        std::fs::write(&path, r#"{"servers":{"broken":{"command":7}}}"#).unwrap();

        let error = McpRegistry::load(&path).unwrap_err();
        assert!(error.to_string().contains("failed to parse MCP registry"));
    }

    #[test]
    fn test_load_structured_registry_without_servers_is_not_legacy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        std::fs::write(&path, r#"{"repositories":{}}"#).unwrap();

        let registry = McpRegistry::load(&path).unwrap();
        assert!(registry.servers.is_empty());
        assert!(registry.repositories.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn test_save_replaces_registry_with_mode_0600() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        std::fs::write(&path, "old contents").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let old_inode = std::fs::metadata(&path).unwrap().ino();

        let registry = McpRegistry {
            servers: HashMap::from([("test".into(), McpServerConfig {
                command: "echo".into(),
                ..Default::default()
            })]),
            repositories: BTreeMap::new(),
            path: Some(path.clone()),
        };
        registry.save().unwrap();

        let metadata = std::fs::metadata(&path).unwrap();
        assert_ne!(metadata.ino(), old_inode);
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn test_registry_roundtrips_secret_remote_values() {
        let mut reg = McpRegistry::new();
        reg.servers.insert("remote".into(), McpServerConfig {
            transport: TransportType::Sse,
            url: Some(Secret::new(
                "https://example.com/mcp?api_key=secret-value".to_string(),
            )),
            headers: HashMap::from([(
                "x-api-key".to_string(),
                Secret::new("header-secret".to_string()),
            )]),
            ..Default::default()
        });

        let json = serde_json::to_string(&reg).unwrap();
        let parsed: McpRegistry = serde_json::from_str(&json).unwrap();
        let server = &parsed.servers["remote"];
        assert_eq!(
            server
                .url
                .as_ref()
                .map(ExposeSecret::expose_secret)
                .map(String::as_str),
            Some("https://example.com/mcp?api_key=secret-value")
        );
        assert_eq!(server.headers["x-api-key"].expose_secret(), "header-secret");
    }

    #[test]
    fn test_registry_roundtrips_oauth_client_secret() {
        let mut reg = McpRegistry::new();
        reg.servers.insert("remote".into(), McpServerConfig {
            oauth: Some(McpOAuthConfig {
                client_id: "client".to_string(),
                client_secret: Some(Secret::new("secret".to_string())),
                auth_url: "https://auth.example.com/authorize".to_string(),
                token_url: "https://auth.example.com/token".to_string(),
                scopes: Vec::new(),
            }),
            ..Default::default()
        });

        let json = serde_json::to_string(&reg).unwrap();
        let parsed: McpRegistry = serde_json::from_str(&json).unwrap();
        let secret = parsed.servers["remote"]
            .oauth
            .as_ref()
            .and_then(|oauth| oauth.client_secret.as_ref())
            .map(ExposeSecret::expose_secret);
        assert_eq!(secret.map(String::as_str), Some("secret"));
    }
}
