//! Validation for user-supplied Slack Web API base URLs.
//!
//! `api_base_url` is attacker-influenceable config that the server turns into
//! outbound HTTP (and WebSocket) requests, so it is an SSRF vector. By default we
//! reject localhost and private/loopback/link-local/CGNAT/unique-local targets.
//! [`normalize_slack_api_base_url`] checks syntax and literal-IP hosts;
//! [`validate_slack_api_base_url`] additionally resolves DNS hostnames and
//! applies the same policy to every resolved address, so a name like
//! `proxy.internal` pointing at a private or metadata address is rejected too.
//!
//! Operators who deliberately front Slack with an internal proxy can opt specific
//! hosts back in via the `MOLTIS_SLACK_API_BASE_URL_ALLOWLIST` env var
//! (comma-separated exact hosts). Cloud metadata addresses stay blocked even when
//! allowlisted, since no legitimate Slack base URL resolves there.
//!
//! Known limitation: DNS is checked when the config is validated (account
//! registration / config save), not re-pinned at connect time, so a record that
//! is rebound afterwards is not re-checked on every request.

use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    sync::OnceLock,
};

use url::Url;

use crate::error::{Error, Result};

/// Env var holding a comma-separated list of exact hosts allowed to bypass the
/// private-address SSRF guard for Slack `api_base_url`.
pub const ALLOWLIST_ENV_VAR: &str = "MOLTIS_SLACK_API_BASE_URL_ALLOWLIST";

/// Normalize and validate a Slack Web API base URL, honoring the operator
/// allowlist from `MOLTIS_SLACK_API_BASE_URL_ALLOWLIST`.
///
/// Returns the trimmed URL (no trailing slash) or an `InvalidInput` error.
pub fn normalize_slack_api_base_url(api_base_url: &str) -> Result<String> {
    let trimmed = api_base_url.trim().trim_end_matches('/');
    let parsed = Url::parse(trimmed).map_err(|e| {
        Error::invalid_input(format!("Slack api_base_url must be an absolute URL: {e}"))
    })?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(Error::invalid_input(
            "Slack api_base_url must be an absolute HTTP(S) URL",
        ));
    }
    validate_host(&parsed, host_allowlist())?;
    Ok(trimmed.to_string())
}

/// Normalize and validate a Slack Web API base URL, additionally resolving DNS
/// hostnames and rejecting any that resolve to private, local, or
/// cloud-metadata addresses.
///
/// Use this at trust boundaries (account registration, config save). Hot paths
/// whose config already passed validation can use the cheaper
/// [`normalize_slack_api_base_url`]. Fails closed: a host that does not resolve
/// is rejected.
pub async fn validate_slack_api_base_url(api_base_url: &str) -> Result<String> {
    let normalized = normalize_slack_api_base_url(api_base_url)?;
    let parsed = Url::parse(&normalized).map_err(|e| {
        Error::invalid_input(format!("Slack api_base_url must be an absolute URL: {e}"))
    })?;
    let host = parsed
        .host_str()
        .ok_or_else(|| Error::invalid_input("Slack api_base_url must include a host"))?;
    let normalized_host = normalize_host(host);
    if normalized_host.parse::<IpAddr>().is_ok() {
        // Literal IPs were fully checked by `normalize_slack_api_base_url`.
        return Ok(normalized);
    }
    let port = parsed.port_or_known_default().unwrap_or(443);
    let resolved: Vec<IpAddr> = tokio::net::lookup_host((normalized_host.as_str(), port))
        .await
        .map_err(|e| {
            Error::invalid_input(format!(
                "Slack api_base_url host {normalized_host} did not resolve: {e}"
            ))
        })?
        .map(|addr| addr.ip())
        .collect();
    check_resolved_ips(&normalized_host, &resolved, host_allowlist())?;
    Ok(normalized)
}

/// The process-wide allowlist, parsed once from the environment.
fn host_allowlist() -> &'static [String] {
    static ALLOW: OnceLock<Vec<String>> = OnceLock::new();
    ALLOW.get_or_init(|| {
        std::env::var(ALLOWLIST_ENV_VAR)
            .ok()
            .map(|raw| parse_allowlist(&raw))
            .unwrap_or_default()
    })
}

/// Split a comma-separated allowlist into normalized host entries.
fn parse_allowlist(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(normalize_host)
        .filter(|host| !host.is_empty())
        .collect()
}

/// Normalize a host for case-insensitive comparison, stripping IPv6 brackets.
fn normalize_host(host: &str) -> String {
    host.trim()
        .trim_matches(&['[', ']'][..])
        .to_ascii_lowercase()
}

/// Validate the URL's host against the SSRF policy, given an allowlist.
///
/// Order matters: cloud-metadata addresses are rejected before the allowlist is
/// consulted, so an allowlisted entry can never reach the metadata service.
fn validate_host(url: &Url, allowlist: &[String]) -> Result<()> {
    let host = url
        .host_str()
        .ok_or_else(|| Error::invalid_input("Slack api_base_url must include a host"))?;
    let normalized = normalize_host(host);
    let parsed_ip = normalized.parse::<IpAddr>().ok();

    if let Some(ip) = parsed_ip
        && is_cloud_metadata_ip(&ip)
    {
        return Err(Error::invalid_input(format!(
            "Slack api_base_url must not target the cloud metadata address {ip}"
        )));
    }

    if allowlist.contains(&normalized) {
        return Ok(());
    }

    if normalized == "localhost" {
        return Err(Error::invalid_input(
            "Slack api_base_url must not target localhost",
        ));
    }
    if let Some(ip) = parsed_ip
        && is_disallowed_ip(&ip)
    {
        return Err(Error::invalid_input(format!(
            "Slack api_base_url must not target private or local IP {ip}"
        )));
    }
    Ok(())
}

/// Apply the SSRF policy to a hostname's resolved addresses.
///
/// Cloud-metadata addresses are rejected even for allowlisted hosts; private
/// and local addresses are rejected unless the host is allowlisted. An empty
/// resolution set is rejected so validation fails closed.
fn check_resolved_ips(host: &str, ips: &[IpAddr], allowlist: &[String]) -> Result<()> {
    if ips.is_empty() {
        return Err(Error::invalid_input(format!(
            "Slack api_base_url host {host} did not resolve to any address"
        )));
    }
    let host_allowlisted = allowlist.iter().any(|entry| entry == host);
    for ip in ips {
        if is_cloud_metadata_ip(ip) {
            return Err(Error::invalid_input(format!(
                "Slack api_base_url host {host} resolves to the cloud metadata address {ip}"
            )));
        }
        if !host_allowlisted && is_disallowed_ip(ip) {
            return Err(Error::invalid_input(format!(
                "Slack api_base_url host {host} resolves to private or local IP {ip}"
            )));
        }
    }
    Ok(())
}

/// Cloud metadata endpoints (AWS/GCP/Azure IMDS). Blocked unconditionally.
fn is_cloud_metadata_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => *v4 == Ipv4Addr::new(169, 254, 169, 254),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_cloud_metadata_ip(&IpAddr::V4(v4));
            }
            // AWS IMDS over IPv6.
            *v6 == Ipv6Addr::new(0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x254)
        },
    }
}

fn is_disallowed_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || is_cgnat(*v4)
        },
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_disallowed_ip(&IpAddr::V4(v4));
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || is_ipv6_unique_local(*v6)
                || is_ipv6_link_local(*v6)
        },
    }
}

fn is_cgnat(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 100 && (64..=127).contains(&octets[1])
}

fn is_ipv6_unique_local(ip: Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xfe00) == 0xfc00
}

fn is_ipv6_link_local(ip: Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xffc0) == 0xfe80
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn check(url: &str, allowlist: &[&str]) -> Result<()> {
        let allow: Vec<String> = allowlist.iter().map(|host| normalize_host(host)).collect();
        validate_host(&Url::parse(url).unwrap(), &allow)
    }

    #[test]
    fn public_host_passes_with_empty_allowlist() {
        assert!(check("https://slack.com/api", &[]).is_ok());
        assert!(check("https://proxy.example.com/api", &[]).is_ok());
    }

    #[test]
    fn localhost_and_private_ips_rejected_by_default() {
        assert!(check("http://localhost/api", &[]).is_err());
        assert!(check("http://127.0.0.1/api", &[]).is_err());
        assert!(check("http://10.0.0.5/api", &[]).is_err());
        assert!(check("http://192.168.1.1/api", &[]).is_err());
        assert!(check("http://100.64.0.1/api", &[]).is_err());
        assert!(check("http://[fc00::1]/api", &[]).is_err());
        assert!(check("http://[fe80::1]/api", &[]).is_err());
    }

    #[test]
    fn allowlisted_hosts_pass() {
        assert!(check("http://localhost/api", &["localhost"]).is_ok());
        assert!(check("http://127.0.0.1:8080/api", &["127.0.0.1"]).is_ok());
        assert!(check("http://proxy.internal/api", &["proxy.internal"]).is_ok());
        assert!(check("http://[::1]/api", &["::1"]).is_ok());
    }

    #[test]
    fn allowlist_is_case_insensitive_and_host_scoped() {
        assert!(check("http://Proxy.Internal/api", &["proxy.internal"]).is_ok());
        // Allowlisting one private host does not open others.
        assert!(check("http://10.0.0.9/api", &["127.0.0.1"]).is_err());
    }

    #[test]
    fn cloud_metadata_blocked_even_when_allowlisted() {
        assert!(check("http://169.254.169.254/api", &["169.254.169.254"]).is_err());
        assert!(check("http://[fd00:ec2::254]/api", &["fd00:ec2::254"]).is_err());
    }

    #[test]
    fn normalize_trims_trailing_slash_and_whitespace() {
        assert_eq!(
            normalize_slack_api_base_url("  https://slack.com/api/  ").unwrap(),
            "https://slack.com/api"
        );
    }

    #[test]
    fn normalize_rejects_non_http_scheme() {
        assert!(normalize_slack_api_base_url("ftp://slack.com/api").is_err());
        assert!(normalize_slack_api_base_url("not-a-url").is_err());
    }

    fn check_resolved(host: &str, ips: &[&str], allowlist: &[&str]) -> Result<()> {
        let ips: Vec<IpAddr> = ips.iter().map(|ip| ip.parse().unwrap()).collect();
        let allow: Vec<String> = allowlist
            .iter()
            .map(|entry| normalize_host(entry))
            .collect();
        check_resolved_ips(host, &ips, &allow)
    }

    #[test]
    fn hostname_resolving_to_private_ip_rejected_by_default() {
        assert!(check_resolved("proxy.internal", &["127.0.0.1"], &[]).is_err());
        assert!(check_resolved("proxy.internal", &["10.0.0.5"], &[]).is_err());
        assert!(check_resolved("proxy.internal", &["fe80::1"], &[]).is_err());
        // One private address among public ones still rejects.
        assert!(check_resolved("proxy.internal", &["93.184.216.34", "192.168.1.1"], &[]).is_err());
    }

    #[test]
    fn hostname_resolving_to_public_ips_passes() {
        assert!(check_resolved("slack.com", &["3.122.0.1", "2600:1f18::1"], &[]).is_ok());
    }

    #[test]
    fn allowlisted_hostname_may_resolve_to_private_ip() {
        assert!(check_resolved("proxy.internal", &["10.0.0.5"], &["proxy.internal"]).is_ok());
        // Allowlisting one host does not open others.
        assert!(check_resolved("other.internal", &["10.0.0.5"], &["proxy.internal"]).is_err());
    }

    #[test]
    fn hostname_resolving_to_metadata_rejected_even_when_allowlisted() {
        assert!(
            check_resolved("proxy.internal", &["169.254.169.254"], &["proxy.internal"]).is_err()
        );
        assert!(check_resolved("proxy.internal", &["fd00:ec2::254"], &["proxy.internal"]).is_err());
        assert!(
            check_resolved("proxy.internal", &["::ffff:169.254.169.254"], &[
                "proxy.internal"
            ])
            .is_err()
        );
    }

    #[test]
    fn empty_resolution_fails_closed() {
        assert!(check_resolved("proxy.internal", &[], &[]).is_err());
        assert!(check_resolved("proxy.internal", &[], &["proxy.internal"]).is_err());
    }

    #[tokio::test]
    async fn validate_rejects_literal_private_ips_without_dns() {
        assert!(
            validate_slack_api_base_url("http://127.0.0.1/api")
                .await
                .is_err()
        );
        assert!(
            validate_slack_api_base_url("http://169.254.169.254/api")
                .await
                .is_err()
        );
        assert!(
            validate_slack_api_base_url("http://localhost/api")
                .await
                .is_err()
        );
    }
}
