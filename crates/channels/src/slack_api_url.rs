//! Validation for user-supplied Slack Web API base URLs.
//!
//! `api_base_url` is attacker-influenceable config that the server turns into
//! outbound HTTP (and WebSocket) requests, so it is an SSRF vector. By default we
//! reject localhost and private/loopback/link-local/CGNAT/unique-local targets.
//!
//! Operators who deliberately front Slack with an internal proxy can opt specific
//! hosts back in via the `MOLTIS_SLACK_API_BASE_URL_ALLOWLIST` env var
//! (comma-separated exact hosts). Cloud metadata addresses stay blocked even when
//! allowlisted, since no legitimate Slack base URL resolves there.

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
}
