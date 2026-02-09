use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

use {
    anyhow::{Result, bail},
    async_trait::async_trait,
    tracing::debug,
    url::Url,
};

use {moltis_agents::tool_registry::AgentTool, moltis_config::schema::WebFetchConfig};

/// Cached fetch result with expiry.
struct CacheEntry {
    value: serde_json::Value,
    expires_at: Instant,
}

/// Web fetch tool — lets the LLM fetch a URL and extract readable content.
pub struct WebFetchTool {
    max_chars: usize,
    timeout: Duration,
    cache_ttl: Duration,
    max_redirects: u8,
    readability: bool,
    cache: Mutex<HashMap<String, CacheEntry>>,
}

impl WebFetchTool {
    /// Build from config; returns `None` if disabled.
    pub fn from_config(config: &WebFetchConfig) -> Option<Self> {
        if !config.enabled {
            return None;
        }
        Some(Self {
            max_chars: config.max_chars,
            timeout: Duration::from_secs(config.timeout_seconds),
            cache_ttl: Duration::from_secs(config.cache_ttl_minutes * 60),
            max_redirects: config.max_redirects,
            readability: config.readability,
            cache: Mutex::new(HashMap::new()),
        })
    }

    fn cache_get(&self, key: &str) -> Option<serde_json::Value> {
        let cache = self.cache.lock().ok()?;
        let entry = cache.get(key)?;
        if Instant::now() < entry.expires_at {
            Some(entry.value.clone())
        } else {
            None
        }
    }

    fn cache_set(&self, key: String, value: serde_json::Value) {
        if self.cache_ttl.is_zero() {
            return;
        }
        if let Ok(mut cache) = self.cache.lock() {
            if cache.len() > 100 {
                let now = Instant::now();
                cache.retain(|_, e| e.expires_at > now);
            }
            cache.insert(key, CacheEntry {
                value,
                expires_at: Instant::now() + self.cache_ttl,
            });
        }
    }

    async fn fetch_url(
        &self,
        url_str: &str,
        extract_mode: &str,
        max_chars: usize,
        accept_language: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut current_url = Url::parse(url_str)?;

        // Validate scheme.
        match current_url.scheme() {
            "http" | "https" => {},
            s => bail!("unsupported URL scheme: {s}"),
        }

        let client = reqwest::Client::builder()
            .timeout(self.timeout)
            .redirect(reqwest::redirect::Policy::none()) // Manual redirect handling.
            .build()?;

        let mut visited: Vec<String> = Vec::new();
        let mut hops = 0u8;

        loop {
            // SSRF check before each request.
            ssrf_check(&current_url).await?;
            visited.push(current_url.to_string());

            let mut req = client.get(current_url.as_str());
            if let Some(lang) = accept_language {
                req = req.header("Accept-Language", lang);
            }
            let resp = req.send().await?;
            let status = resp.status();

            if status.is_redirection() {
                if hops >= self.max_redirects {
                    bail!(
                        "too many redirects ({} hops, max {})",
                        hops + 1,
                        self.max_redirects
                    );
                }
                let location = resp
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| anyhow::anyhow!("redirect without Location header"))?;

                let next = current_url.join(location)?;

                // Loop detection.
                if visited.contains(&next.to_string()) {
                    bail!("redirect loop detected: {} → {}", current_url, next);
                }

                current_url = next;
                hops += 1;
                continue;
            }

            if !status.is_success() {
                return Ok(serde_json::json!({
                    "error": format!("HTTP {status}"),
                    "url": current_url.to_string(),
                }));
            }

            let content_type = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();

            let body = resp.text().await?;

            let (content, detected_mode) =
                extract_content(&body, &content_type, extract_mode, self.readability);

            let truncated = content.len() > max_chars;
            let content = if truncated {
                truncate_at_char_boundary(&content, max_chars)
            } else {
                content
            };

            return Ok(serde_json::json!({
                "url": current_url.to_string(),
                "content_type": content_type,
                "extract_mode": detected_mode,
                "content": content,
                "truncated": truncated,
                "original_length": body.len(),
            }));
        }
    }
}

/// SSRF protection: resolve the URL host and reject private/loopback/link-local IPs.
async fn ssrf_check(url: &Url) -> Result<()> {
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("URL has no host"))?;

    // Try parsing as IP directly.
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_private_ip(&ip) {
            bail!("SSRF blocked: {host} resolves to private IP {ip}");
        }
        return Ok(());
    }

    // DNS resolution.
    let port = url.port_or_known_default().unwrap_or(443);
    let addrs: Vec<_> = tokio::net::lookup_host(format!("{host}:{port}"))
        .await?
        .collect();

    if addrs.is_empty() {
        bail!("DNS resolution failed for {host}");
    }

    for addr in &addrs {
        if is_private_ip(&addr.ip()) {
            bail!("SSRF blocked: {host} resolves to private IP {}", addr.ip());
        }
    }

    Ok(())
}

/// Check if an IP address is private, loopback, or link-local.
fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                // 100.64.0.0/10 (CGNAT)
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64)
                // 192.0.0.0/24
                || (v4.octets()[0] == 192 && v4.octets()[1] == 0 && v4.octets()[2] == 0)
        },
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                // fc00::/7 (unique local)
                || (v6.segments()[0] & 0xFE00) == 0xFC00
                // fe80::/10 (link-local)
                || (v6.segments()[0] & 0xFFC0) == 0xFE80
        },
    }
}

/// Extract readable content from the response body based on content type.
fn extract_content(
    body: &str,
    content_type: &str,
    requested_mode: &str,
    use_readability: bool,
) -> (String, String) {
    let ct_lower = content_type.to_lowercase();

    // JSON: pretty-print.
    if ct_lower.contains("json") {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(body) {
            let pretty = serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| body.into());
            return (pretty, "json".into());
        }
        return (body.into(), "text".into());
    }

    // Plain text.
    if ct_lower.contains("text/plain") || !ct_lower.contains("html") {
        return (body.into(), "text".into());
    }

    // HTML: strip tags or use readability.
    if use_readability && (requested_mode == "markdown" || requested_mode.is_empty()) {
        let cleaned = html_to_text(body);
        return (cleaned, "markdown".into());
    }

    let cleaned = html_to_text(body);
    (cleaned, "text".into())
}

/// Simple HTML to text conversion: strip tags, decode basic entities,
/// collapse whitespace. A lightweight alternative to a full readability
/// crate — good enough for most pages.
fn html_to_text(html: &str) -> String {
    let mut result = String::with_capacity(html.len() / 2);
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;
    let mut last_was_space = false;

    let html_lower = html.to_lowercase();
    let bytes = html.as_bytes();
    let lower_bytes = html_lower.as_bytes();

    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            // Check for script/style open/close tags.
            if i + 7 < lower_bytes.len() && &lower_bytes[i..i + 7] == b"<script" {
                in_script = true;
            }
            if i + 9 < lower_bytes.len() && &lower_bytes[i..i + 9] == b"</script>" {
                in_script = false;
            }
            if i + 6 < lower_bytes.len() && &lower_bytes[i..i + 6] == b"<style" {
                in_style = true;
            }
            if i + 8 < lower_bytes.len() && &lower_bytes[i..i + 8] == b"</style>" {
                in_style = false;
            }

            // Block-level tags → newline.
            if !in_script && !in_style {
                let tag_start = &html_lower[i..];
                if tag_start.starts_with("<br")
                    || tag_start.starts_with("<p")
                    || tag_start.starts_with("</p")
                    || tag_start.starts_with("<div")
                    || tag_start.starts_with("</div")
                    || tag_start.starts_with("<h")
                    || tag_start.starts_with("</h")
                    || tag_start.starts_with("<li")
                {
                    if !result.ends_with('\n') {
                        result.push('\n');
                    }
                    last_was_space = true;
                }
            }

            in_tag = true;
            i += 1;
            continue;
        }

        if bytes[i] == b'>' {
            in_tag = false;
            i += 1;
            continue;
        }

        if in_tag || in_script || in_style {
            i += 1;
            continue;
        }

        // Decode HTML entities.
        if bytes[i] == b'&' {
            let rest = &html[i..];
            if let Some(semi) = rest.find(';') {
                let entity = &rest[..semi + 1];
                let decoded = match entity {
                    "&amp;" => "&",
                    "&lt;" => "<",
                    "&gt;" => ">",
                    "&quot;" => "\"",
                    "&apos;" | "&#39;" => "'",
                    "&nbsp;" | "&#160;" => " ",
                    _ => {
                        // Skip unknown entities.
                        i += 1;
                        continue;
                    },
                };
                result.push_str(decoded);
                last_was_space = decoded == " ";
                i += entity.len();
                continue;
            }
        }

        let ch = bytes[i] as char;
        if ch.is_ascii_whitespace() {
            if !last_was_space {
                result.push(' ');
                last_was_space = true;
            }
        } else {
            result.push(ch);
            last_was_space = false;
        }
        i += 1;
    }

    result.trim().to_string()
}

/// Truncate a string at a char boundary, not mid-UTF-8.
fn truncate_at_char_boundary(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.into();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

#[async_trait]
impl AgentTool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch a web page URL and extract its content as readable text or markdown. \
         Use this when you need to read the contents of a specific web page. \
         The request is sent with the user's Accept-Language header, so pages \
         are returned in the user's preferred language when available."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch (must be http or https)"
                },
                "extract_mode": {
                    "type": "string",
                    "enum": ["markdown", "text"],
                    "description": "Content extraction mode (default: markdown)"
                },
                "max_chars": {
                    "type": "integer",
                    "description": "Maximum characters to return (default: 50000)"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        let url = params
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'url' parameter"))?;

        let extract_mode = params
            .get("extract_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("markdown");

        let max_chars = params
            .get("max_chars")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(self.max_chars);

        // Check cache.
        let cache_key = format!("{url}:{extract_mode}:{max_chars}");
        if let Some(cached) = self.cache_get(&cache_key) {
            debug!("web_fetch cache hit for: {url}");
            return Ok(cached);
        }

        let accept_language = params.get("_accept_language").and_then(|v| v.as_str());

        debug!("web_fetch: {url}");
        let result = self
            .fetch_url(url, extract_mode, max_chars, accept_language)
            .await?;

        self.cache_set(cache_key, result.clone());
        Ok(result)
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    fn default_tool() -> WebFetchTool {
        WebFetchTool {
            max_chars: 50_000,
            timeout: Duration::from_secs(10),
            cache_ttl: Duration::from_secs(60),
            max_redirects: 3,
            readability: true,
            cache: Mutex::new(HashMap::new()),
        }
    }

    #[test]
    fn test_tool_name_and_schema() {
        let tool = default_tool();
        assert_eq!(tool.name(), "web_fetch");
        let schema = tool.parameters_schema();
        assert_eq!(schema["required"][0], "url");
    }

    #[tokio::test]
    async fn test_missing_url_param() {
        let tool = default_tool();
        let result = tool.execute(serde_json::json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("url"));
    }

    // --- SSRF tests ---

    #[test]
    fn test_is_private_ip_v4() {
        use std::net::Ipv4Addr;
        assert!(is_private_ip(&IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(is_private_ip(&IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
        assert!(is_private_ip(&IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(is_private_ip(&IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1))));
        assert!(is_private_ip(&IpAddr::V4(Ipv4Addr::new(169, 254, 1, 1))));
        assert!(is_private_ip(&IpAddr::V4(Ipv4Addr::UNSPECIFIED)));
        assert!(!is_private_ip(&IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(!is_private_ip(&IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
    }

    #[test]
    fn test_is_private_ip_v6() {
        use std::net::Ipv6Addr;
        assert!(is_private_ip(&IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(is_private_ip(&IpAddr::V6(Ipv6Addr::UNSPECIFIED)));
        // fc00::/7 unique local
        assert!(is_private_ip(&IpAddr::V6(
            "fd00::1".parse::<Ipv6Addr>().unwrap()
        )));
        // fe80::/10 link-local
        assert!(is_private_ip(&IpAddr::V6(
            "fe80::1".parse::<Ipv6Addr>().unwrap()
        )));
        // Public
        assert!(!is_private_ip(&IpAddr::V6(
            "2607:f8b0:4004:800::200e".parse::<Ipv6Addr>().unwrap()
        )));
    }

    #[tokio::test]
    async fn test_ssrf_blocks_localhost_url() {
        let url = Url::parse("http://127.0.0.1/secret").unwrap();
        let result = ssrf_check(&url).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("SSRF"));
    }

    #[tokio::test]
    async fn test_ssrf_blocks_private_ip() {
        let url = Url::parse("http://192.168.1.1/admin").unwrap();
        let result = ssrf_check(&url).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("SSRF"));
    }

    #[tokio::test]
    async fn test_ssrf_blocks_link_local() {
        let url = Url::parse("http://169.254.1.1/metadata").unwrap();
        let result = ssrf_check(&url).await;
        assert!(result.is_err());
    }

    // --- Content extraction tests ---

    #[test]
    fn test_html_to_text_basic() {
        let html = "<html><body><h1>Hello</h1><p>World</p></body></html>";
        let text = html_to_text(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
    }

    #[test]
    fn test_html_to_text_strips_scripts() {
        let html = "<p>Before</p><script>alert('xss')</script><p>After</p>";
        let text = html_to_text(html);
        assert!(text.contains("Before"));
        assert!(text.contains("After"));
        assert!(!text.contains("alert"));
    }

    #[test]
    fn test_html_to_text_strips_styles() {
        let html = "<style>.foo{color:red}</style><p>Content</p>";
        let text = html_to_text(html);
        assert!(text.contains("Content"));
        assert!(!text.contains("color"));
    }

    #[test]
    fn test_html_to_text_entities() {
        let html = "<p>A &amp; B &lt; C &gt; D &quot;E&quot;</p>";
        let text = html_to_text(html);
        assert!(text.contains("A & B < C > D \"E\""));
    }

    #[test]
    fn test_extract_content_json() {
        let body = r#"{"key": "value"}"#;
        let (content, mode) = extract_content(body, "application/json", "text", true);
        assert_eq!(mode, "json");
        assert!(content.contains("\"key\""));
    }

    #[test]
    fn test_extract_content_plain_text() {
        let body = "Hello world";
        let (content, mode) = extract_content(body, "text/plain", "text", true);
        assert_eq!(mode, "text");
        assert_eq!(content, "Hello world");
    }

    #[test]
    fn test_truncation() {
        let long = "a".repeat(100);
        let truncated = truncate_at_char_boundary(&long, 50);
        assert_eq!(truncated.len(), 50);
    }

    #[test]
    fn test_truncation_utf8_boundary() {
        let s = "héllo wörld";
        let truncated = truncate_at_char_boundary(s, 3);
        // Should not panic and should be valid UTF-8.
        assert!(truncated.len() <= 3);
        assert!(std::str::from_utf8(truncated.as_bytes()).is_ok());
    }

    // --- Cache tests ---

    #[test]
    fn test_cache_hit_and_miss() {
        let tool = default_tool();
        let key = "test-key".to_string();
        let val = serde_json::json!({"cached": true});

        assert!(tool.cache_get(&key).is_none());
        tool.cache_set(key.clone(), val.clone());
        assert_eq!(tool.cache_get(&key).unwrap(), val);
    }

    #[test]
    fn test_cache_disabled_zero_ttl() {
        let tool = WebFetchTool {
            cache_ttl: Duration::ZERO,
            ..default_tool()
        };
        tool.cache_set("k".into(), serde_json::json!(1));
        assert!(tool.cache_get("k").is_none());
    }

    // --- Config tests ---

    #[test]
    fn test_from_config_disabled() {
        let cfg = WebFetchConfig {
            enabled: false,
            ..Default::default()
        };
        assert!(WebFetchTool::from_config(&cfg).is_none());
    }

    #[test]
    fn test_from_config_enabled() {
        let cfg = WebFetchConfig::default();
        assert!(WebFetchTool::from_config(&cfg).is_some());
    }

    #[tokio::test]
    async fn test_unsupported_scheme() {
        let tool = default_tool();
        let result = tool
            .fetch_url("ftp://example.com", "text", 50000, None)
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unsupported"));
    }
}
