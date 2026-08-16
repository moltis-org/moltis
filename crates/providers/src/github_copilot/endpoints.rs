use tracing::{debug, warn};

const INDIVIDUAL_API_HOSTS: &[&str] =
    &["api.githubcopilot.com", "api.individual.githubcopilot.com"];
const ALLOWED_API_HOST: &str = "githubcopilot.com";
const ALLOWED_API_HOST_SUFFIX: &str = ".githubcopilot.com";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CopilotEndpoint {
    pub(super) base_url: String,
    pub(super) is_enterprise: bool,
    pub(super) cache_value: String,
}

pub(super) fn endpoint_from_metadata(
    api_base: Option<&str>,
    proxy_ep: Option<&str>,
) -> Option<CopilotEndpoint> {
    api_base
        .and_then(endpoint_from_api_base)
        .or_else(|| proxy_ep.and_then(endpoint_from_proxy_ep))
}

pub(super) fn endpoint_from_cached_metadata(value: Option<&str>) -> Option<CopilotEndpoint> {
    let value = value.map(str::trim).filter(|value| !value.is_empty())?;
    if value.contains("://") {
        return endpoint_from_api_base(value);
    }
    endpoint_from_proxy_ep(value)
}

fn endpoint_from_api_base(value: &str) -> Option<CopilotEndpoint> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    let Ok(url) = reqwest::Url::parse(value) else {
        warn!(api_base = %value, "ignoring malformed github-copilot API endpoint");
        return None;
    };
    if url.scheme() != "https" {
        warn!(api_base = %value, "ignoring non-https github-copilot API endpoint");
        return None;
    }
    if !url.username().is_empty() || url.password().is_some() || url.port().is_some() {
        warn!(api_base = %value, "ignoring github-copilot API endpoint with credentials or port");
        return None;
    }
    if !matches!(url.path(), "" | "/") || url.query().is_some() || url.fragment().is_some() {
        warn!(api_base = %value, "ignoring github-copilot API endpoint with path/query/fragment");
        return None;
    }

    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
        warn!(api_base = %value, "ignoring github-copilot API endpoint without host");
        return None;
    };
    if host != ALLOWED_API_HOST && !host.ends_with(ALLOWED_API_HOST_SUFFIX) {
        warn!(api_base_host = %host, "ignoring untrusted github-copilot API endpoint host");
        return None;
    }

    let base_url = format!("https://{host}");
    debug!(api_base = %base_url, "using github-copilot API endpoint from token metadata");
    Some(CopilotEndpoint {
        is_enterprise: !INDIVIDUAL_API_HOSTS.contains(&host.as_str()),
        cache_value: base_url.clone(),
        base_url,
    })
}

fn endpoint_from_proxy_ep(proxy_ep: &str) -> Option<CopilotEndpoint> {
    let ep = proxy_ep.trim();
    if ep.is_empty() {
        return None;
    }
    if !ep
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
    {
        warn!(proxy_ep = %ep, "ignoring malformed github-copilot proxy-ep");
        return None;
    }
    if ep.parse::<std::net::IpAddr>().is_ok() {
        warn!(proxy_ep = %ep, "ignoring IP-address github-copilot proxy-ep");
        return None;
    }
    let host = ep.to_ascii_lowercase();
    if host != ALLOWED_API_HOST && !host.ends_with(ALLOWED_API_HOST_SUFFIX) {
        warn!(proxy_ep_host = %host, "ignoring untrusted github-copilot proxy-ep host");
        return None;
    }

    debug!(proxy_ep = %host, "using github-copilot enterprise proxy endpoint");
    Some(CopilotEndpoint {
        base_url: format!("https://{host}"),
        is_enterprise: true,
        cache_value: host,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_metadata_prefers_enterprise_api_base() {
        let endpoint = endpoint_from_metadata(
            Some("https://api.enterprise.githubcopilot.com/"),
            Some("proxy.enterprise.githubcopilot.com"),
        );
        let Some(endpoint) = endpoint else {
            panic!("endpoint should parse");
        };

        assert_eq!(
            endpoint.base_url,
            "https://api.enterprise.githubcopilot.com"
        );
        assert!(endpoint.is_enterprise);
        assert_eq!(endpoint.cache_value, endpoint.base_url);
    }

    #[test]
    fn individual_api_base_is_not_enterprise() {
        let endpoint = endpoint_from_metadata(Some("https://api.githubcopilot.com"), None);
        let Some(endpoint) = endpoint else {
            panic!("endpoint should parse");
        };

        assert_eq!(endpoint.base_url, "https://api.githubcopilot.com");
        assert!(!endpoint.is_enterprise);
    }

    #[test]
    fn invalid_api_base_falls_back_to_proxy_ep() {
        let endpoint = endpoint_from_metadata(
            Some("https://example.com"),
            Some("proxy.enterprise.githubcopilot.com"),
        );
        let Some(endpoint) = endpoint else {
            panic!("proxy endpoint should parse");
        };

        assert_eq!(
            endpoint.base_url,
            "https://proxy.enterprise.githubcopilot.com"
        );
        assert!(endpoint.is_enterprise);
    }

    #[test]
    fn untrusted_proxy_ep_is_rejected() {
        assert!(endpoint_from_metadata(None, Some("proxy.example.com")).is_none());
    }

    #[test]
    fn cached_metadata_supports_api_base_and_legacy_proxy_ep() {
        let api_endpoint =
            endpoint_from_cached_metadata(Some("https://api.enterprise.githubcopilot.com"));
        let Some(api_endpoint) = api_endpoint else {
            panic!("api endpoint should parse");
        };
        let proxy_endpoint =
            endpoint_from_cached_metadata(Some("proxy.enterprise.githubcopilot.com"));
        let Some(proxy_endpoint) = proxy_endpoint else {
            panic!("proxy endpoint should parse");
        };

        assert_eq!(
            api_endpoint.base_url,
            "https://api.enterprise.githubcopilot.com"
        );
        assert_eq!(
            proxy_endpoint.base_url,
            "https://proxy.enterprise.githubcopilot.com"
        );
        assert!(api_endpoint.is_enterprise);
        assert!(proxy_endpoint.is_enterprise);
    }

    #[test]
    fn missing_cached_metadata_is_invalid() {
        assert!(endpoint_from_cached_metadata(None).is_none());
        assert!(endpoint_from_cached_metadata(Some("   ")).is_none());
    }
}
