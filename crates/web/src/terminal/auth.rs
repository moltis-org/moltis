use std::sync::Arc;

/// Locality detection lives in `moltis_auth::locality` — the single source
/// of truth for the three-tier auth model's "is this connection local?"
/// question. This used to be a separate copy of the same logic in this file,
/// which silently fell out of sync with the real implementation (it never
/// picked up `MOLTIS_TRUST_DOCKER_LOOPBACK`, so the terminal WebSocket
/// handler kept rejecting Docker-loopback connections the global auth gate
/// had already accepted). Re-export instead of duplicating.
pub(crate) use moltis_auth::locality::is_local_connection;

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
