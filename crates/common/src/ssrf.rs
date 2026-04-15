use std::net::IpAddr;

use url::Url;

use crate::{Error, Result};

#[must_use]
fn is_private_ipv4(v4: &std::net::Ipv4Addr) -> bool {
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_broadcast()
        || v4.is_unspecified()
        || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64)
        || (v4.octets()[0] == 192 && v4.octets()[1] == 0 && v4.octets()[2] == 0)
}

/// Check if an IP is covered by an SSRF allowlist entry.
#[must_use]
pub fn is_ssrf_allowed(ip: &IpAddr, allowlist: &[ipnet::IpNet]) -> bool {
    allowlist.iter().any(|net| net.contains(ip))
}

/// Check if an IP address is private, loopback, link-local, or otherwise
/// unsuitable for outbound fetches.
#[must_use]
pub fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_ipv4(v4),
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xFE00) == 0xFC00
                || (v6.segments()[0] & 0xFFC0) == 0xFE80
                || v6.to_ipv4_mapped().is_some_and(|v4| is_private_ipv4(&v4))
        },
    }
}

fn validate_ssrf_ips(host: &str, ips: &[IpAddr], allowlist: &[ipnet::IpNet]) -> Result<()> {
    if ips.is_empty() {
        return Err(Error::message(format!("DNS resolution failed for {host}")));
    }

    for ip in ips {
        if is_private_ip(ip) && !is_ssrf_allowed(ip, allowlist) {
            return Err(Error::message(format!(
                "SSRF blocked: {host} resolves to private IP {ip}"
            )));
        }
    }

    Ok(())
}

/// Resolve the URL host and reject private/loopback/link-local IPs unless
/// explicitly allowlisted.
pub async fn ssrf_check(url: &Url, allowlist: &[ipnet::IpNet]) -> Result<()> {
    let host = url
        .host_str()
        .ok_or_else(|| Error::message("URL has no host"))?;

    if let Ok(ip) = host.parse::<IpAddr>() {
        return validate_ssrf_ips(host, &[ip], allowlist);
    }

    let port = url.port_or_known_default().unwrap_or(443);
    let addrs: Vec<IpAddr> = tokio::net::lookup_host(format!("{host}:{port}"))
        .await?
        .map(|socket_addr| socket_addr.ip())
        .collect();
    validate_ssrf_ips(host, &addrs, allowlist)
}

#[cfg(test)]
mod tests {
    use std::{net::IpAddr, str::FromStr};

    use {
        super::{is_private_ip, is_ssrf_allowed, ssrf_check},
        url::Url,
    };

    #[test]
    fn private_ip_v4_rules() {
        for addr in [
            "127.0.0.1",
            "192.168.1.1",
            "10.0.0.1",
            "172.16.0.1",
            "169.254.1.1",
            "0.0.0.0",
            "100.64.0.1",
            "192.0.0.1",
        ] {
            let ip =
                IpAddr::from_str(addr).unwrap_or_else(|error| panic!("valid test ip: {error}"));
            assert!(is_private_ip(&ip), "{addr} should be private");
        }

        for addr in ["8.8.8.8", "1.1.1.1"] {
            let ip =
                IpAddr::from_str(addr).unwrap_or_else(|error| panic!("valid test ip: {error}"));
            assert!(!is_private_ip(&ip), "{addr} should be public");
        }
    }

    #[test]
    fn private_ip_v6_rules() {
        for addr in [
            "::1",
            "::",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::ffff:192.168.1.1",
        ] {
            let ip =
                IpAddr::from_str(addr).unwrap_or_else(|error| panic!("valid test ip: {error}"));
            assert!(is_private_ip(&ip), "{addr} should be private");
        }

        let public = IpAddr::from_str("2607:f8b0:4004:800::200e")
            .unwrap_or_else(|error| panic!("valid test ip: {error}"));
        assert!(!is_private_ip(&public));

        let mapped_public = IpAddr::from_str("::ffff:8.8.8.8")
            .unwrap_or_else(|error| panic!("valid test ip: {error}"));
        assert!(!is_private_ip(&mapped_public));
    }

    #[test]
    fn allowlist_cidr_match() {
        let allowlist: Vec<ipnet::IpNet> = vec![
            "172.22.0.0/16"
                .parse()
                .unwrap_or_else(|error| panic!("valid cidr: {error}")),
        ];
        let ip =
            IpAddr::from_str("172.22.1.5").unwrap_or_else(|error| panic!("valid test ip: {error}"));
        assert!(is_ssrf_allowed(&ip, &allowlist));
    }

    #[tokio::test]
    async fn blocks_localhost_async() {
        let url = Url::parse("http://127.0.0.1/secret")
            .unwrap_or_else(|error| panic!("valid url: {error}"));
        let result = ssrf_check(&url, &[]).await;
        assert!(result.is_err());
        assert!(
            result
                .err()
                .unwrap_or_else(|| panic!("expected ssrf error"))
                .to_string()
                .contains("SSRF")
        );
    }

    #[tokio::test]
    async fn blocks_ipv4_mapped_localhost_async() {
        let url = Url::parse("http://[::ffff:127.0.0.1]/secret")
            .unwrap_or_else(|error| panic!("valid url: {error}"));
        let result = ssrf_check(&url, &[]).await;
        assert!(result.is_err());
        assert!(
            result
                .err()
                .unwrap_or_else(|| panic!("expected ssrf error"))
                .to_string()
                .contains("SSRF")
        );
    }
}
