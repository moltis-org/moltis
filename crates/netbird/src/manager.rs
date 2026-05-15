use std::{net::IpAddr, str::FromStr};

use {
    async_trait::async_trait,
    serde::{Deserialize, Serialize},
};

use super::{Error, Result};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetbirdMode {
    #[default]
    Off,
    Serve,
}

impl NetbirdMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Serve => "serve",
        }
    }
}

impl std::fmt::Display for NetbirdMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for NetbirdMode {
    type Err = Error;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.to_lowercase().as_str() {
            "off" => Ok(Self::Off),
            "serve" => Ok(Self::Serve),
            other => Err(Error::message(format!(
                "unknown netbird mode: '{other}' (expected off or serve)"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct NetbirdStatus {
    pub mode: NetbirdMode,
    pub installed: bool,
    pub netbird_up: bool,
    pub version: Option<String>,
    pub peer_ip: Option<String>,
    pub dns_name: Option<String>,
    pub url: Option<String>,
}

#[async_trait]
pub trait NetbirdManager: Send + Sync {
    async fn status(&self) -> Result<NetbirdStatus>;
}

#[derive(Default)]
pub struct CliNetbirdManager {
    mode: NetbirdMode,
    port: u16,
    tls: bool,
}

impl CliNetbirdManager {
    #[must_use]
    pub const fn new(mode: NetbirdMode, port: u16, tls: bool) -> Self {
        Self { mode, port, tls }
    }

    async fn run_command(args: &[&str]) -> Result<std::process::Output> {
        tokio::process::Command::new("netbird")
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(|e| Error::message(format!("failed to run netbird CLI: {e}")))
    }
}

#[async_trait]
impl NetbirdManager for CliNetbirdManager {
    async fn status(&self) -> Result<NetbirdStatus> {
        let installed = tokio::process::Command::new("netbird")
            .arg("version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .is_ok();

        if !installed {
            return Ok(NetbirdStatus {
                mode: NetbirdMode::Off,
                installed: false,
                netbird_up: false,
                version: None,
                peer_ip: None,
                dns_name: None,
                url: None,
            });
        }

        let version = Self::run_command(&["version"])
            .await
            .ok()
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|value| value.lines().next().unwrap_or_default().trim().to_string())
            .filter(|value| !value.is_empty());

        let parsed = Self::run_command(&["status", "--json"])
            .await
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| serde_json::from_slice::<serde_json::Value>(&output.stdout).ok());

        let peer_ip = parsed.as_ref().and_then(extract_peer_ip);
        let dns_name = parsed.as_ref().and_then(extract_dns_name);
        let netbird_up = peer_ip.is_some() || dns_name.is_some();
        let url = if self.mode == NetbirdMode::Serve {
            dns_name.as_ref().or(peer_ip.as_ref()).map(|host| {
                format!(
                    "{}://{}:{}",
                    if self.tls {
                        "https"
                    } else {
                        "http"
                    },
                    host,
                    self.port
                )
            })
        } else {
            None
        };

        Ok(NetbirdStatus {
            mode: if netbird_up {
                self.mode
            } else {
                NetbirdMode::Off
            },
            installed,
            netbird_up,
            version,
            peer_ip,
            dns_name,
            url,
        })
    }
}

fn extract_peer_ip(value: &serde_json::Value) -> Option<String> {
    [
        "/netbirdIp",
        "/ip",
        "/managementState/netbirdIp",
        "/fullStatus/managementState/netbirdIp",
    ]
    .into_iter()
    .find_map(|path| value.pointer(path).and_then(|item| item.as_str()))
    .map(ToOwned::to_owned)
}

fn extract_dns_name(value: &serde_json::Value) -> Option<String> {
    [
        "/fqdn",
        "/dnsName",
        "/hostname",
        "/fullStatus/managementState/fqdn",
    ]
    .into_iter()
    .find_map(|path| value.pointer(path).and_then(|item| item.as_str()))
    .map(|value| value.trim_end_matches('.').to_string())
}

#[must_use]
pub fn is_loopback_addr(addr: &str) -> bool {
    match addr {
        "localhost" | "127.0.0.1" | "::1" => true,
        other => other.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback()),
    }
}

pub fn validate_netbird_config(mode: NetbirdMode, bind_addr: &str) -> Result<()> {
    if mode == NetbirdMode::Off || is_loopback_addr(bind_addr) {
        return Ok(());
    }

    Err(Error::message(format!(
        "netbird {mode} requires the gateway to bind to a loopback address (127.0.0.1, ::1, or localhost), but got '{bind_addr}'"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_fields_from_known_json_paths() {
        let value = serde_json::json!({
            "managementState": {
                "netbirdIp": "100.96.0.10"
            },
            "fqdn": "moltis.netbird.cloud."
        });

        assert_eq!(extract_peer_ip(&value).as_deref(), Some("100.96.0.10"));
        assert_eq!(
            extract_dns_name(&value).as_deref(),
            Some("moltis.netbird.cloud")
        );
    }

    #[test]
    fn validates_loopback_only_for_serve() {
        assert!(validate_netbird_config(NetbirdMode::Off, "0.0.0.0").is_ok());
        assert!(validate_netbird_config(NetbirdMode::Serve, "127.0.0.1").is_ok());
        assert!(validate_netbird_config(NetbirdMode::Serve, "0.0.0.0").is_err());
    }
}
