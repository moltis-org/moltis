//! Saved daemons the app can connect to, stored as JSON next to the moltis
//! config (`<config_dir>/desktop/servers.json`). API keys are never stored
//! here; they live in the OS keychain under [`ServerProfile::keychain_url`].

use std::{
    io,
    path::{Path, PathBuf},
};

use {
    moltis_client::{Endpoint, TlsTrust},
    serde::{Deserialize, Serialize},
};

#[cfg(feature = "ssh")]
use moltis_client::ssh::SshTarget;

/// Id of the daemon configured on this machine (never saved to disk).
pub const LOCAL_ID: &str = "local";

/// How a daemon is reached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerKind {
    /// Straight to its HTTP(S) port.
    Direct { endpoint: Endpoint },
    /// Through an SSH port forward to its loopback port.
    #[cfg(feature = "ssh")]
    Ssh {
        target: SshTarget,
        #[serde(default)]
        tls: TlsTrust,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerProfile {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub kind: ServerKind,
}

impl ServerProfile {
    /// Offer the local service even before its first launch writes moltis.toml.
    /// Read-only discovery also honours config overrides.
    pub fn local() -> Option<Self> {
        let config = moltis_config::discover_and_load_readonly();
        Self::local_for(&config, &moltis_config::config_dir()?).ok()
    }

    /// The local profile read again, when its trust has changed since this
    /// copy was made. On a clean install the profile is read before the
    /// daemon's first launch writes `certs/ca.pem`, so it starts out trusting
    /// only the system store.
    pub fn reloaded_local(&self) -> Option<Self> {
        self.reloaded(Self::local())
    }

    fn reloaded(&self, fresh: Option<Self>) -> Option<Self> {
        fresh.filter(|fresh| self.is_local() && fresh.tls() != self.tls())
    }

    fn local_for(config: &moltis_config::MoltisConfig, dir: &Path) -> moltis_client::Result<Self> {
        Ok(Self {
            id: LOCAL_ID.into(),
            name: "This computer".into(),
            kind: ServerKind::Direct {
                endpoint: moltis_client::local::endpoint_for(config, dir)?,
            },
        })
    }

    #[must_use]
    pub fn direct(name: impl Into<String>, endpoint: Endpoint) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            kind: ServerKind::Direct { endpoint },
        }
    }

    #[must_use]
    pub fn is_local(&self) -> bool {
        self.id == LOCAL_ID
    }

    /// Keychain entry holding this server's API key.
    #[must_use]
    pub fn keychain_url(&self) -> String {
        format!("moltis-desktop://{}", self.id)
    }

    /// Where it is, for display.
    #[must_use]
    pub fn address(&self) -> String {
        match &self.kind {
            ServerKind::Direct { endpoint } => endpoint.to_string(),
            #[cfg(feature = "ssh")]
            ServerKind::Ssh { target, .. } => {
                format!("ssh {} → port {}", target.destination, target.remote_port)
            },
        }
    }

    /// The TLS trust used for this server.
    #[must_use]
    pub fn tls(&self) -> &TlsTrust {
        match &self.kind {
            ServerKind::Direct { endpoint } => &endpoint.tls,
            #[cfg(feature = "ssh")]
            ServerKind::Ssh { tls, .. } => tls,
        }
    }

    /// Replace the TLS trust (after the user accepted a fingerprint).
    pub fn set_tls(&mut self, trust: TlsTrust) {
        match &mut self.kind {
            ServerKind::Direct { endpoint } => endpoint.tls = trust,
            #[cfg(feature = "ssh")]
            ServerKind::Ssh { tls, .. } => *tls = trust,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreFile {
    #[serde(default)]
    servers: Vec<ServerProfile>,
    #[serde(default)]
    last_used: Option<String>,
}

/// The saved servers and which one was used last.
#[derive(Debug)]
pub struct ServerStore {
    path: Option<PathBuf>,
    file: StoreFile,
}

impl ServerStore {
    /// `<config_dir>/desktop/servers.json`.
    #[must_use]
    pub fn default_path() -> Option<PathBuf> {
        Some(
            moltis_config::config_dir()?
                .join("desktop")
                .join("servers.json"),
        )
    }

    /// Load from `path`; a missing or unreadable file starts empty.
    #[must_use]
    pub fn load(path: Option<PathBuf>) -> Self {
        let file = path
            .as_deref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| match serde_json::from_str(&text) {
                Ok(file) => Some(file),
                Err(error) => {
                    crate::logging::warn!(%error, "ignoring unreadable servers.json");
                    None
                },
            })
            .unwrap_or_default();
        Self { path, file }
    }

    #[must_use]
    pub fn servers(&self) -> &[ServerProfile] {
        &self.file.servers
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&ServerProfile> {
        self.file.servers.iter().find(|server| server.id == id)
    }

    #[must_use]
    pub fn last_used(&self) -> Option<&str> {
        self.file.last_used.as_deref()
    }

    /// Insert or replace by id. The local daemon is never stored.
    pub fn upsert(&mut self, profile: ServerProfile) {
        if profile.is_local() {
            return;
        }
        match self.file.servers.iter_mut().find(|s| s.id == profile.id) {
            Some(existing) => *existing = profile,
            None => self.file.servers.push(profile),
        }
    }

    pub fn remove(&mut self, id: &str) {
        self.file.servers.retain(|server| server.id != id);
        if self.file.last_used.as_deref() == Some(id) {
            self.file.last_used = None;
        }
    }

    pub fn set_last_used(&mut self, id: &str) {
        self.file.last_used = Some(id.to_string());
    }

    /// Write atomically (temp file + rename), owner-only on Unix.
    pub fn save(&self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        write_private(path, serde_json::to_string_pretty(&self.file)?.as_bytes())
    }
}

fn write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&temp, path)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn endpoint(url: &str) -> Endpoint {
        Endpoint::parse(url, TlsTrust::System).unwrap()
    }

    #[test]
    fn clean_install_has_a_local_service_profile() {
        let dir = tempfile::tempdir().unwrap();
        let config = moltis_config::MoltisConfig::default();
        let profile = ServerProfile::local_for(&config, dir.path()).unwrap();
        assert!(profile.is_local());
        assert_eq!(profile.name, "This computer");
        assert!(profile.address().contains("localhost"));
        assert!(!dir.path().join("moltis.toml").exists());
    }

    #[test]
    fn local_profile_picks_up_a_ca_written_after_it_was_read() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = moltis_config::MoltisConfig::default();
        config.tls.enabled = true;
        let before = ServerProfile::local_for(&config, dir.path()).unwrap();
        assert_eq!(before.tls(), &TlsTrust::System);
        // Unchanged trust: keep the profile as is.
        let same = ServerProfile::local_for(&config, dir.path()).unwrap();
        assert_eq!(before.reloaded(Some(same)), None);

        // The daemon's first launch writes its CA.
        std::fs::create_dir_all(dir.path().join("certs")).unwrap();
        std::fs::write(dir.path().join("certs/ca.pem"), "PEM").unwrap();
        let after = ServerProfile::local_for(&config, dir.path()).unwrap();
        let reloaded = before.reloaded(Some(after)).unwrap();
        assert_eq!(reloaded.tls(), &TlsTrust::CaPem { pem: "PEM".into() });

        // Remote profiles are never replaced by the local one.
        let remote = ServerProfile::direct("studio", endpoint("https://studio.local:13131"));
        let fresh = ServerProfile::local_for(&config, dir.path()).unwrap();
        assert_eq!(remote.reloaded(Some(fresh)), None);
    }

    #[test]
    fn local_profile_honours_custom_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = moltis_config::MoltisConfig::default();
        config.server.port = 4321;
        config.tls.enabled = false;
        let profile = ServerProfile::local_for(&config, dir.path()).unwrap();
        assert!(profile.is_local());
        assert_eq!(profile.address(), "http://localhost:4321");
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop/servers.json");
        let mut store = ServerStore::load(Some(path.clone()));
        assert!(store.servers().is_empty());

        let mut profile = ServerProfile::direct("Studio", endpoint("https://studio:1"));
        store.upsert(profile.clone());
        store.set_last_used(&profile.id);
        store.save().unwrap();

        let loaded = ServerStore::load(Some(path.clone()));
        assert_eq!(loaded.servers(), std::slice::from_ref(&profile));
        assert_eq!(loaded.last_used(), Some(profile.id.as_str()));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        profile.set_tls(TlsTrust::Fingerprint {
            sha256: "ab".into(),
        });
        store.upsert(profile.clone());
        assert_eq!(store.servers().len(), 1);
        assert_eq!(store.get(&profile.id).unwrap().tls(), profile.tls());

        store.remove(&profile.id);
        assert!(store.servers().is_empty());
        assert_eq!(store.last_used(), None);
    }

    #[test]
    fn corrupt_or_missing_file_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("servers.json");
        std::fs::write(&path, "{not json").unwrap();
        assert!(ServerStore::load(Some(path)).servers().is_empty());
        assert!(ServerStore::load(None).servers().is_empty());
        ServerStore::load(None).save().unwrap();
    }

    #[test]
    fn local_daemon_is_never_saved() {
        let mut store = ServerStore::load(None);
        let mut local = ServerProfile::direct("This Mac", endpoint("http://localhost:1"));
        local.id = LOCAL_ID.into();
        store.upsert(local);
        assert!(store.servers().is_empty());
    }

    #[test]
    fn keychain_url_and_address() {
        let profile = ServerProfile::direct("x", endpoint("https://h:2"));
        assert!(profile.keychain_url().starts_with("moltis-desktop://"));
        assert_eq!(profile.address(), "https://h:2");
        assert!(!profile.is_local());
    }

    #[cfg(feature = "ssh")]
    #[test]
    fn ssh_profiles_serialize() {
        let profile = ServerProfile {
            id: "s".into(),
            name: "Remote".into(),
            kind: ServerKind::Ssh {
                target: SshTarget {
                    destination: "me@box".into(),
                    remote_port: 18789,
                    ssh_port: None,
                    identity_file: None,
                    tls: true,
                },
                tls: TlsTrust::System,
            },
        };
        let json = serde_json::to_value(&profile).unwrap();
        assert_eq!(json["kind"], "ssh");
        assert_eq!(
            serde_json::from_value::<ServerProfile>(json).unwrap(),
            profile
        );
        assert_eq!(profile.address(), "ssh me@box → port 18789");
    }
}
