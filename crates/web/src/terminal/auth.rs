use std::{net::SocketAddr, sync::Arc};

/// Returns `true` when the request carries headers typically set by reverse proxies.
fn has_proxy_headers(headers: &axum::http::HeaderMap) -> bool {
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
/// in between).  This is the per-request check used by the three-tier auth
/// model:
///
/// 1. Password set -> always require auth
/// 2. No password + local -> full access (dev convenience)
/// 3. No password + remote/proxied -> onboarding only
///
/// A connection is considered local when **all** of the following hold:
///
/// - `MOLTIS_BEHIND_PROXY` is **not** set (`behind_proxy == false`)
/// - No proxy headers are present (X-Forwarded-For, X-Real-IP, etc.)
/// - The `Host` header resolves to a loopback address (or is absent)
/// - The TCP source IP is loopback
pub(crate) fn is_local_connection(
    headers: &axum::http::HeaderMap,
    remote_addr: SocketAddr,
    behind_proxy: bool,
) -> bool {
    // Hard override: env var says we're behind a proxy.
    if behind_proxy {
        return false;
    }

    // Proxy headers present -> proxied traffic.
    if has_proxy_headers(headers) {
        return false;
    }

    // Host header points to a non-loopback name -> likely proxied.
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

pub(crate) async fn websocket_header_authenticated(
    headers: &axum::http::HeaderMap,
    credential_store: Option<&Arc<moltis_gateway::auth::CredentialStore>>,
    is_local: bool,
) -> bool {
    let Some(store) = credential_store else {
        return false;
    };

    matches!(
        moltis_httpd::auth_middleware::check_auth(store, headers, is_local).await,
        moltis_httpd::auth_middleware::AuthResult::Allowed(_)
    )
}

/// Check whether a WebSocket `Origin` header matches the request `Host`.
///
/// Extracts the host portion of the origin URL and compares it to the Host
/// header.  Accepts `localhost`, `127.0.0.1`, and `[::1]` interchangeably
/// so that `http://localhost:8080` matches a Host of `127.0.0.1:8080`.
pub(crate) fn is_same_origin(origin: &str, host: &str) -> bool {
    // Origin is a full URL (e.g. "https://localhost:8080"), Host is just
    // "host:port" or "host".
    let origin_host = origin
        .split("://")
        .nth(1)
        .unwrap_or(origin)
        .split('/')
        .next()
        .unwrap_or("");

    fn strip_port(h: &str) -> &str {
        if h.starts_with('[') {
            // IPv6: [::1]:port
            h.rsplit_once("]:")
                .map_or(h, |(addr, _)| addr)
                .trim_start_matches('[')
                .trim_end_matches(']')
        } else {
            h.rsplit_once(':').map_or(h, |(addr, _)| addr)
        }
    }
    fn get_port(h: &str) -> Option<&str> {
        if h.starts_with('[') {
            h.rsplit_once("]:").map(|(_, p)| p)
        } else {
            h.rsplit_once(':').map(|(_, p)| p)
        }
    }

    let origin_port = get_port(origin_host);
    let host_port = get_port(host);

    let oh = strip_port(origin_host);
    let hh = strip_port(host);

    // Normalise loopback variants so 127.0.0.1 == localhost == ::1.
    // Subdomains of .localhost (e.g. moltis.localhost) are also loopback per RFC 6761.
    let is_loopback =
        |h: &str| matches!(h, "localhost" | "127.0.0.1" | "::1") || h.ends_with(".localhost");

    (oh == hh || (is_loopback(oh) && is_loopback(hh))) && origin_port == host_port
}
