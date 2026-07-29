use {
    async_trait::async_trait,
    futures::AsyncReadExt,
    isahc::{
        HttpClient,
        config::{RedirectPolicy, ResolveMap},
        http::{HeaderMap, StatusCode, Uri, header::RETRY_AFTER},
        prelude::Configurable,
    },
    std::{
        net::IpAddr,
        time::{Duration, SystemTime},
    },
    web_push::{WebPushClient, WebPushError, WebPushMessage, request_builder},
};

const MAX_RESPONSE_SIZE: usize = 64 * 1024;

pub(super) struct PinnedWebPushClient;

struct ResolvedEndpoint {
    host: String,
    port: u16,
    addresses: Vec<IpAddr>,
}

async fn resolve_endpoint(endpoint: &Uri) -> Result<ResolvedEndpoint, WebPushError> {
    if endpoint.scheme_str() != Some("https") {
        return Err(WebPushError::InvalidUri);
    }
    let host = endpoint
        .host()
        .map(|host| host.trim_matches(['[', ']']).to_ascii_lowercase())
        .filter(|host| !host.is_empty())
        .ok_or(WebPushError::InvalidUri)?;
    let port = endpoint.port_u16().unwrap_or(443);
    let addresses = if let Ok(ip) = host.parse::<IpAddr>() {
        vec![ip]
    } else {
        tokio::net::lookup_host((host.as_str(), port))
            .await
            .map_err(|_| WebPushError::InvalidUri)?
            .map(|address| address.ip())
            .collect()
    };
    if addresses.is_empty() || addresses.iter().any(moltis_common::ssrf::is_private_ip) {
        return Err(WebPushError::InvalidUri);
    }

    Ok(ResolvedEndpoint {
        host,
        port,
        addresses,
    })
}

fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let value = headers.get(RETRY_AFTER)?.to_str().ok()?;
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }

    let date = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    let retry_at: SystemTime = date.into();
    retry_at.duration_since(SystemTime::now()).ok()
}

fn parse_response(
    status: StatusCode,
    headers: &HeaderMap,
    body: Vec<u8>,
) -> Result<(), WebPushError> {
    let parsed = request_builder::parse_response(status, body);
    if let Err(WebPushError::ServerError {
        retry_after: None,
        info,
    }) = parsed
    {
        Err(WebPushError::ServerError {
            retry_after: retry_after(headers),
            info,
        })
    } else {
        parsed
    }
}

#[cfg(test)]
pub(super) fn expired_endpoint_error() -> WebPushError {
    match parse_response(StatusCode::GONE, &HeaderMap::new(), Vec::new()) {
        Err(error) => error,
        Ok(()) => WebPushError::Unspecified,
    }
}

pub(super) fn endpoint_identifier(endpoint: &str) -> String {
    use {base64::Engine, sha2::Digest};

    let digest = sha2::Sha256::digest(endpoint.as_bytes());
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
    format!("endpoint#{}", &encoded[..10])
}

#[async_trait]
impl WebPushClient for PinnedWebPushClient {
    async fn send(&self, message: WebPushMessage) -> Result<(), WebPushError> {
        let resolved = resolve_endpoint(&message.endpoint).await?;
        let resolve_map = resolved
            .addresses
            .iter()
            .fold(ResolveMap::new(), |map, address| {
                map.add(&resolved.host, resolved.port, *address)
            });
        let client = HttpClient::builder()
            .dns_resolve(resolve_map)
            .redirect_policy(RedirectPolicy::None)
            .proxy(None)
            .build()?;
        let request = request_builder::build_request::<isahc::AsyncBody>(message);
        let response = client.send_async(request).await?;
        let status = response.status();
        let headers = response.headers().clone();
        let mut body = Vec::new();
        if response
            .into_body()
            .take(MAX_RESPONSE_SIZE as u64 + 1)
            .read_to_end(&mut body)
            .await?
            > MAX_RESPONSE_SIZE
        {
            return Err(WebPushError::ResponseTooLarge);
        }
        parse_response(status, &headers, body)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn global_literal_is_resolved_for_exact_pinning() {
        let endpoint: Uri = "https://8.8.8.8/push".parse().unwrap();
        let resolved = resolve_endpoint(&endpoint).await.unwrap();
        assert_eq!(resolved.host, "8.8.8.8");
        assert_eq!(resolved.port, 443);
        assert_eq!(resolved.addresses, vec![
            "8.8.8.8".parse::<IpAddr>().unwrap()
        ]);
    }

    #[tokio::test]
    async fn non_global_literals_are_rejected_before_request_building() {
        for endpoint in [
            "https://127.0.0.1/push",
            "https://10.0.0.1/push",
            "https://169.254.169.254/push",
            "https://198.51.100.1/push",
            "https://224.0.0.1/push",
            "https://[::1]/push",
            "https://[2001:db8::1]/push",
            "https://[ff0e::1]/push",
        ] {
            let endpoint: Uri = endpoint.parse().unwrap();
            assert!(resolve_endpoint(&endpoint).await.is_err(), "{endpoint}");
        }
    }

    #[tokio::test]
    async fn resolved_non_global_hostname_is_rejected() {
        let endpoint: Uri = "https://localhost/push".parse().unwrap();
        assert!(resolve_endpoint(&endpoint).await.is_err());
    }

    #[test]
    fn web_push_response_parser_preserves_typed_expiry() {
        assert!(matches!(
            parse_response(StatusCode::GONE, &HeaderMap::new(), Vec::new()),
            Err(WebPushError::EndpointNotValid(_))
        ));
    }

    #[test]
    fn web_push_response_parser_preserves_retry_after() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, "12".parse().unwrap());
        assert!(matches!(
            parse_response(StatusCode::SERVICE_UNAVAILABLE, &headers, Vec::new()),
            Err(WebPushError::ServerError {
                retry_after: Some(duration),
                ..
            }) if duration == Duration::from_secs(12)
        ));
    }

    #[test]
    fn endpoint_identifier_omits_capability_path_and_token() {
        let identifier = endpoint_identifier(
            "https://push.example.test/private/capability?token=super-secret-token",
        );
        assert!(identifier.starts_with("endpoint#"));
        assert_eq!(identifier.len(), 19);
        assert!(!identifier.contains("push.example.test"));
        assert!(!identifier.contains("private"));
        assert!(!identifier.contains("super-secret-token"));
        assert!(!identifier.contains("token="));
        assert_eq!(
            identifier,
            endpoint_identifier(
                "https://push.example.test/private/capability?token=super-secret-token"
            )
        );
        assert_ne!(
            identifier,
            endpoint_identifier(
                "https://push.example.test/private/capability?token=different-token"
            )
        );
    }
}
