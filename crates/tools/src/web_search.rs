use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use {
    anyhow::{Result, bail},
    async_trait::async_trait,
    secrecy::{ExposeSecret, Secret},
    serde::{Deserialize, Serialize},
    tracing::{debug, warn},
};

use {
    moltis_agents::tool_registry::AgentTool,
    moltis_config::schema::{
        PerplexityConfig, SearchProvider as ConfigSearchProvider, WebSearchConfig,
    },
};

/// Cached search result with expiry.
struct CacheEntry {
    value: serde_json::Value,
    expires_at: Instant,
}

/// Web search tool — lets the LLM search the web via Brave Search or Perplexity.
///
/// When the configured provider's API key is missing, the tool automatically
/// falls back to DuckDuckGo HTML search so the LLM never has to ask the user.
pub struct WebSearchTool {
    provider: SearchProvider,
    api_key: Secret<String>,
    max_results: u8,
    timeout: Duration,
    cache_ttl: Duration,
    cache: Mutex<HashMap<String, CacheEntry>>,
    /// Whether to fall back to DuckDuckGo when the API key is missing.
    /// Always `true` in production; set to `false` in unit tests to avoid
    /// network calls.
    fallback_enabled: bool,
}

#[derive(Debug, Clone)]
enum SearchProvider {
    Brave,
    Perplexity { base_url: String, model: String },
}

/// A single Brave search result.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BraveResult {
    title: String,
    url: String,
    description: String,
}

/// Brave API web search response (subset).
#[derive(Debug, Deserialize)]
struct BraveSearchResponse {
    #[serde(default)]
    web: Option<BraveWebResults>,
}

#[derive(Debug, Deserialize)]
struct BraveWebResults {
    #[serde(default)]
    results: Vec<BraveWebResult>,
}

#[derive(Debug, Deserialize)]
struct BraveWebResult {
    title: String,
    url: String,
    #[serde(default)]
    description: String,
}

/// Perplexity (OpenAI-compatible) chat completion response (subset).
#[derive(Debug, Deserialize)]
struct PerplexityResponse {
    choices: Vec<PerplexityChoice>,
    #[serde(default)]
    citations: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PerplexityChoice {
    message: PerplexityMessage,
}

#[derive(Debug, Deserialize)]
struct PerplexityMessage {
    content: String,
}

impl WebSearchTool {
    /// Build from config; returns `None` if disabled or no API key available.
    pub fn from_config(config: &WebSearchConfig) -> Option<Self> {
        if !config.enabled {
            return None;
        }

        match config.provider {
            ConfigSearchProvider::Brave => {
                let api_key = config
                    .api_key
                    .as_ref()
                    .map(|s| s.expose_secret().clone())
                    .or_else(|| std::env::var("BRAVE_API_KEY").ok())
                    .unwrap_or_default();
                Some(Self::new(
                    SearchProvider::Brave,
                    Secret::new(api_key),
                    config.max_results,
                    Duration::from_secs(config.timeout_seconds),
                    Duration::from_secs(config.cache_ttl_minutes * 60),
                    true,
                ))
            },
            ConfigSearchProvider::Perplexity => {
                let (api_key, base_url) = resolve_perplexity_config(&config.perplexity);
                let model = config
                    .perplexity
                    .model
                    .clone()
                    .unwrap_or_else(|| "perplexity/sonar-pro".into());
                Some(Self::new(
                    SearchProvider::Perplexity { base_url, model },
                    api_key,
                    config.max_results,
                    Duration::from_secs(config.timeout_seconds),
                    Duration::from_secs(config.cache_ttl_minutes * 60),
                    true,
                ))
            },
        }
    }

    fn new(
        provider: SearchProvider,
        api_key: Secret<String>,
        max_results: u8,
        timeout: Duration,
        cache_ttl: Duration,
        fallback_enabled: bool,
    ) -> Self {
        Self {
            provider,
            api_key,
            max_results,
            timeout,
            cache_ttl,
            cache: Mutex::new(HashMap::new()),
            fallback_enabled,
        }
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
            // Evict expired entries periodically.
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

    async fn search_brave(
        &self,
        query: &str,
        count: u8,
        params: &serde_json::Value,
        accept_language: Option<&str>,
    ) -> Result<serde_json::Value> {
        if self.api_key.expose_secret().is_empty() {
            return Ok(serde_json::json!({
                "error": "Brave Search API key not configured",
                "hint": "Set BRAVE_API_KEY environment variable or tools.web.search.api_key in config"
            }));
        }

        let mut url = format!(
            "https://api.search.brave.com/res/v1/web/search?q={}&count={count}",
            urlencoding::encode(query)
        );

        if let Some(country) = params.get("country").and_then(|v| v.as_str()) {
            url.push_str(&format!("&country={country}"));
        }
        if let Some(lang) = params.get("search_lang").and_then(|v| v.as_str()) {
            url.push_str(&format!("&search_lang={lang}"));
        }
        if let Some(lang) = params.get("ui_lang").and_then(|v| v.as_str()) {
            url.push_str(&format!("&ui_lang={lang}"));
        }
        if let Some(freshness) = params.get("freshness").and_then(|v| v.as_str()) {
            url.push_str(&format!("&freshness={freshness}"));
        }

        let client = reqwest::Client::builder().timeout(self.timeout).build()?;

        let mut req = client
            .get(&url)
            .header("Accept", "application/json")
            .header("Accept-Encoding", "gzip")
            .header(
                "X-Subscription-Token",
                self.api_key.expose_secret().as_str(),
            );
        if let Some(lang) = accept_language {
            req = req.header("Accept-Language", lang);
        }
        let resp = req.send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Brave Search API returned {status}: {body}");
        }

        let brave_resp: BraveSearchResponse = resp.json().await?;
        let results: Vec<BraveResult> = brave_resp
            .web
            .map(|w| {
                w.results
                    .into_iter()
                    .map(|r| BraveResult {
                        title: r.title,
                        url: r.url,
                        description: r.description,
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(serde_json::json!({
            "provider": "brave",
            "query": query,
            "results": results,
        }))
    }

    async fn search_perplexity(
        &self,
        query: &str,
        base_url: &str,
        model: &str,
    ) -> Result<serde_json::Value> {
        if self.api_key.expose_secret().is_empty() {
            return Ok(serde_json::json!({
                "error": "Perplexity API key not configured",
                "hint": "Set PERPLEXITY_API_KEY or OPENROUTER_API_KEY environment variable, or tools.web.search.perplexity.api_key in config"
            }));
        }

        let client = reqwest::Client::builder().timeout(self.timeout).build()?;

        let body = serde_json::json!({
            "model": model,
            "messages": [
                {"role": "user", "content": query}
            ]
        });

        let resp = client
            .post(format!("{base_url}/chat/completions"))
            .header(
                "Authorization",
                format!("Bearer {}", self.api_key.expose_secret()),
            )
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            bail!("Perplexity API returned {status}: {text}");
        }

        let pplx: PerplexityResponse = resp.json().await?;
        let answer = pplx
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        Ok(serde_json::json!({
            "provider": "perplexity",
            "query": query,
            "answer": answer,
            "citations": pplx.citations,
        }))
    }

    /// Fallback: search DuckDuckGo's HTML endpoint when no API key is configured.
    async fn search_duckduckgo(&self, query: &str, count: u8) -> Result<serde_json::Value> {
        let client = reqwest::Client::builder().timeout(self.timeout).build()?;

        let resp = client
            .post("https://html.duckduckgo.com/html/")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Referer", "https://html.duckduckgo.com/")
            .body(format!("q={}&b=", urlencoding::encode(query)))
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            bail!("DuckDuckGo returned HTTP {status}");
        }

        let html = resp.text().await?;

        if html.contains("challenge-form") || html.contains("not a Robot") {
            bail!("DuckDuckGo returned a CAPTCHA challenge");
        }

        let results = parse_duckduckgo_html(&html, count);

        Ok(serde_json::json!({
            "provider": "duckduckgo",
            "query": query,
            "results": results,
            "note": "Results from DuckDuckGo (search API key not configured)"
        }))
    }
}

/// Resolve Perplexity API key and base URL from config / env.
fn resolve_perplexity_config(cfg: &PerplexityConfig) -> (Secret<String>, String) {
    let api_key = cfg
        .api_key
        .as_ref()
        .map(|s| s.expose_secret().clone())
        .or_else(|| std::env::var("PERPLEXITY_API_KEY").ok())
        .or_else(|| std::env::var("OPENROUTER_API_KEY").ok())
        .unwrap_or_default();

    let base_url = cfg.base_url.clone().unwrap_or_else(|| {
        if api_key.starts_with("pplx-") {
            "https://api.perplexity.ai".into()
        } else {
            // Assume OpenRouter for sk-or- or other keys.
            "https://openrouter.ai/api/v1".into()
        }
    });

    (Secret::new(api_key), base_url)
}

// ---------------------------------------------------------------------------
// DuckDuckGo HTML parsing helpers
// ---------------------------------------------------------------------------

/// Parse DuckDuckGo HTML search results into structured result objects.
fn parse_duckduckgo_html(html: &str, max_results: u8) -> Vec<serde_json::Value> {
    let mut results = Vec::new();
    let max = max_results as usize;
    let mut search_from = 0;

    while results.len() < max {
        let Some(anchor_pos) = html[search_from..].find("class=\"result__a\"") else {
            break;
        };
        let anchor_abs = search_from + anchor_pos;
        search_from = anchor_abs + 1;

        let Some(href) = extract_href_before(html, anchor_abs) else {
            continue;
        };

        let url = resolve_ddg_redirect(&href);
        let title = extract_tag_text(html, anchor_abs);

        let snippet = html[anchor_abs..]
            .find("class=\"result__snippet\"")
            .map(|offset| extract_tag_text(html, anchor_abs + offset))
            .unwrap_or_default();

        if !url.is_empty() && !title.is_empty() {
            results.push(serde_json::json!({
                "title": decode_html_entities(&title),
                "url": url,
                "description": decode_html_entities(&snippet),
            }));
        }
    }

    results
}

/// Find the `href="..."` attribute value in the tag surrounding `class_pos`.
fn extract_href_before(html: &str, class_pos: usize) -> Option<String> {
    let tag_start = html[..class_pos].rfind('<')?;
    let tag_region = &html[tag_start..];
    let href_start = tag_region.find("href=\"")?;
    let value_start = href_start + 6;
    let value_end = tag_region[value_start..].find('"')?;
    Some(tag_region[value_start..value_start + value_end].to_string())
}

/// Extract text content from the element at `class_pos` (after the closing `>`).
fn extract_tag_text(html: &str, class_pos: usize) -> String {
    let Some(tag_close) = html[class_pos..].find('>') else {
        return String::new();
    };
    let content_start = class_pos + tag_close + 1;
    let remaining = &html[content_start..];
    let end = remaining.find("</").unwrap_or(remaining.len());
    strip_tags(&remaining[..end])
}

/// Strip HTML tags, keeping only text content.
fn strip_tags(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {},
        }
    }
    result.trim().to_string()
}

/// Decode common HTML entities.
fn decode_html_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&#160;", " ")
}

/// Resolve a DuckDuckGo redirect URL (`//duckduckgo.com/l/?uddg=...`) to the
/// actual destination. Returns the href as-is when it's not a redirect.
fn resolve_ddg_redirect(href: &str) -> String {
    let full_url = if href.starts_with("//") {
        format!("https:{href}")
    } else {
        href.to_string()
    };

    if full_url.contains("duckduckgo.com/l/")
        && let Ok(parsed) = url::Url::parse(&full_url)
    {
        for (key, value) in parsed.query_pairs() {
            if key == "uddg" {
                return value.into_owned();
            }
        }
    }

    full_url
}

/// URL-encode helper (subset; reqwest doesn't re-export this).
mod urlencoding {
    pub fn encode(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(b as char);
                },
                _ => {
                    out.push_str(&format!("%{b:02X}"));
                },
            }
        }
        out
    }
}

#[async_trait]
impl AgentTool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web and return results. Use this when you need up-to-date information, \
         current events, or facts you're unsure about. Results are localized to the \
         user's preferred language when the search provider supports it."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "count": {
                    "type": "integer",
                    "description": "Number of results (1-10, default 5)",
                    "minimum": 1,
                    "maximum": 10
                },
                "country": {
                    "type": "string",
                    "description": "Country code for search results (e.g. 'US', 'GB')"
                },
                "search_lang": {
                    "type": "string",
                    "description": "Search language (e.g. 'en')"
                },
                "ui_lang": {
                    "type": "string",
                    "description": "UI language (e.g. 'en-US')"
                },
                "freshness": {
                    "type": "string",
                    "description": "Freshness filter (Brave only): 'pd' (past day), 'pw' (past week), 'pm' (past month), 'py' (past year)"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing 'query' parameter"))?;

        let count = params
            .get("count")
            .and_then(|v| v.as_u64())
            .map(|n| n.clamp(1, 10) as u8)
            .unwrap_or(self.max_results);

        // Check cache.
        let cache_key = format!("{:?}:{query}:{count}", self.provider);
        if let Some(cached) = self.cache_get(&cache_key) {
            debug!("web_search cache hit for: {query}");
            return Ok(cached);
        }

        let accept_language = params.get("_accept_language").and_then(|v| v.as_str());

        debug!("web_search: {query} (count={count})");
        let result = match &self.provider {
            SearchProvider::Brave => {
                self.search_brave(query, count, &params, accept_language)
                    .await?
            },
            SearchProvider::Perplexity { base_url, model } => {
                self.search_perplexity(query, base_url, model).await?
            },
        };

        // When the configured provider can't run (no API key), try DuckDuckGo
        // HTML search as a transparent fallback so the LLM never has to ask.
        let result = if self.fallback_enabled
            && result.get("error").is_some()
            && self.api_key.expose_secret().is_empty()
        {
            warn!(
                provider = ?self.provider,
                "search API key not configured, falling back to DuckDuckGo"
            );
            match self.search_duckduckgo(query, count).await {
                Ok(ddg_result) => ddg_result,
                Err(e) => {
                    warn!(%e, "DuckDuckGo fallback failed, returning original error");
                    result
                },
            }
        } else {
            result
        };

        self.cache_set(cache_key, result.clone());
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brave_tool() -> WebSearchTool {
        WebSearchTool::new(
            SearchProvider::Brave,
            Secret::new(String::new()),
            5,
            Duration::from_secs(10),
            Duration::from_secs(60),
            false, // no network fallback in tests
        )
    }

    fn perplexity_tool() -> WebSearchTool {
        WebSearchTool::new(
            SearchProvider::Perplexity {
                base_url: "https://api.perplexity.ai".into(),
                model: "sonar-pro".into(),
            },
            Secret::new(String::new()),
            5,
            Duration::from_secs(10),
            Duration::from_secs(60),
            false, // no network fallback in tests
        )
    }

    #[test]
    fn test_tool_name_and_schema() {
        let tool = brave_tool();
        assert_eq!(tool.name(), "web_search");
        let schema = tool.parameters_schema();
        assert_eq!(schema["required"][0], "query");
    }

    #[tokio::test]
    async fn test_missing_query_param() {
        let tool = brave_tool();
        let result = tool.execute(serde_json::json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("query"));
    }

    #[tokio::test]
    async fn test_brave_missing_api_key_returns_hint() {
        let tool = brave_tool();
        let result = tool
            .execute(serde_json::json!({"query": "test"}))
            .await
            .unwrap();
        assert!(result["error"].as_str().unwrap().contains("not configured"));
        assert!(result["hint"].as_str().unwrap().contains("BRAVE_API_KEY"));
    }

    #[tokio::test]
    async fn test_perplexity_missing_api_key_returns_hint() {
        let tool = perplexity_tool();
        let result = tool
            .execute(serde_json::json!({"query": "test"}))
            .await
            .unwrap();
        assert!(result["error"].as_str().unwrap().contains("not configured"));
        assert!(
            result["hint"]
                .as_str()
                .unwrap()
                .contains("PERPLEXITY_API_KEY")
        );
    }

    #[test]
    fn test_cache_hit_and_miss() {
        let tool = brave_tool();
        let key = "test-key".to_string();
        let val = serde_json::json!({"cached": true});

        assert!(tool.cache_get(&key).is_none());
        tool.cache_set(key.clone(), val.clone());
        assert_eq!(tool.cache_get(&key).unwrap(), val);
    }

    #[test]
    fn test_cache_disabled_when_zero_ttl() {
        let tool = WebSearchTool::new(
            SearchProvider::Brave,
            Secret::new(String::new()),
            5,
            Duration::from_secs(10),
            Duration::ZERO,
            false,
        );
        tool.cache_set("k".into(), serde_json::json!(1));
        assert!(tool.cache_get("k").is_none());
    }

    #[test]
    fn test_urlencoding() {
        assert_eq!(urlencoding::encode("hello world"), "hello%20world");
        assert_eq!(urlencoding::encode("a+b=c"), "a%2Bb%3Dc");
        assert_eq!(urlencoding::encode("safe-_.~"), "safe-_.~");
    }

    #[test]
    fn test_brave_response_parsing() {
        let json = serde_json::json!({
            "web": {
                "results": [
                    {"title": "Rust", "url": "https://rust-lang.org", "description": "A language"},
                    {"title": "Crates", "url": "https://crates.io", "description": "Packages"}
                ]
            }
        });
        let resp: BraveSearchResponse = serde_json::from_value(json).unwrap();
        let results = resp.web.unwrap().results;
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Rust");
    }

    #[test]
    fn test_perplexity_response_parsing() {
        let json = serde_json::json!({
            "choices": [{"message": {"content": "Answer text"}}],
            "citations": ["https://example.com"]
        });
        let resp: PerplexityResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.choices[0].message.content, "Answer text");
        assert_eq!(resp.citations.len(), 1);
    }

    #[test]
    fn test_resolve_perplexity_config_pplx_prefix() {
        let cfg = PerplexityConfig {
            api_key: Some(Secret::new("pplx-abc123".to_string())),
            base_url: None,
            model: None,
        };
        let (key, url) = resolve_perplexity_config(&cfg);
        assert_eq!(key.expose_secret(), "pplx-abc123");
        assert!(url.contains("perplexity.ai"));
    }

    #[test]
    fn test_resolve_perplexity_config_openrouter_prefix() {
        let cfg = PerplexityConfig {
            api_key: Some(Secret::new("sk-or-abc123".to_string())),
            base_url: None,
            model: None,
        };
        let (key, url) = resolve_perplexity_config(&cfg);
        assert_eq!(key.expose_secret(), "sk-or-abc123");
        assert!(url.contains("openrouter.ai"));
    }

    #[test]
    fn test_from_config_disabled() {
        let cfg = WebSearchConfig {
            enabled: false,
            ..Default::default()
        };
        assert!(WebSearchTool::from_config(&cfg).is_none());
    }

    #[test]
    fn test_count_clamping() {
        // count parameter should be clamped to 1-10
        let params = serde_json::json!({"query": "test", "count": 50});
        let count = params
            .get("count")
            .and_then(|v| v.as_u64())
            .map(|n| n.clamp(1, 10) as u8)
            .unwrap_or(5);
        assert_eq!(count, 10);
    }

    // --- DuckDuckGo fallback tests ---

    #[test]
    fn test_parse_duckduckgo_html_basic() {
        let html = r#"
        <div class="web-result">
          <h2><a href="//duckduckgo.com/l/?uddg=https%3A%2F%2Frust-lang.org&amp;rut=abc" class="result__a">Rust Programming Language</a></h2>
          <a class="result__snippet">A language empowering everyone to build reliable software.</a>
        </div>
        <div class="web-result">
          <h2><a href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fcrates.io&amp;rut=def" class="result__a">crates.io: Rust Package Registry</a></h2>
          <a class="result__snippet">The Rust community's package registry.</a>
        </div>
        "#;
        let results = parse_duckduckgo_html(html, 5);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["title"], "Rust Programming Language");
        assert_eq!(results[0]["url"], "https://rust-lang.org");
        assert_eq!(
            results[0]["description"],
            "A language empowering everyone to build reliable software."
        );
        assert_eq!(results[1]["title"], "crates.io: Rust Package Registry");
        assert_eq!(results[1]["url"], "https://crates.io");
    }

    #[test]
    fn test_parse_duckduckgo_html_respects_max() {
        let html = r#"
        <div class="web-result">
          <h2><a href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fa.com&amp;rut=1" class="result__a">A</a></h2>
          <a class="result__snippet">Desc A</a>
        </div>
        <div class="web-result">
          <h2><a href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fb.com&amp;rut=2" class="result__a">B</a></h2>
          <a class="result__snippet">Desc B</a>
        </div>
        <div class="web-result">
          <h2><a href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fc.com&amp;rut=3" class="result__a">C</a></h2>
          <a class="result__snippet">Desc C</a>
        </div>
        "#;
        let results = parse_duckduckgo_html(html, 2);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_parse_duckduckgo_html_empty() {
        let results = parse_duckduckgo_html("<html><body>No results</body></html>", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_parse_duckduckgo_html_with_entities() {
        let html = r#"
        <div class="web-result">
          <h2><a href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com&amp;rut=x" class="result__a">Tom &amp; Jerry</a></h2>
          <a class="result__snippet">A &lt;classic&gt; show</a>
        </div>
        "#;
        let results = parse_duckduckgo_html(html, 5);
        assert_eq!(results[0]["title"], "Tom & Jerry");
        assert_eq!(results[0]["description"], "A <classic> show");
    }

    #[test]
    fn test_resolve_ddg_redirect_standard() {
        let href = "//duckduckgo.com/l/?uddg=https%3A%2F%2Frust-lang.org%2Flearn&rut=abc";
        assert_eq!(resolve_ddg_redirect(href), "https://rust-lang.org/learn");
    }

    #[test]
    fn test_resolve_ddg_redirect_not_a_redirect() {
        assert_eq!(
            resolve_ddg_redirect("https://example.com/page"),
            "https://example.com/page"
        );
    }

    #[test]
    fn test_resolve_ddg_redirect_protocol_relative() {
        assert_eq!(
            resolve_ddg_redirect("//example.com/page"),
            "https://example.com/page"
        );
    }

    #[test]
    fn test_strip_tags_basic() {
        assert_eq!(strip_tags("hello <b>world</b>"), "hello world");
        assert_eq!(strip_tags("no tags"), "no tags");
        assert_eq!(strip_tags("<a href='x'>link</a>"), "link");
    }

    #[test]
    fn test_decode_html_entities_basic() {
        assert_eq!(decode_html_entities("a &amp; b"), "a & b");
        assert_eq!(decode_html_entities("&lt;div&gt;"), "<div>");
        assert_eq!(decode_html_entities("it&#39;s"), "it's");
    }

    #[test]
    fn test_extract_href_before() {
        let html = r#"<a href="https://example.com" class="result__a">Title</a>"#;
        let class_pos = html.find("class=\"result__a\"").unwrap();
        assert_eq!(
            extract_href_before(html, class_pos),
            Some("https://example.com".to_string())
        );
    }
}
