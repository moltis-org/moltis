//! Browse the local network for daemons advertising `_moltis._tcp`.

use std::{collections::BTreeMap, net::IpAddr, time::Duration};

use mdns_sd::{ResolvedService, ServiceDaemon, ServiceEvent};

use crate::{
    endpoint::Endpoint,
    error::{Error, Result},
    tls::TlsTrust,
};

/// DNS-SD service type the gateway registers (see `moltis-gateway::mdns`).
pub const SERVICE_TYPE: &str = "_moltis._tcp.local.";

/// A daemon seen on the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredDaemon {
    /// DNS-SD instance name.
    pub name: String,
    /// The machine's hostname from the TXT record.
    pub hostname: Option<String>,
    pub version: Option<String>,
    pub port: u16,
    pub addresses: Vec<IpAddr>,
}

impl DiscoveredDaemon {
    /// An `https` endpoint for the first address (IPv4 preferred). Its
    /// certificate is unknown, so the caller pins it after the user confirms.
    #[must_use]
    pub fn endpoint(&self) -> Option<Endpoint> {
        let address = self
            .addresses
            .iter()
            .find(|a| a.is_ipv4())
            .or_else(|| self.addresses.first())?;
        let host = match address {
            IpAddr::V4(v4) => v4.to_string(),
            IpAddr::V6(v6) => format!("[{v6}]"),
        };
        Endpoint::parse(&format!("https://{host}:{}", self.port), TlsTrust::System).ok()
    }

    fn from_resolved(service: &ResolvedService) -> Self {
        let mut addresses: Vec<IpAddr> = service
            .get_addresses()
            .iter()
            .map(mdns_sd::ScopedIp::to_ip_addr)
            .filter(|ip| !ip.is_loopback())
            .collect();
        addresses.sort();
        let name = service
            .get_fullname()
            .strip_suffix(SERVICE_TYPE)
            .unwrap_or(service.get_fullname())
            .trim_end_matches('.')
            .to_string();
        Self {
            name,
            hostname: service.get_property_val_str("hostname").map(String::from),
            version: service.get_property_val_str("version").map(String::from),
            port: service.get_port(),
            addresses,
        }
    }
}

/// Collect daemons that answer within `duration`, deduplicated by name.
pub async fn browse(duration: Duration) -> Result<Vec<DiscoveredDaemon>> {
    let daemon = ServiceDaemon::new().map_err(|e| Error::Connect(format!("mDNS: {e}")))?;
    let events = daemon
        .browse(SERVICE_TYPE)
        .map_err(|e| Error::Connect(format!("mDNS browse: {e}")))?;
    let mut found = BTreeMap::new();
    let deadline = tokio::time::Instant::now() + duration;
    loop {
        let event = tokio::select! {
            event = events.recv_async() => event,
            () = tokio::time::sleep_until(deadline) => break,
        };
        match event {
            Ok(ServiceEvent::ServiceResolved(service)) => {
                let daemon = DiscoveredDaemon::from_resolved(&service);
                found.insert(daemon.name.clone(), daemon);
            },
            Ok(ServiceEvent::ServiceRemoved(_, fullname)) => {
                found.retain(|_, d: &mut DiscoveredDaemon| !fullname.starts_with(&d.name));
            },
            Ok(_) => {},
            Err(_) => break,
        }
    }
    let _ = daemon.shutdown();
    Ok(found.into_values().collect())
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    fn daemon(addresses: Vec<IpAddr>) -> DiscoveredDaemon {
        DiscoveredDaemon {
            name: "studio".into(),
            hostname: Some("studio.local".into()),
            version: None,
            port: 18789,
            addresses,
        }
    }

    #[test]
    fn endpoint_prefers_ipv4() {
        let d = daemon(vec![
            "fe80::1".parse().unwrap(),
            "192.168.1.9".parse().unwrap(),
        ]);
        assert_eq!(
            d.endpoint().unwrap().to_string(),
            "https://192.168.1.9:18789"
        );
        let v6 = daemon(vec!["fd00::2".parse().unwrap()]);
        assert_eq!(
            v6.endpoint().unwrap().to_string(),
            "https://[fd00::2]:18789"
        );
        assert!(daemon(vec![]).endpoint().is_none());
    }
}
