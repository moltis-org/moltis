//! mDNS/DNS-SD service advertisement for `_moltis._tcp`.
//!
//! Allows iOS (and other) clients on the same LAN to discover this gateway
//! automatically via Bonjour / mDNS browse.

use mdns_sd::{ServiceDaemon, ServiceInfo};

const SERVICE_TYPE: &str = "_moltis._tcp.local.";

/// Register this gateway as a `_moltis._tcp` mDNS service.
///
/// Returns the [`ServiceDaemon`] handle — keep it alive for as long as the
/// service should be discoverable. Dropping it or calling [`shutdown`] will
/// unregister the service.
pub fn register(instance_name: &str, port: u16, version: &str) -> anyhow::Result<ServiceDaemon> {
    let daemon = ServiceDaemon::new()?;

    let host = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "moltis-gateway".to_string());

    let host_label = format!("{host}.local.");

    let properties = [("version", version), ("hostname", &host)];

    let service = ServiceInfo::new(
        SERVICE_TYPE,
        instance_name,
        &host_label,
        "",
        port,
        &properties[..],
    )?
    .enable_addr_auto();

    daemon.register(service)?;

    tracing::info!(
        service_type = SERVICE_TYPE,
        instance = instance_name,
        port,
        "mDNS service registered"
    );

    Ok(daemon)
}

/// Gracefully unregister and shut down the mDNS daemon.
pub fn shutdown(daemon: &ServiceDaemon) {
    match daemon.shutdown() {
        Ok(receiver) => match receiver.recv() {
            Ok(status) => tracing::debug!(?status, "mDNS daemon shut down"),
            Err(e) => tracing::debug!("mDNS shutdown recv error: {e}"),
        },
        Err(e) => tracing::debug!("mDNS shutdown error: {e}"),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn service_type_is_moltis_tcp() {
        assert_eq!(SERVICE_TYPE, "_moltis._tcp.local.");
    }

    #[test]
    fn register_and_shutdown_smoke() {
        let daemon =
            register("test-instance", 0, "0.0.0-test").expect("mDNS register should succeed");
        shutdown(&daemon);
    }

    #[test]
    fn register_with_unicode_instance_name() {
        let daemon =
            register("moltis-тест", 0, "0.0.0-test").expect("mDNS register should handle unicode");
        shutdown(&daemon);
    }
}
