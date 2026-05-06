//! Shared HTTP client and retry header helpers for LLM providers.

static SHARED_CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();

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

/// Parse `Retry-After` header as milliseconds.
///
/// `Retry-After` may be either delta-seconds or an HTTP date. We currently
/// consume delta-seconds, which is what providers typically return for 429.
pub(crate) fn retry_after_ms_from_headers(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let value = headers.get(reqwest::header::RETRY_AFTER)?;
    let text = value.to_str().ok()?.trim();
    let seconds = text.parse::<u64>().ok()?;
    seconds.checked_mul(1_000)
}

/// Attach an explicit retry hint marker consumable by runner retry logic.
pub(crate) fn with_retry_after_marker(base: String, retry_after_ms: Option<u64>) -> String {
    match retry_after_ms {
        Some(ms) => format!("{base} (retry_after_ms={ms})"),
        None => base,
    }
}
