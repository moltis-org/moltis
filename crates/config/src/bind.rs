//! Bind-address classification shared by config validation and the gateway.

use std::net::IpAddr;

/// Whether `bind` only accepts connections from this machine.
///
/// Accepts `localhost`, any `*.localhost` name (RFC 6761), and loopback IPs
/// with or without IPv6 brackets.
#[must_use]
pub fn is_loopback_bind(bind: &str) -> bool {
    let host = bind.trim().trim_end_matches('.');
    if host.eq_ignore_ascii_case("localhost") || host.to_ascii_lowercase().ends_with(".localhost") {
        return true;
    }
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<IpAddr>()
        .is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_names_and_ips_are_loopback() {
        for bind in [
            "127.0.0.1",
            "127.1.2.3",
            "::1",
            "[::1]",
            "localhost",
            "LOCALHOST",
            "moltis.localhost",
            "localhost.",
        ] {
            assert!(is_loopback_bind(bind), "{bind} should be loopback");
        }
    }

    #[test]
    fn wildcard_and_lan_binds_are_not_loopback() {
        for bind in [
            "0.0.0.0",
            "::",
            "192.168.1.10",
            "example.com",
            "localhost.example.com",
            "",
        ] {
            assert!(!is_loopback_bind(bind), "{bind} should not be loopback");
        }
    }
}
