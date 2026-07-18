use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use slack_morphism::prelude::*;

use moltis_channels::{Error as ChannelError, Result as ChannelResult};

pub const DEFAULT_SLACK_API_BASE_URL: &str = "https://slack.com/api";

pub fn normalize_slack_api_base_url(api_base_url: &str) -> ChannelResult<String> {
    let trimmed = api_base_url.trim().trim_end_matches('/');
    let parsed = reqwest::Url::parse(trimmed).map_err(|e| {
        ChannelError::invalid_input(format!("Slack api_base_url must be an absolute URL: {e}"))
    })?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(ChannelError::invalid_input(
            "Slack api_base_url must be an absolute HTTP(S) URL",
        ));
    }
    validate_public_host(&parsed)?;
    Ok(trimmed.to_string())
}

fn validate_public_host(url: &reqwest::Url) -> ChannelResult<()> {
    let host = url
        .host_str()
        .ok_or_else(|| ChannelError::invalid_input("Slack api_base_url must include a host"))?;
    if host.eq_ignore_ascii_case("localhost") {
        return Err(ChannelError::invalid_input(
            "Slack api_base_url must not target localhost",
        ));
    }
    if let Ok(ip) = host.trim_matches(&['[', ']'][..]).parse::<IpAddr>()
        && is_disallowed_ip(&ip)
    {
        return Err(ChannelError::invalid_input(format!(
            "Slack api_base_url must not target private or local IP {ip}"
        )));
    }
    Ok(())
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

pub fn slack_client_for_base_url(
    api_base_url: &str,
) -> ChannelResult<SlackClient<SlackClientHyperHttpsConnector>> {
    let api_base_url = normalize_slack_api_base_url(api_base_url)?;
    let connector = SlackClientHyperConnector::new()
        .map_err(|e| ChannelError::unavailable(format!("hyper connector: {e}")))?
        .with_slack_api_url(&api_base_url);
    Ok(SlackClient::new(connector))
}

pub fn slack_api_method_url(api_base_url: &str, method: &str) -> ChannelResult<String> {
    Ok(format!(
        "{}/{}",
        normalize_slack_api_base_url(api_base_url)?,
        method.trim_start_matches('/')
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_trailing_slashes() {
        assert_eq!(
            normalize_slack_api_base_url("https://proxy.example/api/").unwrap(),
            "https://proxy.example/api"
        );
    }

    #[test]
    fn rejects_relative_base_urls() {
        assert!(normalize_slack_api_base_url("/api").is_err());
    }

    #[test]
    fn rejects_localhost_base_urls() {
        assert!(normalize_slack_api_base_url("http://localhost:3000/api").is_err());
    }

    #[test]
    fn rejects_private_ip_base_urls() {
        assert!(normalize_slack_api_base_url("http://169.254.169.254/api").is_err());
        assert!(normalize_slack_api_base_url("http://10.0.0.1/api").is_err());
        assert!(normalize_slack_api_base_url("http://[fd00::1]/api").is_err());
        assert!(normalize_slack_api_base_url("http://[::ffff:169.254.169.254]/api").is_err());
    }

    #[test]
    fn builds_method_url_from_default_base() {
        assert_eq!(
            slack_api_method_url(DEFAULT_SLACK_API_BASE_URL, "chat.startStream").unwrap(),
            "https://slack.com/api/chat.startStream"
        );
    }

    #[test]
    fn builds_method_url_from_trailing_slash_base() {
        assert_eq!(
            slack_api_method_url("https://proxy.example/api/", "chat.startStream").unwrap(),
            "https://proxy.example/api/chat.startStream"
        );
    }
}
