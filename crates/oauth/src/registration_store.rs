//! Persists dynamic client registrations for MCP OAuth servers.
//!
//! Stores client credentials at `~/.config/moltis/mcp_oauth_registrations.json`
//! so that re-registration is avoided on subsequent connections.

use std::{collections::HashMap, fmt::Write as _, path::PathBuf};

use {
    secrecy::Secret,
    serde::{Deserialize, Serialize},
    sha2::{Digest, Sha256},
    tracing::{debug, info, warn},
};

use crate::{Result, config_dir::moltis_config_dir};

/// A stored dynamic client registration.
#[derive(Clone, Serialize, Deserialize)]
pub struct StoredRegistration {
    pub client_id: String,
    #[serde(
        default,
        serialize_with = "crate::types::serialize_option_secret",
        skip_serializing_if = "Option::is_none"
    )]
    pub client_secret: Option<Secret<String>>,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub resource: String,
    pub registered_at: u64,
}

impl std::fmt::Debug for StoredRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredRegistration")
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("authorization_endpoint", &self.authorization_endpoint)
            .field("token_endpoint", &self.token_endpoint)
            .field("resource", &self.resource)
            .field("registered_at", &self.registered_at)
            .finish()
    }
}

/// File-based store for MCP OAuth client registrations.
#[derive(Debug, Clone)]
pub struct RegistrationStore {
    path: PathBuf,
}

impl RegistrationStore {
    pub fn new() -> Self {
        let path = moltis_config_dir().join("mcp_oauth_registrations.json");
        Self { path }
    }

    /// Create a store at a specific path (useful for testing).
    pub fn with_path(path: PathBuf) -> Self {
        Self { path }
    }

    /// Load a stored registration for the given server URL.
    pub fn load(&self, server_url: &str) -> Option<StoredRegistration> {
        let server_key = server_lookup_key(server_url);
        let data = match std::fs::read_to_string(&self.path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                debug!(path = %self.path.display(), server_key, "registration file not found");
                return None;
            },
            Err(e) => {
                warn!(
                    path = %self.path.display(),
                    server_key,
                    error = %e,
                    "registration file read failed"
                );
                return None;
            },
        };

        let map: HashMap<String, StoredRegistration> = match serde_json::from_str(&data) {
            Ok(m) => m,
            Err(e) => {
                warn!(
                    path = %self.path.display(),
                    server_key,
                    error = %e,
                    "registration file parse failed"
                );
                return None;
            },
        };

        match map
            .get(&server_key)
            .cloned()
            .or_else(|| map.get(server_url).cloned())
        {
            Some(reg) => {
                debug!(server_key, client_id = %reg.client_id, "loaded stored registration");
                Some(reg)
            },
            None => {
                debug!(server_key, "no stored registration found");
                None
            },
        }
    }

    /// Save a registration for the given server URL.
    pub fn save(&self, server_url: &str, reg: &StoredRegistration) -> Result<()> {
        let server_key = server_lookup_key(server_url);
        info!(server_key, client_id = %reg.client_id, "saving MCP OAuth registration");

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut map: HashMap<String, StoredRegistration> = std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|d| serde_json::from_str(&d).ok())
            .unwrap_or_default();

        map.remove(server_url);
        map.insert(server_key, reg.clone());

        let data = serde_json::to_string_pretty(&map)?;
        std::fs::write(&self.path, &data)?;

        // Set file permissions to 0600 on Unix (contains client secrets)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600))?;
        }

        Ok(())
    }

    /// Delete a stored registration.
    pub fn delete(&self, server_url: &str) -> Result<()> {
        let server_key = server_lookup_key(server_url);
        info!(server_key, "deleting MCP OAuth registration");

        let data = match std::fs::read_to_string(&self.path) {
            Ok(d) => d,
            Err(_) => return Ok(()),
        };

        let mut map: HashMap<String, StoredRegistration> = serde_json::from_str(&data)?;
        map.remove(&server_key);
        map.remove(server_url);

        let data = serde_json::to_string_pretty(&map)?;
        std::fs::write(&self.path, &data)?;
        Ok(())
    }
}

fn server_lookup_key(server_url: &str) -> String {
    let digest = Sha256::digest(server_url.as_bytes());
    let mut key = String::with_capacity("sha256:".len() + digest.len() * 2);
    key.push_str("sha256:");
    for byte in digest {
        let _ = write!(&mut key, "{byte:02x}");
    }
    key
}

impl Default for RegistrationStore {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret;

    use super::*;

    fn temp_store() -> (tempfile::TempDir, RegistrationStore) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("registrations.json");
        let store = RegistrationStore::with_path(path);
        (dir, store)
    }

    fn sample_registration() -> StoredRegistration {
        StoredRegistration {
            client_id: "test-client".to_string(),
            client_secret: Some(Secret::new("test-secret".to_string())),
            authorization_endpoint: "https://auth.example.com/authorize".to_string(),
            token_endpoint: "https://auth.example.com/token".to_string(),
            resource: "https://mcp.example.com".to_string(),
            registered_at: 1_700_000_000,
        }
    }

    #[test]
    fn roundtrip_save_load() {
        let (_dir, store) = temp_store();
        let reg = sample_registration();
        let raw_url = "https://mcp.example.com?api_key=super-secret";

        store.save(raw_url, &reg).unwrap();
        let file_contents = std::fs::read_to_string(&store.path).unwrap();
        assert!(!file_contents.contains(raw_url));
        assert!(!file_contents.contains("super-secret"));

        let loaded = store.load(raw_url).unwrap();

        assert_eq!(loaded.client_id, "test-client");
        assert_eq!(
            loaded
                .client_secret
                .as_ref()
                .map(|s| s.expose_secret().as_str()),
            Some("test-secret")
        );
        assert_eq!(
            loaded.authorization_endpoint,
            "https://auth.example.com/authorize"
        );
        assert_eq!(loaded.token_endpoint, "https://auth.example.com/token");
    }

    #[test]
    fn load_nonexistent_returns_none() {
        let (_dir, store) = temp_store();
        assert!(store.load("https://no-such.example.com").is_none());
    }

    #[test]
    fn load_supports_legacy_raw_url_keys() {
        let (_dir, store) = temp_store();
        let reg = sample_registration();

        let data = serde_json::json!({
            "https://mcp.example.com?token=legacy-secret": reg
        });
        std::fs::write(&store.path, serde_json::to_string_pretty(&data).unwrap()).unwrap();

        let loaded = store
            .load("https://mcp.example.com?token=legacy-secret")
            .unwrap();
        assert_eq!(loaded.client_id, "test-client");
    }

    #[test]
    fn delete_registration() {
        let (_dir, store) = temp_store();
        let reg = sample_registration();
        let raw_url = "https://mcp.example.com?api_key=delete-me";

        store.save(raw_url, &reg).unwrap();
        assert!(store.load(raw_url).is_some());

        store.delete(raw_url).unwrap();
        assert!(store.load(raw_url).is_none());
    }

    #[test]
    fn delete_nonexistent_is_ok() {
        let (_dir, store) = temp_store();
        assert!(store.delete("https://no-such.example.com").is_ok());
    }

    #[test]
    fn multiple_servers() {
        let (_dir, store) = temp_store();
        let mut reg1 = sample_registration();
        reg1.client_id = "client-a".to_string();
        let mut reg2 = sample_registration();
        reg2.client_id = "client-b".to_string();

        store.save("https://server-a.example.com", &reg1).unwrap();
        store.save("https://server-b.example.com", &reg2).unwrap();

        assert_eq!(
            store
                .load("https://server-a.example.com")
                .unwrap()
                .client_id,
            "client-a"
        );
        assert_eq!(
            store
                .load("https://server-b.example.com")
                .unwrap()
                .client_id,
            "client-b"
        );
    }

    #[cfg(unix)]
    #[test]
    fn file_permissions_are_0600() {
        use std::os::unix::fs::PermissionsExt;

        let (_dir, store) = temp_store();
        let reg = sample_registration();
        store.save("https://mcp.example.com", &reg).unwrap();

        let perms = std::fs::metadata(&store.path).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o600);
    }

    #[test]
    fn no_client_secret_roundtrip() {
        let (_dir, store) = temp_store();
        let mut reg = sample_registration();
        reg.client_secret = None;

        store.save("https://mcp.example.com", &reg).unwrap();
        let loaded = store.load("https://mcp.example.com").unwrap();

        assert!(loaded.client_secret.is_none());
    }

    #[test]
    fn debug_redacts_secret() {
        let reg = sample_registration();
        let debug = format!("{reg:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("test-secret"));
    }
}
