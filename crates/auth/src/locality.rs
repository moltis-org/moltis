//! Connection locality detection for auth decisions.
//!
//! Determines whether a request is a direct local connection (no proxy)
//! based on headers, remote address, and proxy configuration.

use std::net::SocketAddr;

/// Returns `true` when common proxy/forwarding headers are present.
pub fn has_proxy_headers(headers: &axum::http::HeaderMap) -> bool {
    headers.contains_key("x-forwarded-for")
        || headers.contains_key("x-real-ip")
        || headers.contains_key("cf-connecting-ip")
        || headers.get("forwarded").is_some()
}

/// Returns `true` when `host` (without port) is a loopback name/address.
fn is_loopback_host(host: &str) -> bool {
    // Strip port (IPv6 bracket form, bare IPv6, or simple host:port).
    let name = if host.starts_with('[') {
        // [::1]:port or [::1]
        host.rsplit_once("]:")
            .map_or(host, |(addr, _)| addr)
            .trim_start_matches('[')
            .trim_end_matches(']')
    } else if host.matches(':').count() > 1 {
        // Bare IPv6 like ::1 (multiple colons, no brackets) — no port stripping.
        host
    } else {
        host.rsplit_once(':').map_or(host, |(addr, _)| addr)
    };
    matches!(name, "localhost" | "127.0.0.1" | "::1") || name.ends_with(".localhost")
}

/// Determine whether a connection is a **direct local** connection (no proxy
/// in between).
///
/// When `behind_proxy` is `true`, the caller is known to be behind a reverse
/// proxy, so loopback source IPs are never treated as proof of a direct local
/// connection.
pub fn is_local_connection(
    headers: &axum::http::HeaderMap,
    remote_addr: SocketAddr,
    behind_proxy: bool,
) -> bool {
    // Hard override: env var says we're behind a proxy.
    if behind_proxy {
        return false;
    }

    // Proxy headers present → proxied traffic.
    if has_proxy_headers(headers) {
        return false;
    }

    // Host header points to a non-loopback name → likely proxied.
    if let Some(host) = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        && !is_loopback_host(host)
    {
        return false;
    }

    // TCP source must be loopback.
    remote_addr.ip().is_loopback()
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_proxy_headers_detects_xff() {
        let mut h = axum::http::HeaderMap::new();
        h.insert("x-forwarded-for", "203.0.113.50".parse().unwrap());
        assert!(has_proxy_headers(&h));
    }

    #[test]
    fn has_proxy_headers_detects_x_real_ip() {
        let mut h = axum::http::HeaderMap::new();
        h.insert("x-real-ip", "203.0.113.50".parse().unwrap());
        assert!(has_proxy_headers(&h));
    }

    #[test]
    fn has_proxy_headers_detects_cf_connecting_ip() {
        let mut h = axum::http::HeaderMap::new();
        h.insert("cf-connecting-ip", "203.0.113.50".parse().unwrap());
        assert!(has_proxy_headers(&h));
    }

    #[test]
    fn has_proxy_headers_detects_forwarded() {
        let mut h = axum::http::HeaderMap::new();
        h.insert("forwarded", "for=203.0.113.50".parse().unwrap());
        assert!(has_proxy_headers(&h));
    }

    #[test]
    fn has_proxy_headers_empty() {
        assert!(!has_proxy_headers(&axum::http::HeaderMap::new()));
    }

    #[test]
    fn is_loopback_host_variants() {
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("localhost:18789"));
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("127.0.0.1:18789"));
        assert!(is_loopback_host("::1"));
        assert!(is_loopback_host("[::1]:18789"));
        assert!(is_loopback_host("moltis.localhost"));
        assert!(is_loopback_host("moltis.localhost:8080"));

        assert!(!is_loopback_host("example.com"));
        assert!(!is_loopback_host("example.com:18789"));
        assert!(!is_loopback_host("192.168.1.1:18789"));
        assert!(!is_loopback_host("moltis.example.com"));
    }

    #[test]
    fn local_connection_direct_loopback() {
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::HOST, "localhost:18789".parse().unwrap());
        assert!(is_local_connection(&headers, addr, false));
    }

    #[test]
    fn local_connection_with_proxy_headers() {
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::HOST, "localhost:18789".parse().unwrap());
        headers.insert("x-forwarded-for", "203.0.113.50".parse().unwrap());
        assert!(!is_local_connection(&headers, addr, false));
    }

    #[test]
    fn local_connection_behind_proxy_override() {
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::HOST, "localhost:18789".parse().unwrap());
        assert!(!is_local_connection(&headers, addr, true));
    }

    #[test]
    fn non_loopback_addr_not_local() {
        let addr: SocketAddr = "192.168.1.1:12345".parse().unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::HOST, "localhost:18789".parse().unwrap());
        assert!(!is_local_connection(&headers, addr, false));
    }

    #[test]
    fn local_connection_ipv6_loopback() {
        let addr: SocketAddr = "[::1]:12345".parse().unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::HOST, "[::1]:18789".parse().unwrap());
        assert!(is_local_connection(&headers, addr, false));
    }

    #[test]
    fn local_connection_no_host_header() {
        // CLI/SDK clients may not send a Host header.
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let headers = axum::http::HeaderMap::new();
        assert!(is_local_connection(&headers, addr, false));
    }

    #[test]
    fn not_local_when_xff_spoofs_loopback_value() {
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::HOST, "localhost:18789".parse().unwrap());
        // Header presence alone marks the request as proxied, even when value
        // is spoofed to look loopback.
        headers.insert("x-forwarded-for", "127.0.0.1".parse().unwrap());
        assert!(!is_local_connection(&headers, addr, false));
    }

    #[test]
    fn not_local_when_forwarded_spoofs_loopback_value() {
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::HOST, "localhost:18789".parse().unwrap());
        // RFC 7239 Forwarded header should never allow localhost bypass.
        headers.insert("forwarded", "for=127.0.0.1;proto=https".parse().unwrap());
        assert!(!is_local_connection(&headers, addr, false));
    }

    #[test]
    fn not_local_when_host_is_external() {
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::HOST,
            "moltis.example.com".parse().unwrap(),
        );
        assert!(!is_local_connection(&headers, addr, false));
    }

    /// Simulates a reverse proxy (Caddy/nginx) on the same machine that
    /// does NOT add proxy headers (bare nginx `proxy_pass`). The Host header
    /// is rewritten to the upstream (loopback) address and the TCP source is
    /// loopback. Without `MOLTIS_BEHIND_PROXY`, this would appear local.
    #[test]
    fn bare_proxy_without_env_var_appears_local() {
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::HOST, "127.0.0.1:18789".parse().unwrap());
        // This is the known limitation: bare proxy looks like local.
        assert!(is_local_connection(&headers, addr, false));
        // Setting MOLTIS_BEHIND_PROXY fixes it.
        assert!(!is_local_connection(&headers, addr, true));
    }

    /// Typical Caddy/nginx with proper headers: loopback TCP but
    /// X-Forwarded-For reveals the real client IP.
    #[test]
    fn proxy_with_xff_detected() {
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::HOST,
            "moltis.example.com".parse().unwrap(),
        );
        headers.insert("x-forwarded-for", "203.0.113.50".parse().unwrap());
        assert!(!is_local_connection(&headers, addr, false));
    }

    /// Proxy that rewrites Host to a public domain (but no XFF).
    #[test]
    fn proxy_detected_via_host_header() {
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::HOST,
            "moltis.example.com".parse().unwrap(),
        );
        assert!(!is_local_connection(&headers, addr, false));
    }
}
