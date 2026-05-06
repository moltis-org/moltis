use std::sync::OnceLock;

static SHARED_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// Initialize the shared provider HTTP client with optional upstream proxy.
///
/// Call once at gateway startup; subsequent calls are no-ops.
pub fn init_shared_http_client(proxy_url: Option<&str>) {
    let _ = SHARED_CLIENT.set(moltis_common::http_client::build_http_client(proxy_url));
}

/// Shared HTTP client for LLM providers.
///
/// All providers that don't need custom redirect/proxy settings should
/// reuse this client to share connection pools, DNS cache, and TLS sessions.
///
/// Falls back to a client with default headers (including User-Agent)
/// if [`init_shared_http_client`] was never called (e.g. in tests).
pub fn shared_http_client() -> &'static reqwest::Client {
    SHARED_CLIENT.get_or_init(moltis_common::http_client::build_default_http_client)
}
