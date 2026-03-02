//! mDNS/DNS-SD service advertisement for `_moltis._tcp`.
//!
//! Allows iOS (and other) clients on the same LAN to discover this gateway
//! automatically via Bonjour / mDNS browse.

use mdns_sd::{ServiceDaemon, ServiceInfo};

const SERVICE_TYPE: &str = "_moltis._tcp.local.";

fn alias_service_type(alias_slug: &str) -> Option<String> {
    let mut slug = alias_slug.trim().trim_matches('-').to_ascii_lowercase();
    if slug.is_empty() || slug == "moltis" {
        return None;
    }

    // DNS-SD service labels should be short and conservative.
    if slug.len() > 15 {
        slug.truncate(15);
        slug = slug.trim_matches('-').to_string();
    }
    if slug.is_empty() {
        return None;
    }

    Some(format!("_{slug}._tcp.local."))
}

/// Register this gateway as a `_moltis._tcp` mDNS service.
///
/// Returns the [`ServiceDaemon`] handle — keep it alive for as long as the
/// service should be discoverable. Dropping it or calling [`shutdown`] will
/// unregister the service.
pub fn register(
    instance_name: &str,
    port: u16,
    version: &str,
    service_alias_slug: Option<&str>,
) -> anyhow::Result<ServiceDaemon> {
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

    if let Some(alias_type) = service_alias_slug.and_then(alias_service_type) {
        let alias_service = ServiceInfo::new(
            &alias_type,
            instance_name,
            &host_label,
            "",
            port,
            &properties[..],
        )?
        .enable_addr_auto();
        daemon.register(alias_service)?;
        tracing::info!(
            service_type = alias_type,
            instance = instance_name,
            port,
            "mDNS alias service registered"
        );
    }

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
    fn alias_service_type_uses_slug() {
        assert_eq!(
            alias_service_type("my-bot").as_deref(),
            Some("_my-bot._tcp.local.")
        );
    }

    #[test]
    fn alias_service_type_ignores_empty_or_default() {
        assert!(alias_service_type("").is_none());
        assert!(alias_service_type("moltis").is_none());
    }

    #[test]
    fn register_and_shutdown_smoke() {
        let daemon =
            register("test-instance", 0, "0.0.0-test", None).expect("mDNS register should succeed");
        shutdown(&daemon);
    }

    #[test]
    fn register_with_unicode_instance_name() {
        let daemon = register("moltis-тест", 0, "0.0.0-test", Some("my-bot"))
            .expect("mDNS register should handle unicode");
        shutdown(&daemon);
    }
}
