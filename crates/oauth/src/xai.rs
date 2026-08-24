//! xAI Grok OAuth helpers for device-flow login and CLI chat proxy requests.

use reqwest::header::{HeaderMap, HeaderValue};

/// Default Grok CLI client version accepted by `cli-chat-proxy.grok.com`.
const DEFAULT_CLIENT_VERSION: &str = "0.2.101";
/// Session product label expected by the subscription proxy.
const DEFAULT_CLIENT_NAME: &str = "grok-shell";

fn client_version() -> String {
    std::env::var("MOLTIS_XAI_CLIENT_VERSION").unwrap_or_else(|_| DEFAULT_CLIENT_VERSION.into())
}

fn client_name() -> String {
    std::env::var("MOLTIS_XAI_CLIENT_NAME").unwrap_or_else(|_| DEFAULT_CLIENT_NAME.into())
}

fn platform_label() -> String {
    let os = match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "aarch64",
        "x86_64" => "x86_64",
        other => other,
    };
    format!("{os}; {arch}")
}

/// Headers required by xAI device-code / token endpoints.
pub fn xai_device_headers() -> HeaderMap {
    let version = client_version();
    let mut headers = HeaderMap::new();
    if let Ok(val) = HeaderValue::from_str(&version) {
        headers.insert("x-grok-client-version", val);
    }
    headers.insert("x-grok-client-surface", HeaderValue::from_static("cli"));
    headers
}

/// Identity headers required by `cli-chat-proxy.grok.com`.
///
/// When `model_id` is set (inference), also sends `x-grok-model-override` so the
/// proxy can remap subscription variants (e.g. `grok-4.5` → `grok-4.5-build`).
pub fn xai_proxy_headers(model_id: Option<&str>) -> HeaderMap {
    let version = client_version();
    let name = client_name();
    let mut headers = HeaderMap::new();

    let user_agent = format!("{name}/{version} ({})", platform_label());
    if let Ok(val) = HeaderValue::from_str(&user_agent) {
        headers.insert(reqwest::header::USER_AGENT, val);
    }
    if let Ok(val) = HeaderValue::from_str(&name) {
        headers.insert("x-grok-client-identifier", val);
    }
    if let Ok(val) = HeaderValue::from_str(&version) {
        headers.insert("x-grok-client-version", val);
    }
    headers.insert(
        "x-grok-client-mode",
        HeaderValue::from_static("interactive"),
    );
    headers.insert("X-XAI-Token-Auth", HeaderValue::from_static("xai-grok-cli"));
    headers.insert(
        "x-authenticateresponse",
        HeaderValue::from_static("authenticate-response"),
    );
    if let Some(model_id) = model_id
        && let Ok(val) = HeaderValue::from_str(model_id)
    {
        headers.insert("x-grok-model-override", val);
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_headers_include_version_and_surface() {
        let headers = xai_device_headers();
        assert!(headers.get("x-grok-client-version").is_some());
        assert_eq!(
            headers.get("x-grok-client-surface").map(|v| v.to_str().unwrap()),
            Some("cli")
        );
    }

    #[test]
    fn proxy_headers_include_identity_fields() {
        let headers = xai_proxy_headers(Some("grok-4.5"));
        assert!(headers.get(reqwest::header::USER_AGENT).is_some());
        assert!(headers.get("x-grok-client-identifier").is_some());
        assert!(headers.get("x-grok-client-version").is_some());
        assert_eq!(
            headers
                .get("x-grok-client-mode")
                .and_then(|v| v.to_str().ok()),
            Some("interactive")
        );
        assert_eq!(
            headers
                .get("X-XAI-Token-Auth")
                .and_then(|v| v.to_str().ok()),
            Some("xai-grok-cli")
        );
        assert_eq!(
            headers
                .get("x-grok-model-override")
                .and_then(|v| v.to_str().ok()),
            Some("grok-4.5")
        );
    }

    #[test]
    fn proxy_headers_omit_model_override_when_unset() {
        let headers = xai_proxy_headers(None);
        assert!(headers.get("x-grok-model-override").is_none());
    }
}
