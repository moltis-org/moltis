static SHARED_CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();

/// Initialize the shared HTTP client with optional proxy.
/// Call once at gateway startup; subsequent calls are no-ops.
pub fn init_shared_http_client(proxy_url: Option<&str>) {
    let _ = SHARED_CLIENT.set(moltis_common::http_client::build_http_client(proxy_url));
}

/// Shared HTTP client for tools that don't need custom configuration.
///
/// Reusing a single `reqwest::Client` avoids per-request connection pool,
/// DNS resolver, and TLS session cache overhead — significant on
/// memory-constrained devices.
///
/// Falls back to a client with default headers (including User-Agent)
/// if [`init_shared_http_client`] was never called (e.g. in tests).
pub fn shared_http_client() -> &'static reqwest::Client {
    SHARED_CLIENT.get_or_init(moltis_common::http_client::build_default_http_client)
}

/// Build a `reqwest::Client` with optional proxy configuration.
///
/// Re-export of [`moltis_common::http_client::build_http_client`] for
/// backward compatibility.
pub fn build_http_client(proxy_url: Option<&str>) -> reqwest::Client {
    moltis_common::http_client::build_http_client(proxy_url)
}
