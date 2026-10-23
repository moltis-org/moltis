//! Shared HTTP client and retry header helpers for LLM providers.

/// Keep the transport cause when crossing the string-valued stream error
/// boundary, without leaking credentials or query parameters from the URL.
pub(crate) fn format_transport_error(error: reqwest::Error) -> String {
    let timeout = error.is_timeout();
    let connect = error.is_connect();
    format!(
        "provider transport error (timeout={timeout}, connect={connect}): {:#}",
        anyhow::Error::new(error.without_url())
    )
}

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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use {
        super::*,
        std::{error::Error as _, time::Duration},
        tokio::io::AsyncReadExt,
    };

    #[tokio::test]
    async fn transport_error_keeps_cause_without_request_url() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/private-path?token=secret-value",
            listener.local_addr().unwrap()
        );
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let received = socket.read(&mut request).await.unwrap();
            assert!(received > 0, "client must start sending its request");
            // Close without a response to produce a genuine transport error.
        });
        let error = reqwest::Client::new()
            .post(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .unwrap_err();
        let cause = error.source().unwrap().to_string();
        let flags = format!(
            "timeout={}, connect={}",
            error.is_timeout(),
            error.is_connect()
        );
        let message = format_transport_error(error);
        assert!(message.contains(&cause));
        assert!(message.contains(&flags));
        assert!(!message.contains("secret-value"));
        assert!(!message.contains("private-path"));
        server.await.unwrap();
    }
}
