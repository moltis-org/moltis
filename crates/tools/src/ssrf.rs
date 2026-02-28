use std::net::{IpAddr, ToSocketAddrs};

use {crate::error::Error, url::Url};

use crate::Result;

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
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                // 100.64.0.0/10 (CGNAT)
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64)
                // 192.0.0.0/24
                || (v4.octets()[0] == 192 && v4.octets()[1] == 0 && v4.octets()[2] == 0)
        },
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                // fc00::/7 (unique local)
                || (v6.segments()[0] & 0xFE00) == 0xFC00
                // fe80::/10 (link-local)
                || (v6.segments()[0] & 0xFFC0) == 0xFE80
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

/// SSRF protection for async callers.
///
/// Resolves the URL host and rejects private/loopback/link-local IPs unless
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

/// SSRF protection for blocking callers.
pub fn ssrf_check_blocking(url: &Url, allowlist: &[ipnet::IpNet]) -> Result<()> {
    let host = url
        .host_str()
        .ok_or_else(|| Error::message("URL has no host"))?;

    if let Ok(ip) = host.parse::<IpAddr>() {
        return validate_ssrf_ips(host, &[ip], allowlist);
    }

    let port = url.port_or_known_default().unwrap_or(443);
    let addrs: Vec<IpAddr> = (host, port)
        .to_socket_addrs()?
        .map(|socket_addr| socket_addr.ip())
        .collect();
    validate_ssrf_ips(host, &addrs, allowlist)
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, rstest::rstest};

    #[rstest]
    #[case("127.0.0.1", true)]
    #[case("192.168.1.1", true)]
    #[case("10.0.0.1", true)]
    #[case("172.16.0.1", true)]
    #[case("169.254.1.1", true)]
    #[case("0.0.0.0", true)]
    #[case("8.8.8.8", false)]
    #[case("1.1.1.1", false)]
    fn private_ip_v4(#[case] addr: &str, #[case] expected: bool) {
        let ip: IpAddr = addr.parse().unwrap();
        assert_eq!(is_private_ip(&ip), expected, "{addr}");
    }

    #[rstest]
    #[case("::1", true)]
    #[case("::", true)]
    #[case("fd00::1", true)]
    #[case("fe80::1", true)]
    #[case("2607:f8b0:4004:800::200e", false)]
    fn private_ip_v6(#[case] addr: &str, #[case] expected: bool) {
        let ip: IpAddr = addr.parse().unwrap();
        assert_eq!(is_private_ip(&ip), expected, "{addr}");
    }

    #[tokio::test]
    async fn blocks_localhost_async() {
        let url = Url::parse("http://127.0.0.1/secret").unwrap();
        let result = ssrf_check(&url, &[]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("SSRF"));
    }

    #[test]
    fn blocks_localhost_blocking() {
        let url = Url::parse("http://127.0.0.1/secret").unwrap();
        let result = ssrf_check_blocking(&url, &[]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("SSRF"));
    }

    #[test]
    fn allowlist_cidr_match() {
        let allowlist: Vec<ipnet::IpNet> = vec!["172.22.0.0/16".parse().unwrap()];
        let ip: IpAddr = "172.22.1.5".parse().unwrap();
        assert!(is_ssrf_allowed(&ip, &allowlist));
    }

    #[test]
    fn allowlist_non_match() {
        let allowlist: Vec<ipnet::IpNet> = vec!["172.22.0.0/16".parse().unwrap()];
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        assert!(!is_ssrf_allowed(&ip, &allowlist));
    }

    #[tokio::test]
    async fn allowlist_permits_private_async() {
        let allowlist: Vec<ipnet::IpNet> = vec!["172.22.0.0/16".parse().unwrap()];
        let url = Url::parse("http://172.22.1.5/api").unwrap();
        let result = ssrf_check(&url, &allowlist).await;
        assert!(result.is_ok());
    }

    #[test]
    fn allowlist_permits_private_blocking() {
        let allowlist: Vec<ipnet::IpNet> = vec!["172.22.0.0/16".parse().unwrap()];
        let url = Url::parse("http://172.22.1.5/api").unwrap();
        let result = ssrf_check_blocking(&url, &allowlist);
        assert!(result.is_ok());
    }
}
