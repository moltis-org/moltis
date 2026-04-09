use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

#[cfg(feature = "firecrawl")]
use secrecy::ExposeSecret;
use {async_trait::async_trait, tracing::debug, url::Url};

use crate::error::Error;

use {
    crate::ssrf::ssrf_check, moltis_agents::tool_registry::AgentTool,
    moltis_config::schema::WebFetchConfig,
};

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
    ssrf_allowlist: Vec<ipnet::IpNet>,
    cache: Mutex<HashMap<String, CacheEntry>>,
    proxy_url: Option<String>,
    /// Firecrawl fallback: API key for calling Firecrawl when local extraction
    /// produces poor results.
    #[cfg(feature = "firecrawl")]
    firecrawl_api_key: Option<secrecy::Secret<String>>,
    /// Firecrawl base URL.
    #[cfg(feature = "firecrawl")]
    firecrawl_base_url: String,
    /// Firecrawl: only extract main content.
    #[cfg(feature = "firecrawl")]
    firecrawl_only_main_content: bool,
    /// Whether to use Firecrawl as fallback when readability extraction
    /// produces poor results.
    #[cfg(feature = "firecrawl")]
    firecrawl_fallback: bool,
}

impl WebFetchTool {
    /// Build from config; returns `None` if disabled.
    pub fn from_config(config: &WebFetchConfig) -> Option<Self> {
        if !config.enabled {
            return None;
        }
        let ssrf_allowlist: Vec<ipnet::IpNet> = config
            .ssrf_allowlist
            .iter()
            .filter_map(|s| match s.parse::<ipnet::IpNet>() {
                Ok(net) => Some(net),
                Err(e) => {
                    tracing::warn!("ignoring invalid ssrf_allowlist entry \"{s}\": {e}");
                    None
                },
            })
            .collect();
        Some(Self {
            max_chars: config.max_chars,
            timeout: Duration::from_secs(config.timeout_seconds),
            cache_ttl: Duration::from_secs(config.cache_ttl_minutes * 60),
            max_redirects: config.max_redirects,
            readability: config.readability,
            ssrf_allowlist,
            cache: Mutex::new(HashMap::new()),
            proxy_url: None,
            #[cfg(feature = "firecrawl")]
            firecrawl_api_key: None,
            #[cfg(feature = "firecrawl")]
            firecrawl_base_url: crate::firecrawl::DEFAULT_BASE_URL.into(),
            #[cfg(feature = "firecrawl")]
            firecrawl_only_main_content: true,
            #[cfg(feature = "firecrawl")]
            firecrawl_fallback: false,
        })
    }

    /// Route HTTP traffic through a proxy (e.g. the trusted-network proxy).
    #[must_use]
    pub fn with_proxy(mut self, url: String) -> Self {
        self.proxy_url = Some(url);
        self
    }

    /// Attach Firecrawl config for fallback extraction.
    ///
    /// When enabled, `web_fetch` will try Firecrawl as a fallback when local
    /// readability extraction produces poor results (very short output
    /// relative to the HTML body).
    #[cfg(feature = "firecrawl")]
    #[must_use]
    pub fn with_firecrawl(mut self, config: &moltis_config::schema::FirecrawlConfig) -> Self {
        if config.enabled && config.web_fetch_fallback {
            self.firecrawl_api_key = crate::firecrawl::resolve_api_key(config);
            self.firecrawl_base_url = if config.base_url.trim().is_empty() {
                crate::firecrawl::DEFAULT_BASE_URL.into()
            } else {
                config.base_url.clone()
            };
            self.firecrawl_only_main_content = config.only_main_content;
            self.firecrawl_fallback = self.firecrawl_api_key.is_some();
            if !self.firecrawl_fallback {
                tracing::warn!(
                    "firecrawl web_fetch_fallback is enabled but no API key found; \
                     set tools.web.firecrawl.api_key or FIRECRAWL_API_KEY env var"
                );
            }
        }
        self
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
    ) -> crate::Result<serde_json::Value> {
        let mut current_url = Url::parse(url_str)?;

        // Validate scheme.
        match current_url.scheme() {
            "http" | "https" => {},
            s => return Err(Error::message(format!("unsupported URL scheme: {s}"))),
        }

        let mut client_builder = reqwest::Client::builder()
            .timeout(self.timeout)
            .redirect(reqwest::redirect::Policy::none()); // Manual redirect handling.
        // Prefer the sandbox proxy when set, otherwise fall through to the
        // upstream proxy (if configured).
        let upstream = moltis_common::http_client::upstream_proxy_url();
        let effective_proxy = self.proxy_url.as_deref().or(upstream);
        if let Some(url) = effective_proxy
            && let Ok(proxy) = reqwest::Proxy::all(url)
        {
            let proxy = proxy.no_proxy(reqwest::NoProxy::from_string("localhost,127.0.0.1,::1"));
            client_builder = client_builder.proxy(proxy);
        }
        let client = client_builder.build()?;

        let mut visited: Vec<String> = Vec::new();
        let mut hops = 0u8;

        loop {
            // SSRF check before each request.
            ssrf_check(&current_url, &self.ssrf_allowlist).await?;
            visited.push(current_url.to_string());

            let mut req = client.get(current_url.as_str());
            if let Some(lang) = accept_language {
                req = req.header("Accept-Language", lang);
            }
            let resp = req.send().await?;
            let status = resp.status();

            if status.is_redirection() {
                if hops >= self.max_redirects {
                    return Err(Error::message(format!(
                        "too many redirects ({} hops, max {})",
                        hops + 1,
                        self.max_redirects
                    )));
                }
                let location = resp
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| Error::message("redirect without Location header"))?;

                let next = current_url.join(location)?;

                // Loop detection.
                if visited.contains(&next.to_string()) {
                    return Err(Error::message(format!(
                        "redirect loop detected: {} → {}",
                        current_url, next
                    )));
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

            let (mut content, mut detected_mode) =
                extract_content(&body, &content_type, extract_mode, self.readability);

            // Firecrawl fallback: when readability extraction produced very
            // little content relative to the HTML body, try Firecrawl.
            #[cfg(feature = "firecrawl")]
            if self.firecrawl_fallback
                && detected_mode != "json"
                && content_type.to_lowercase().contains("html")
                && let Some(ref api_key) = self.firecrawl_api_key
            {
                // Heuristic: readability output < 10% of HTML body suggests
                // extraction failed (JS-rendered page, anti-bot protection, etc.).
                let ratio_poor = !body.is_empty() && content.len() < body.len() / 10;
                let content_very_short = content.len() < 200 && body.len() > 1000;
                if ratio_poor || content_very_short {
                    debug!(
                        "web_fetch: readability produced {}/{} chars, trying firecrawl fallback",
                        content.len(),
                        body.len()
                    );
                    match crate::firecrawl::firecrawl_scrape(
                        crate::shared_http_client(),
                        &self.firecrawl_base_url,
                        api_key.expose_secret(),
                        current_url.as_str(),
                        self.firecrawl_only_main_content,
                        self.timeout,
                    )
                    .await
                    {
                        Ok(Some(scrape)) => {
                            content = scrape.markdown;
                            detected_mode = "firecrawl".into();
                        },
                        Ok(None) => {
                            debug!(
                                "web_fetch: firecrawl returned no content, keeping local extraction"
                            );
                        },
                        Err(e) => {
                            debug!(
                                "web_fetch: firecrawl fallback failed: {e}, keeping local extraction"
                            );
                        },
                    }
                }
            }

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

/// Convert HTML to plain text using the `html2text` crate.
/// Strips tags, decodes all HTML entities, and collapses consecutive
/// blank lines. Uses `TrivialDecorator` so link URLs and markup
/// annotations are dropped.
fn html_to_text(html: &str) -> String {
    let clean = |text: String| -> String {
        let mut lines: Vec<&str> = text.lines().map(str::trim_end).collect();
        lines.dedup_by(|a, b| a.is_empty() && b.is_empty());
        lines.join("\n").trim().to_string()
    };

    match html2text::config::with_decorator(html2text::render::TrivialDecorator::new())
        .string_from_read(html.as_bytes(), 1_000_000)
    {
        Ok(text) => clean(text),
        Err(e) => {
            tracing::warn!("html2text parse failed, returning raw HTML body: {e}");
            clean(html.to_string())
        },
    }
}

/// Truncate a string at a char boundary, not mid-UTF-8.
pub(crate) fn truncate_at_char_boundary(s: &str, max: usize) -> String {
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

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let url = params
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::message("missing 'url' parameter"))?;

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
    use {
        super::*,
        crate::ssrf::{is_private_ip, is_ssrf_allowed, ssrf_check},
        std::net::IpAddr,
    };

    fn default_tool() -> WebFetchTool {
        WebFetchTool {
            max_chars: 50_000,
            timeout: Duration::from_secs(10),
            cache_ttl: Duration::from_secs(60),
            max_redirects: 3,
            readability: true,
            ssrf_allowlist: vec![],
            cache: Mutex::new(HashMap::new()),
            proxy_url: None,
            #[cfg(feature = "firecrawl")]
            firecrawl_api_key: None,
            #[cfg(feature = "firecrawl")]
            firecrawl_base_url: crate::firecrawl::DEFAULT_BASE_URL.into(),
            #[cfg(feature = "firecrawl")]
            firecrawl_only_main_content: true,
            #[cfg(feature = "firecrawl")]
            firecrawl_fallback: false,
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

    use rstest::rstest;

    #[rstest]
    #[case("127.0.0.1", true)]
    #[case("192.168.1.1", true)]
    #[case("10.0.0.1", true)]
    #[case("172.16.0.1", true)]
    #[case("169.254.1.1", true)]
    #[case("0.0.0.0", true)]
    #[case("8.8.8.8", false)]
    #[case("1.1.1.1", false)]
    fn test_is_private_ip_v4(#[case] addr: &str, #[case] expected: bool) {
        let ip: IpAddr = addr.parse().unwrap();
        assert_eq!(is_private_ip(&ip), expected, "{addr}");
    }

    #[rstest]
    #[case("::1", true)]
    #[case("::", true)]
    #[case("fd00::1", true)]
    #[case("fe80::1", true)]
    #[case("2607:f8b0:4004:800::200e", false)]
    fn test_is_private_ip_v6(#[case] addr: &str, #[case] expected: bool) {
        let ip: IpAddr = addr.parse().unwrap();
        assert_eq!(is_private_ip(&ip), expected, "{addr}");
    }

    #[tokio::test]
    async fn test_ssrf_blocks_localhost_url() {
        let url = Url::parse("http://127.0.0.1/secret").unwrap();
        let result = ssrf_check(&url, &[]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("SSRF"));
    }

    #[tokio::test]
    async fn test_ssrf_blocks_private_ip() {
        let url = Url::parse("http://192.168.1.1/admin").unwrap();
        let result = ssrf_check(&url, &[]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("SSRF"));
    }

    #[tokio::test]
    async fn test_ssrf_blocks_link_local() {
        let url = Url::parse("http://169.254.1.1/metadata").unwrap();
        let result = ssrf_check(&url, &[]).await;
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

    // --- Regression tests: multibyte char boundary safety (#420) ---

    #[test]
    fn test_html_to_text_replacement_chars() {
        // U+FFFD (3 bytes in UTF-8) from lossy decoding of legacy pages.
        let html = "<p>Hello \u{FFFD}\u{FFFD}\u{FFFD} world</p>";
        let text = html_to_text(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("world"));
        assert!(
            text.contains('\u{FFFD}'),
            "replacement chars should pass through"
        );
    }

    #[test]
    fn test_html_to_text_cjk_characters() {
        let html = "<h1>タイトル</h1><p>日本語のテスト</p>";
        let text = html_to_text(html);
        assert!(text.contains("タイトル"));
        assert!(text.contains("日本語のテスト"));
    }

    #[test]
    fn test_html_to_text_mixed_encoding_content() {
        let html =
            "<div>Price: &lt;¥500&gt;</div><script>var x='テスト';</script><p>End \u{FFFD}</p>";
        let text = html_to_text(html);
        assert!(text.contains("Price:"));
        assert!(text.contains("<¥500>"));
        assert!(text.contains("End"));
        assert!(!text.contains("var x"));
    }

    #[test]
    fn test_html_to_text_entity_with_multibyte_neighbors() {
        let html = "<p>東京&amp;大阪</p>";
        let text = html_to_text(html);
        assert!(text.contains("東京&大阪"));
    }

    #[test]
    fn test_html_to_text_drops_link_urls() {
        let html = r#"<p>Visit <a href="https://example.com/secret">our site</a> today.</p>"#;
        let text = html_to_text(html);
        assert!(text.contains("our site"), "link text should be kept");
        assert!(
            !text.contains("https://example.com"),
            "link href must not appear in plain-text output"
        );
        assert!(
            !text.contains("[1]"),
            "link footnote references must not appear"
        );
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

    // --- SSRF allowlist tests ---

    #[test]
    fn test_is_ssrf_allowed_matching_cidr() {
        let allowlist: Vec<ipnet::IpNet> = vec!["172.22.0.0/16".parse().unwrap()];
        let ip: IpAddr = "172.22.1.5".parse().unwrap();
        assert!(is_ssrf_allowed(&ip, &allowlist));
    }

    #[test]
    fn test_is_ssrf_allowed_non_matching() {
        let allowlist: Vec<ipnet::IpNet> = vec!["172.22.0.0/16".parse().unwrap()];
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        assert!(!is_ssrf_allowed(&ip, &allowlist));
    }

    #[test]
    fn test_is_ssrf_allowed_empty_blocks_all() {
        let ip: IpAddr = "172.22.1.5".parse().unwrap();
        assert!(!is_ssrf_allowed(&ip, &[]));
    }

    #[test]
    fn test_is_ssrf_allowed_single_host() {
        let allowlist: Vec<ipnet::IpNet> = vec!["172.22.0.5/32".parse().unwrap()];
        assert!(is_ssrf_allowed(&"172.22.0.5".parse().unwrap(), &allowlist));
        assert!(!is_ssrf_allowed(&"172.22.0.6".parse().unwrap(), &allowlist));
    }

    #[test]
    fn test_is_ssrf_allowed_ipv6() {
        let allowlist: Vec<ipnet::IpNet> = vec!["fd00::/8".parse().unwrap()];
        let ip: IpAddr = "fd12::1".parse().unwrap();
        assert!(is_ssrf_allowed(&ip, &allowlist));
        let ip_outside: IpAddr = "fe80::1".parse().unwrap();
        assert!(!is_ssrf_allowed(&ip_outside, &allowlist));
    }

    #[tokio::test]
    async fn test_ssrf_check_allowlist_permits_private_ip() {
        let allowlist: Vec<ipnet::IpNet> = vec!["172.22.0.0/16".parse().unwrap()];
        let url = Url::parse("http://172.22.1.5/api").unwrap();
        let result = ssrf_check(&url, &allowlist).await;
        assert!(result.is_ok(), "allowlisted IP should pass SSRF check");
    }

    #[tokio::test]
    async fn test_ssrf_check_allowlist_still_blocks_non_matching() {
        let allowlist: Vec<ipnet::IpNet> = vec!["172.22.0.0/16".parse().unwrap()];
        let url = Url::parse("http://10.0.0.1/admin").unwrap();
        let result = ssrf_check(&url, &allowlist).await;
        assert!(
            result.is_err(),
            "non-allowlisted private IP should be blocked"
        );
    }

    #[test]
    fn test_from_config_parses_ssrf_allowlist() {
        let cfg = WebFetchConfig {
            ssrf_allowlist: vec![
                "172.22.0.0/16".into(),
                "invalid".into(),
                "10.0.0.0/8".into(),
            ],
            ..Default::default()
        };
        let tool = WebFetchTool::from_config(&cfg).unwrap();
        assert_eq!(tool.ssrf_allowlist.len(), 2);
    }

    #[test]
    fn test_with_proxy() {
        let tool = default_tool();
        assert!(tool.proxy_url.is_none());
        let tool = tool.with_proxy("http://127.0.0.1:18791".into());
        assert_eq!(tool.proxy_url.as_deref(), Some("http://127.0.0.1:18791"));
    }

    #[test]
    fn test_from_config_has_no_proxy_by_default() {
        let cfg = WebFetchConfig::default();
        let tool = WebFetchTool::from_config(&cfg).unwrap();
        assert!(tool.proxy_url.is_none());
    }
}
