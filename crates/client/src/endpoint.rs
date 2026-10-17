//! Where a daemon listens and how to trust its TLS certificate.

use {
    serde::{Deserialize, Serialize},
    url::Url,
};

use crate::{
    error::{Error, Result},
    tls::TlsTrust,
};

/// Path of the WebSocket RPC endpoint.
pub const WS_PATH: &str = "/ws/chat";

/// A daemon's base URL plus the TLS trust used to reach it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    base: Url,
    #[serde(default)]
    pub tls: TlsTrust,
}

impl Endpoint {
    /// Parse `https://host:port`, `http://host:port` or a bare `host:port`
    /// (treated as `http`). Paths, queries and fragments are dropped.
    pub fn parse(input: &str, tls: TlsTrust) -> Result<Self> {
        let trimmed = input.trim();
        if trimmed.trim_matches('/').is_empty() {
            return Err(Error::InvalidUrl("empty URL".into()));
        }
        let with_scheme = if trimmed.contains("://") {
            trimmed.to_string()
        } else {
            format!("http://{trimmed}")
        };
        let mut base =
            Url::parse(&with_scheme).map_err(|e| Error::InvalidUrl(format!("{input}: {e}")))?;
        if !matches!(base.scheme(), "http" | "https") {
            return Err(Error::InvalidUrl(format!(
                "{input}: scheme must be http or https"
            )));
        }
        if base.host_str().is_none_or(str::is_empty) {
            return Err(Error::InvalidUrl(format!("{input}: missing host")));
        }
        base.set_path("/");
        base.set_query(None);
        base.set_fragment(None);
        Ok(Self { base, tls })
    }

    /// The `http(s)://host:port/` base.
    #[must_use]
    pub fn base_url(&self) -> &Url {
        &self.base
    }

    #[must_use]
    pub fn is_tls(&self) -> bool {
        self.base.scheme() == "https"
    }

    /// Whether the host is this machine (`localhost`, `127.0.0.0/8`, `::1`).
    #[must_use]
    pub fn is_loopback(&self) -> bool {
        match self.base.host() {
            Some(url::Host::Domain(domain)) => {
                domain.eq_ignore_ascii_case("localhost") || domain.ends_with(".localhost")
            },
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            None => false,
        }
    }

    /// Whether credentials may be sent: over TLS, or to this machine (which
    /// includes SSH tunnels, encrypted by ssh).
    #[must_use]
    pub fn protects_credentials(&self) -> bool {
        self.is_tls() || self.is_loopback()
    }

    #[must_use]
    pub fn host(&self) -> &str {
        self.base.host_str().unwrap_or_default()
    }

    #[must_use]
    pub fn port(&self) -> u16 {
        self.base
            .port_or_known_default()
            .unwrap_or(if self.is_tls() {
                443
            } else {
                80
            })
    }

    /// `ws(s)://host:port/ws/chat`.
    #[must_use]
    pub fn ws_url(&self) -> Url {
        let mut url = self.base.clone();
        // Only http/https bases exist, and both map to a valid ws scheme.
        let _ = url.set_scheme(if self.is_tls() {
            "wss"
        } else {
            "ws"
        });
        url.set_path(WS_PATH);
        url
    }

    /// An HTTP URL under the base, e.g. `/api/auth/status`.
    #[must_use]
    pub fn http_url(&self, path: &str) -> Url {
        let mut url = self.base.clone();
        url.set_path(path);
        url
    }

    /// Same endpoint, different TLS trust.
    #[must_use]
    pub fn with_tls(mut self, tls: TlsTrust) -> Self {
        self.tls = tls;
        self
    }
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.base.as_str().trim_end_matches('/'))
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &str) -> Endpoint {
        Endpoint::parse(input, TlsTrust::System).unwrap()
    }

    #[test]
    fn ws_urls() {
        assert_eq!(
            parse("http://localhost:8080").ws_url().as_str(),
            "ws://localhost:8080/ws/chat"
        );
        assert_eq!(
            parse("https://moltis.example.com/").ws_url().as_str(),
            "wss://moltis.example.com/ws/chat"
        );
        assert_eq!(
            parse("192.168.1.1:8080").ws_url().as_str(),
            "ws://192.168.1.1:8080/ws/chat"
        );
        assert_eq!(
            parse("https://[::1]:9000/some/path?x=1").ws_url().as_str(),
            "wss://[::1]:9000/ws/chat"
        );
    }

    #[test]
    fn http_urls_and_parts() {
        let endpoint = parse("https://host:18789");
        assert_eq!(
            endpoint.http_url("/api/auth/status").as_str(),
            "https://host:18789/api/auth/status"
        );
        assert_eq!(endpoint.host(), "host");
        assert_eq!(endpoint.port(), 18789);
        assert!(endpoint.is_tls());
        assert_eq!(endpoint.to_string(), "https://host:18789");
        assert_eq!(parse("https://host").port(), 443);
    }

    #[test]
    fn loopback_and_credential_safety() {
        for url in [
            "http://localhost:1",
            "http://127.0.0.1:1",
            "http://[::1]:1",
            "http://a.localhost",
        ] {
            assert!(parse(url).is_loopback(), "{url}");
            assert!(parse(url).protects_credentials(), "{url}");
        }
        assert!(!parse("http://192.168.1.2:1").is_loopback());
        assert!(!parse("http://studio.lan:1").protects_credentials());
        assert!(parse("https://studio.lan:1").protects_credentials());
    }

    #[test]
    fn rejects_bad_input() {
        for input in ["", "   ", "ftp://host", "http://"] {
            assert!(
                Endpoint::parse(input, TlsTrust::System).is_err(),
                "{input:?} should be rejected"
            );
        }
    }

    #[test]
    fn serde_round_trip() {
        let endpoint = parse("https://h:1").with_tls(TlsTrust::Fingerprint {
            sha256: "ab".into(),
        });
        let json = serde_json::to_string(&endpoint).unwrap();
        assert_eq!(serde_json::from_str::<Endpoint>(&json).unwrap(), endpoint);
    }
}
