//! The daemon configured on this machine, found through `moltis.toml`.

use std::path::{Path, PathBuf};

use moltis_config::MoltisConfig;

use crate::{endpoint::Endpoint, error::Result, tls::TlsTrust};

/// A daemon this machine is configured to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalDaemon {
    pub endpoint: Endpoint,
    pub config_path: PathBuf,
}

/// Read the local configuration without creating or changing anything.
/// `None` when moltis was never set up on this machine.
#[must_use]
pub fn local_daemon() -> Option<LocalDaemon> {
    let config_path = moltis_config::find_or_default_config_path();
    if !config_path.exists() {
        return None;
    }
    let config = moltis_config::discover_and_load_readonly();
    let config_dir = moltis_config::config_dir()?;
    let endpoint = endpoint_for(&config, &config_dir).ok()?;
    Some(LocalDaemon {
        endpoint,
        config_path,
    })
}

/// Where a daemon running with `config` can be reached from this machine.
pub fn endpoint_for(config: &MoltisConfig, config_dir: &Path) -> Result<Endpoint> {
    let host = connect_host(&config.server.bind);
    let port = config.server.port;
    if !config.tls.enabled {
        return Endpoint::parse(&format!("http://{host}:{port}"), TlsTrust::System);
    }
    // Auto-generated certificates chain to the local CA; a custom
    // certificate is expected to be trusted by the system.
    let ca_path = config
        .tls
        .ca_cert_path
        .as_ref()
        .map_or_else(|| config_dir.join("certs").join("ca.pem"), PathBuf::from);
    let tls = if config.tls.cert_path.is_none() || config.tls.ca_cert_path.is_some() {
        std::fs::read_to_string(&ca_path)
            .map(|pem| TlsTrust::CaPem { pem })
            .unwrap_or_default()
    } else {
        TlsTrust::System
    };
    Endpoint::parse(&format!("https://{host}:{port}"), tls)
}

/// Wildcard and loopback binds are reached as `localhost`, which the
/// generated certificate always covers.
fn connect_host(bind: &str) -> String {
    match bind.trim() {
        "" | "0.0.0.0" | "::" | "[::]" | "127.0.0.1" | "::1" | "[::1]" | "localhost" => {
            "localhost".into()
        },
        other if other.contains(':') && !other.starts_with('[') => format!("[{other}]"),
        other => other.into(),
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    fn config(bind: &str, port: u16, tls: bool) -> MoltisConfig {
        let mut config = MoltisConfig::default();
        config.server.bind = bind.into();
        config.server.port = port;
        config.tls.enabled = tls;
        config
    }

    #[test]
    fn plain_http_on_loopback() {
        let endpoint = endpoint_for(&config("127.0.0.1", 4000, false), Path::new("/x")).unwrap();
        assert_eq!(endpoint.to_string(), "http://localhost:4000");
        assert_eq!(endpoint.tls, TlsTrust::System);
    }

    #[test]
    fn tls_uses_local_ca() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("certs")).unwrap();
        std::fs::write(dir.path().join("certs/ca.pem"), "PEM").unwrap();
        let endpoint = endpoint_for(&config("0.0.0.0", 18789, true), dir.path()).unwrap();
        assert_eq!(endpoint.to_string(), "https://localhost:18789");
        assert_eq!(endpoint.tls, TlsTrust::CaPem { pem: "PEM".into() });
    }

    #[test]
    fn tls_without_ca_falls_back_to_system() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = endpoint_for(&config("10.0.0.5", 1, true), dir.path()).unwrap();
        assert_eq!(endpoint.to_string(), "https://10.0.0.5:1");
        assert_eq!(endpoint.tls, TlsTrust::System);
    }

    #[test]
    fn custom_certificate_uses_system_trust() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("certs")).unwrap();
        std::fs::write(dir.path().join("certs/ca.pem"), "PEM").unwrap();
        let mut config = config("::", 2, true);
        config.tls.cert_path = Some("/etc/cert.pem".into());
        let endpoint = endpoint_for(&config, dir.path()).unwrap();
        assert_eq!(endpoint.tls, TlsTrust::System);
    }

    #[test]
    fn ipv6_bind_is_bracketed() {
        assert_eq!(connect_host("fd00::1"), "[fd00::1]");
        assert_eq!(connect_host("::"), "localhost");
        assert_eq!(connect_host("host.lan"), "host.lan");
    }
}
