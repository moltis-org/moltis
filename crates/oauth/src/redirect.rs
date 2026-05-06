//! Redirect URI normalization helpers.
//!
//! Centralizes the RFC 8252 §7.3/§8.3 loopback rewrite so both MCP OAuth
//! (RFC 7591 dynamic client registration) and provider OAuth produce
//! authorization-server-compatible URIs regardless of the scheme the
//! browser started on.

use {
    tracing::debug,
    url::{Host, Url},
};

/// Rewrite `https://` → `http://` for loopback redirect URIs.
///
/// RFC 8252 §7.3/§8.3 mandate that loopback redirect URIs use the `http`
/// scheme. Strict authorization servers (e.g. Attio) reject
/// `https://localhost` during RFC 7591 dynamic client registration with
/// `invalid_redirect_uri`, and some also reject it during the authorization
/// request. Moltis serves the web UI over TLS, so the origin-derived
/// callback arrives as `https://localhost:<port>/auth/callback`.
///
/// The main TLS listener's peek-based HTTP→HTTPS redirect (see
/// `moltis_tls::serve_tls_with_http_redirect`) transparently bounces the
/// plain-HTTP OAuth callback back onto the real HTTPS callback handler,
/// preserving path and query string, so this rewrite is safe for any
/// deployment where the web UI is reached via a loopback host
/// (`localhost`, `127.0.0.1`, `::1`).
///
/// Non-loopback hosts (`moltis.lan`, real TLS hostnames) and non-`https`
/// schemes are returned unchanged — a confidential-client HTTPS redirect
/// on a real hostname is a perfectly valid web-app redirect URI and is
/// unaffected by §8.3.
#[must_use]
pub fn normalize_loopback_redirect(uri: &str) -> String {
    let Ok(mut parsed) = Url::parse(uri) else {
        return uri.to_string();
    };
    if parsed.scheme() != "https" {
        return uri.to_string();
    }
    // `https` is a "special" scheme in the WHATWG URL spec, so any Url
    // with `scheme() == "https"` is guaranteed to have a host. Using
    // `is_some_and` keeps the `None` branch in the std library rather
    // than introducing an unreachable arm in our own code.
    let is_loopback = parsed.host().is_some_and(|host| match host {
        Host::Domain(domain) => domain.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(ip) => ip.is_loopback(),
        Host::Ipv6(ip) => ip.is_loopback(),
    });
    if !is_loopback {
        return uri.to_string();
    }
    // `set_scheme` only fails when changing between "special" and
    // non-"special" schemes; `https`→`http` are both special and share
    // the same structure, so this never fails in practice.
    let _ = parsed.set_scheme("http");
    let normalized = parsed.to_string();
    debug!(
        original = %uri,
        normalized = %normalized,
        "rewrote loopback OAuth redirect URI from https to http (RFC 8252 §7.3/§8.3)"
    );
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_https_localhost() {
        assert_eq!(
            normalize_loopback_redirect("https://localhost:1455/auth/callback"),
            "http://localhost:1455/auth/callback",
        );
    }

    #[test]
    fn rewrites_https_localhost_default_port() {
        assert_eq!(
            normalize_loopback_redirect("https://localhost/auth/callback"),
            "http://localhost/auth/callback",
        );
    }

    #[test]
    fn rewrites_https_ipv4_loopback() {
        assert_eq!(
            normalize_loopback_redirect("https://127.0.0.1:1455/auth/callback"),
            "http://127.0.0.1:1455/auth/callback",
        );
    }

    #[test]
    fn rewrites_https_ipv6_loopback() {
        assert_eq!(
            normalize_loopback_redirect("https://[::1]:1455/auth/callback"),
            "http://[::1]:1455/auth/callback",
        );
    }

    #[test]
    fn rewrites_https_localhost_case_insensitive() {
        assert_eq!(
            normalize_loopback_redirect("https://LocalHost:1455/auth/callback"),
            "http://localhost:1455/auth/callback",
        );
    }

    #[test]
    fn preserves_real_hostname() {
        assert_eq!(
            normalize_loopback_redirect("https://moltis.lan/auth/callback"),
            "https://moltis.lan/auth/callback",
        );
        assert_eq!(
            normalize_loopback_redirect("https://moltis.example.com:1455/auth/callback"),
            "https://moltis.example.com:1455/auth/callback",
        );
    }

    #[test]
    fn preserves_non_loopback_ipv4() {
        assert_eq!(
            normalize_loopback_redirect("https://192.168.1.10:1455/auth/callback"),
            "https://192.168.1.10:1455/auth/callback",
        );
    }

    #[test]
    fn preserves_http_scheme() {
        assert_eq!(
            normalize_loopback_redirect("http://localhost:1455/auth/callback"),
            "http://localhost:1455/auth/callback",
        );
    }

    #[test]
    fn preserves_query_and_path() {
        assert_eq!(
            normalize_loopback_redirect("https://localhost:1455/auth/callback?foo=bar"),
            "http://localhost:1455/auth/callback?foo=bar",
        );
    }

    #[test]
    fn returns_unparseable_input_unchanged() {
        assert_eq!(normalize_loopback_redirect("not a url"), "not a url");
    }
}
